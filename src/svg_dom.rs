//! The `<svg>` elements of an HTML document, as the XML tree that the SVG parser reads.
//!
//! The SVG parser (`svg/`, a fork of `usvg`) does not read XML text: it reads a
//! `usvg::xml::Document`, which is built here from the DOM that the HTML parser made. So an
//! inline `<svg>` is part of the document: the style sheets of the document apply to its
//! elements, with the selector matching and the cascade of the rest of Umbruch, and the
//! elements of one `<svg>` can refer to the ones of another (`<use href="#icon">`, a gradient
//! in a hidden `<svg>` of the page).

use std::collections::HashSet;
use std::sync::Arc;

use html5ever::{QualName, local_name, ns};
use usvg::xml::{AttributeData, CssDeclaration, DocumentBuilder};

use crate::layout::construct::IslandSpec;
use crate::parser::ParsedData;
use crate::style::parse::Styles;
use crate::tree::{DomRef, TreeLinked};

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Whether the node is an `<svg>` element in the SVG namespace.
pub fn is_svg_element(node: &DomRef) -> bool {
    matches!(&node.html_data, ParsedData::Element(element)
        if element.name == QualName::new(None, ns!(svg), local_name!("svg")))
}

/// Whether the node is an `<svg>` element that is not inside of another `<svg>`, an element
/// that is a replaced element of the page.
pub fn is_outermost_svg(node: &DomRef) -> bool {
    if !is_svg_element(node) {
        return false;
    }
    let mut parent = node.parent();
    while let Some(ancestor) = parent {
        if is_svg_element(&ancestor) {
            return false;
        }
        // HTML inside of a `<foreignObject>` starts a context of its own
        if matches!(&ancestor.html_data, ParsedData::Element(element) if element.name.ns != ns!(svg)) {
            return true;
        }
        parent = ancestor.parent();
    }
    true
}

/// The outermost `<svg>` elements of the document that `node` belongs to.
pub fn svg_roots(node: &DomRef) -> Vec<DomRef> {
    let mut root = node.clone();
    while let Some(parent) = root.parent() {
        root = parent;
    }
    let mut found = vec![];
    collect(&root, &mut found);
    found
}

fn collect(node: &DomRef, found: &mut Vec<DomRef>) {
    if is_svg_element(node) {
        found.push(node.clone());
        return;
    }
    for child in TreeLinked::children(node) {
        collect(&child, found);
    }
}

/// What the HTML context contributes to an `<svg>` element.
#[derive(Debug, Clone, Default)]
pub struct Inherited {
    /// The `color` of the element, as a CSS value: `currentColor` in the SVG.
    pub color: Option<String>,
    /// The `font-family` of the element, as a CSS value.
    pub font_family: Option<String>,
    /// The `font-size` of the element, as a CSS value.
    pub font_size: Option<String>,
}

/// The XML tree of the `<svg>` element `svg`.
///
/// `others` are the other `<svg>` elements of the document. If the element refers to ids that
/// it does not define itself, their contents are added to a `<defs>` at its end, where the
/// references find them.
pub fn document(
    svg: &DomRef,
    others: &[DomRef],
    styles: &Styles,
    inherited: &Inherited,
    islands: &mut Vec<IslandSpec>,
) -> usvg::xml::Document<'static> {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let id = add(&mut builder, root, svg, styles, Some(inherited), islands);
    if let Some(id) = id {
        let (defined, referenced) = ids(svg, styles);
        if referenced.iter().any(|id| !defined.contains(id)) {
            let defs = builder.element(id, Some(SVG_NS), "defs", vec![], vec![], None);
            for other in others.iter().filter(|other| *other != svg) {
                for child in TreeLinked::children(other) {
                    add(&mut builder, defs, &child, styles, None, islands);
                }
            }
        }
    }
    builder.build()
}

/// Adds the node and what is below it. The content of a `<foreignObject>` is not SVG: it is
/// left out, and the element gets a handle (the index of its entry in `islands`) with which the
/// content is found again.
fn add(
    builder: &mut DocumentBuilder,
    parent: usize,
    node: &DomRef,
    styles: &Styles,
    inherited: Option<&Inherited>,
    islands: &mut Vec<IslandSpec>,
) -> Option<usize> {
    match &node.html_data {
        ParsedData::Text(text) => {
            builder.text(parent, &text.borrow());
            None
        }
        ParsedData::Element(element) => {
            if element.name.ns != ns!(svg) {
                return None;
            }
            let mut attributes: Vec<AttributeData> = element
                .attributes
                .borrow()
                .iter()
                .map(|(name, attribute)| AttributeData {
                    namespace: (name.ns != ns!()).then(|| Arc::from(&*name.ns)),
                    name: Arc::from(&*name.local),
                    value: Arc::from(&*attribute.value),
                })
                .collect();
            if let Some(inherited) = inherited {
                // what the SVG elements inherit from the page; their own attributes win
                for (name, value) in [
                    ("color", &inherited.color),
                    ("font-family", &inherited.font_family),
                    ("font-size", &inherited.font_size),
                ] {
                    if let Some(value) = value {
                        if !attributes.iter().any(|a| a.namespace.is_none() && &*a.name == name) {
                            attributes.push(AttributeData {
                                namespace: None,
                                name: Arc::from(name),
                                value: Arc::from(value.as_str()),
                            });
                        }
                    }
                }
            }
            if inherited.is_some() {
                // An `<svg>` element in a page without a `viewBox` has no ratio: a side without
                // a length is 300 by 150 pixels (CSS Sizing 3, 5.1), not the 100 by 100 pixels
                // that usvg uses for an image file.
                let has = |attributes: &[AttributeData], name: &str| {
                    attributes.iter().any(|a| a.namespace.is_none() && &*a.name == name)
                };
                if !has(&attributes, "viewBox") {
                    for (name, value) in [("width", "300"), ("height", "150")] {
                        if !has(&attributes, name) {
                            attributes.push(AttributeData {
                                namespace: None,
                                name: Arc::from(name),
                                value: Arc::from(value),
                            });
                        }
                    }
                }
            }
            let css = declarations(node, styles);
            let foreign = &*element.name.local == "foreignObject";
            let handle = foreign.then(|| {
                islands.push(IslandSpec { node: node.clone(), width: 0.0, height: 0.0 });
                (islands.len() - 1) as u64
            });
            let id = builder.element(parent, Some(SVG_NS), &element.name.local, attributes, css, handle);
            if !foreign {
                for child in TreeLinked::children(node) {
                    add(builder, id, &child, styles, None, islands);
                }
            }
            Some(id)
        }
        _ => None,
    }
}

/// The declarations of the rules of the document that match the element, in the order of the
/// cascade: rules of a higher specificity come later, rules of the same specificity are in the
/// order of the style sheet.
fn declarations(node: &DomRef, styles: &Styles) -> Vec<CssDeclaration> {
    let Some(element) = node.clone().into_element_ref() else {
        return vec![];
    };
    let mut matched: Vec<_> = styles
        .author_rules_iter()
        .filter_map(|rule| rule.matching_specificity(&element).map(|specificity| (specificity, rule)))
        .collect();
    matched.sort_by_key(|&(specificity, _)| specificity);
    matched
        .into_iter()
        .flat_map(|(_, rule)| rule.properties.raw_declarations())
        .map(|declaration| CssDeclaration {
            name: declaration.name.clone(),
            value: declaration.value.clone(),
            important: declaration.important,
        })
        .collect()
}

/// The ids that the element and its descendants define, and the ids that they refer to with
/// `href="#id"` and `url(#id)`.
fn ids(svg: &DomRef, styles: &Styles) -> (HashSet<String>, HashSet<String>) {
    let mut defined = HashSet::new();
    let mut referenced = HashSet::new();
    fn walk(
        node: &DomRef,
        styles: &Styles,
        defined: &mut HashSet<String>,
        referenced: &mut HashSet<String>,
    ) {
        if let ParsedData::Element(element) = &node.html_data {
            for (name, attribute) in element.attributes.borrow().iter() {
                if &*name.local == "id" && name.ns == ns!() {
                    defined.insert(attribute.value.to_string());
                } else if &*name.local == "href" {
                    if let Some(id) = attribute.value.trim().strip_prefix('#') {
                        referenced.insert(id.to_owned());
                    }
                } else {
                    references_in(&attribute.value, referenced);
                }
            }
            for declaration in declarations(node, styles) {
                references_in(&declaration.value, referenced);
            }
        }
        for child in TreeLinked::children(node) {
            walk(&child, styles, defined, referenced);
        }
    }
    walk(svg, styles, &mut defined, &mut referenced);
    (defined, referenced)
}

/// Adds the ids of the `url(#id)` references in `text`.
fn references_in(text: &str, referenced: &mut HashSet<String>) {
    let mut rest = text;
    while let Some(start) = rest.find("url(") {
        rest = &rest[start + 4..];
        let end = rest.find(')').unwrap_or(rest.len());
        let reference = rest[..end].trim().trim_matches(['"', '\'']);
        if let Some(id) = reference.strip_prefix('#') {
            referenced.insert(id.to_owned());
        }
        rest = &rest[end..];
    }
}
