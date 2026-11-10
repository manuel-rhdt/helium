//! Tagged PDF: the logical structure of the document (ISO 32000-1, 14.7 and 14.8).
//!
//! The input is HTML, so the structure is already there: the structure tree is the DOM with
//! every element mapped to a standard PDF structure type (`p` → `P`, `ul` → `L`, `th` → `TH`,
//! ...). Elements without a PDF counterpart (`span`, `em`, ...) are transparent: their content
//! belongs to the nearest mapped ancestor.
//!
//! The tree is built once after box construction ([`StructureTree::build`]). It assigns a
//! *content slot* to every piece of the DOM that paints something: text nodes, the markers of
//! list items and `<math>` elements. The code that paints this content wraps it in
//! [`TextOperator::BeginTag`](crate::layout::components::TextOperator::BeginTag) with the slot
//! and the logical offset of the content in it. While the PDF is written, [`TagRecorder`] turns
//! every tag into a marked-content sequence with an MCID and finally writes the structure
//! elements, whose children are sorted by slot and offset: the reading order of the structure
//! tree is the logical order of the DOM, whatever order the content has on the page (bidi
//! reordering, page margin boxes, z-index).

use std::collections::HashMap;

use html5ever::{LocalName, QualName, ns};
use lemon_pdf::annotation::Annotation;
use lemon_pdf::object::{IndirectReference, Name, TextString};
use lemon_pdf::structure_tree::{
    AttributeObject, AttributeValue, MarkedContentReference, NumberTree, ObjectReference,
    ParentTreeEntry, StructElem, StructKid, StructParent, StructTreeRoot, StructType,
};
use lemon_pdf::{DocumentContext, Page};
use specs::Entity;

use crate::parser::{ElementData, ParsedData};
use crate::style::properties::computed::ComputedValues;
use crate::style::properties::specified::ListStyleType;
use crate::tree::{DomRef, NodeData, TreeLinked};

/// The index of a node of the [`StructureTree`].
pub type StructNodeId = u32;

/// Marks the content that follows as (part of) a content slot of the structure tree.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ContentTag {
    /// The structure node that the content belongs to.
    pub node: StructNodeId,
    /// The content slot (a text node, a list marker, a formula) inside the node.
    pub slot: u32,
    /// The logical position of the content inside its slot (a byte offset into the text of a
    /// text node). Marked-content sequences of a slot are ordered by it.
    pub offset: u32,
}

/// A child of a structure node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kid {
    Node(StructNodeId),
    /// The content of a content slot: all marked-content sequences tagged with it.
    Content(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructNode {
    /// The standard structure type, like `P` or `TH`.
    pub role: &'static str,
    pub parent: Option<StructNodeId>,
    /// The children in document order.
    pub kids: Vec<Kid>,
    /// The language, if it differs from the language of the parent.
    pub lang: Option<String>,
    /// The alternate description (`/Alt`).
    pub alt: Option<String>,
    /// The expansion of an abbreviation (`/E`).
    pub expansion: Option<String>,
    /// The structure attributes as `(owner, key, value)`.
    pub attributes: Vec<(&'static str, &'static str, AttributeValue)>,
    /// The text of a `Link`, the description of its annotations.
    pub link_text: Option<String>,
}

impl StructNode {
    fn new(role: &'static str, parent: Option<StructNodeId>) -> Self {
        StructNode {
            role,
            parent,
            kids: vec![],
            lang: None,
            alt: None,
            expansion: None,
            attributes: vec![],
            link_text: None,
        }
    }
}

/// The logical structure of the document. Node 0 is the `Document` element.
#[derive(Debug, Clone, Default)]
pub struct StructureTree {
    pub nodes: Vec<StructNode>,
    /// The content slot of every entity that paints content: `(node, slot)`, by entity id.
    content: HashMap<u32, (StructNodeId, u32)>,
    /// The `Link` node of every entity inside an `<a href>`, by entity id.
    links: HashMap<u32, StructNodeId>,
    /// The language of the document (the `lang` of the root element).
    pub lang: Option<String>,
    /// The title of the document (`<title>`).
    pub title: Option<String>,
    next_slot: u32,
}

/// What an element becomes in the structure tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapping {
    /// The element and its content are not part of the structure (`<head>`, `aria-hidden`):
    /// content that is painted anyway becomes an artifact.
    Skip,
    /// The content of the element belongs to the parent.
    Transparent,
    Role(&'static str),
}

/// Structure types that group block-level elements. Text directly inside them is wrapped in an
/// implicit `P`.
fn is_grouping(role: &str) -> bool {
    matches!(
        role,
        "Document" | "Part" | "Art" | "Sect" | "Div" | "BlockQuote" | "NonStruct"
    )
}

/// The structure types of the parts of a table that are kept even if they are empty, so that
/// the table stays regular.
fn is_table_part(role: &str) -> bool {
    matches!(role, "TR" | "TH" | "TD" | "THead" | "TBody" | "TFoot")
}

/// The structure type of a role.
fn struct_type(role: &'static str) -> StructType {
    match role {
        "Document" => StructType::Document,
        "Part" => StructType::Part,
        "Art" => StructType::Art,
        "Sect" => StructType::Sect,
        "Div" => StructType::Div,
        "BlockQuote" => StructType::BlockQuote,
        "Caption" => StructType::Caption,
        "NonStruct" => StructType::NonStruct,
        "P" => StructType::P,
        "H" => StructType::H,
        "H1" => StructType::H1,
        "H2" => StructType::H2,
        "H3" => StructType::H3,
        "H4" => StructType::H4,
        "H5" => StructType::H5,
        "H6" => StructType::H6,
        "L" => StructType::L,
        "LI" => StructType::LI,
        "Lbl" => StructType::Lbl,
        "LBody" => StructType::LBody,
        "Table" => StructType::Table,
        "TR" => StructType::TR,
        "TH" => StructType::TH,
        "TD" => StructType::TD,
        "THead" => StructType::THead,
        "TBody" => StructType::TBody,
        "TFoot" => StructType::TFoot,
        "Span" => StructType::Span,
        "Quote" => StructType::Quote,
        "Note" => StructType::Note,
        "Code" => StructType::Code,
        "Link" => StructType::Link,
        "Figure" => StructType::Figure,
        "Formula" => StructType::Formula,
        other => StructType::Other(Name::from(other)),
    }
}

fn attribute(element: &ElementData, name: &str) -> Option<String> {
    element
        .attributes
        .borrow()
        .get(&QualName::new(None, ns!(), LocalName::from(name)))
        .map(|attribute| attribute.value.to_string())
}

fn heading(level: usize) -> &'static str {
    ["H1", "H2", "H3", "H4", "H5", "H6"]
        .get(level.wrapping_sub(1))
        .copied()
        .unwrap_or("H")
}

/// Maps an ARIA `role` to a structure type, if the role has an equivalent.
fn map_aria_role(role: &str, element: &ElementData) -> Option<Mapping> {
    Some(match role {
        "presentation" | "none" => Mapping::Transparent,
        "heading" => {
            let level = attribute(element, "aria-level")
                .and_then(|level| level.trim().parse().ok())
                .unwrap_or(2);
            Mapping::Role(heading(level))
        }
        "paragraph" => Mapping::Role("P"),
        "list" => Mapping::Role("L"),
        "listitem" => Mapping::Role("LI"),
        "table" | "grid" => Mapping::Role("Table"),
        "row" => Mapping::Role("TR"),
        "cell" | "gridcell" => Mapping::Role("TD"),
        "columnheader" | "rowheader" => Mapping::Role("TH"),
        "link" => Mapping::Role("Link"),
        "math" => Mapping::Role("Formula"),
        "blockquote" => Mapping::Role("BlockQuote"),
        "note" => Mapping::Role("Note"),
        "code" => Mapping::Role("Code"),
        "caption" => Mapping::Role("Caption"),
        "article" => Mapping::Role("Art"),
        "main" => Mapping::Role("Art"),
        "region" | "navigation" | "complementary" | "banner" | "contentinfo" => {
            Mapping::Role("Sect")
        }
        _ => return None,
    })
}

fn map_element(element: &ElementData, style: &ComputedValues) -> Mapping {
    if attribute(element, "aria-hidden").is_some_and(|hidden| hidden.trim() == "true") {
        return Mapping::Skip;
    }
    if let Some(mapping) = attribute(element, "role")
        .and_then(|role| map_aria_role(role.split_whitespace().next()?, element))
    {
        return mapping;
    }

    let name = &element.name;
    if name.ns == ns!(mathml) {
        return if &*name.local == "math" {
            Mapping::Role("Formula")
        } else {
            Mapping::Transparent
        };
    }
    if name.ns != ns!(html) {
        return Mapping::Transparent;
    }

    let role = match &*name.local {
        "head" | "title" | "script" | "style" | "template" | "meta" | "link" | "noscript"
        | "hr" | "br" | "col" | "colgroup" => return Mapping::Skip,
        // the root of the document is the `Document` element
        "html" | "body" => return Mapping::Transparent,
        "main" | "article" => "Art",
        "section" | "nav" | "aside" | "header" | "footer" => "Sect",
        "div" | "address" | "figure" | "hgroup" | "dl" | "dt" | "dd" | "details" | "summary"
        | "fieldset" => "Div",
        "p" | "pre" => "P",
        "h1" => "H1",
        "h2" => "H2",
        "h3" => "H3",
        "h4" => "H4",
        "h5" => "H5",
        "h6" => "H6",
        "blockquote" => "BlockQuote",
        "ul" | "ol" | "menu" => "L",
        "li" => "LI",
        "table" => "Table",
        "thead" => "THead",
        "tbody" => "TBody",
        "tfoot" => "TFoot",
        "tr" => "TR",
        "th" => "TH",
        "td" => "TD",
        "caption" | "figcaption" | "legend" => "Caption",
        "a" if attribute(element, "href").is_some() => "Link",
        "code" | "kbd" | "samp" => "Code",
        "q" => "Quote",
        "abbr" if attribute(element, "title").is_some() => "Span",
        "img" => "Figure",
        _ => {
            // elements that are only presentational (`span`, `em`, `b`, ...) and unknown
            // inline elements belong to their parent, unknown blocks are divisions
            return if style.box_.display.is_block_level() {
                Mapping::Role("Div")
            } else {
                Mapping::Transparent
            };
        }
    };
    Mapping::Role(role)
}

fn list_numbering(list_style_type: ListStyleType) -> &'static str {
    match list_style_type {
        ListStyleType::Disc => "Disc",
        ListStyleType::Circle => "Circle",
        ListStyleType::Square => "Square",
        ListStyleType::Decimal => "Decimal",
        ListStyleType::LowerAlpha => "LowerAlpha",
        ListStyleType::UpperAlpha => "UpperAlpha",
        ListStyleType::LowerRoman => "LowerRoman",
        ListStyleType::UpperRoman => "UpperRoman",
        ListStyleType::None => "None",
    }
}

fn element_name(node: &DomRef) -> Option<String> {
    match &node.html_data {
        ParsedData::Element(element) => Some(element.name.local.to_string()),
        _ => None,
    }
}

/// The scope of a table header cell: from its `scope` attribute, or `Column` if its row has
/// only header cells and `Row` otherwise.
fn header_scope(node: &DomRef, element: &ElementData) -> &'static str {
    match attribute(element, "scope").as_deref().map(str::trim) {
        Some("row") | Some("rowgroup") => return "Row",
        Some("col") | Some("colgroup") => return "Column",
        _ => {}
    }
    let only_headers = node.parent().is_none_or(|row| {
        TreeLinked::children(&row)
            .filter_map(|cell| element_name(&cell))
            .all(|name| name == "th")
    });
    if only_headers { "Column" } else { "Row" }
}

/// The text content of a DOM subtree.
fn text_content(node: &DomRef, out: &mut String) {
    if let ParsedData::Text(text) = &node.html_data {
        out.push_str(&text.borrow());
    }
    for child in TreeLinked::children(node) {
        text_content(&child, out);
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The alternate description of a `<math>` element: its `alttext`, else a TeX annotation,
/// else its text content.
fn formula_alt(node: &DomRef, element: &ElementData) -> String {
    if let Some(alt) = attribute(element, "alttext").filter(|alt| !alt.trim().is_empty()) {
        return alt;
    }
    fn find_annotation(node: &DomRef) -> Option<String> {
        if let ParsedData::Element(element) = &node.html_data {
            if &*element.name.local == "annotation"
                && attribute(element, "encoding").is_some_and(|encoding| encoding.contains("tex"))
            {
                let mut text = String::new();
                text_content(node, &mut text);
                return Some(text.trim().to_owned());
            }
        }
        TreeLinked::children(node).find_map(|child| find_annotation(&child))
    }
    if let Some(tex) = find_annotation(node) {
        return tex;
    }
    let mut text = String::new();
    text_content(node, &mut text);
    collapse_whitespace(&text)
}

struct Builder {
    tree: StructureTree,
    /// The open implicit paragraph of every grouping node.
    implicit_paragraphs: HashMap<StructNodeId, StructNodeId>,
}

impl Builder {
    fn add_node(&mut self, parent: StructNodeId, role: &'static str, inline: bool) -> StructNodeId {
        let parent = if inline {
            self.inline_parent(parent)
        } else {
            self.implicit_paragraphs.remove(&parent);
            parent
        };
        let id = self.tree.nodes.len() as StructNodeId;
        self.tree.nodes.push(StructNode::new(role, Some(parent)));
        self.tree.nodes[parent as usize].kids.push(Kid::Node(id));
        id
    }

    /// The node that inline content of `parent` goes into: `parent` itself or the implicit
    /// paragraph of a grouping element.
    fn inline_parent(&mut self, parent: StructNodeId) -> StructNodeId {
        if !is_grouping(self.tree.nodes[parent as usize].role) {
            return parent;
        }
        if let Some(&paragraph) = self.implicit_paragraphs.get(&parent) {
            return paragraph;
        }
        let paragraph = self.add_node(parent, "P", false);
        self.implicit_paragraphs.insert(parent, paragraph);
        paragraph
    }

    /// Adds a content slot for the content that `entity` paints.
    fn add_content(&mut self, node: StructNodeId, entity: Entity) {
        let slot = self.tree.next_slot;
        self.tree.next_slot += 1;
        self.tree.nodes[node as usize].kids.push(Kid::Content(slot));
        self.tree.content.insert(entity.id(), (node, slot));
    }

    fn walk(&mut self, dom: &DomRef, parent: StructNodeId, lang: &str, link: Option<StructNodeId>) {
        let entity = dom.layout_entity.get().map(|entity| entity.0);
        match &dom.html_data {
            ParsedData::Text(text) => {
                let Some(entity) = entity else {
                    return;
                };
                let blank = text.borrow().trim().is_empty();
                // whitespace between blocks is not content
                if blank
                    && is_grouping(self.tree.nodes[parent as usize].role)
                    && !self.implicit_paragraphs.contains_key(&parent)
                {
                    return;
                }
                let node = self.inline_parent(parent);
                self.add_content(node, entity);
                if let Some(link) = link {
                    self.tree.links.insert(entity.id(), link);
                }
            }
            ParsedData::Element(element) => {
                // elements without a box (`display: none`) paint nothing
                if entity.is_none() {
                    return;
                }
                let style = NodeData::style(dom).unwrap_or_default();
                let element_lang = style
                    .font
                    .lang
                    .tag()
                    .filter(|tag| is_language_tag(tag))
                    .unwrap_or("")
                    .to_owned();
                let lang_changed = element_lang != lang;
                let inline = !style.box_.display.is_block_level();

                let mut mapping = map_element(element, &style);
                if mapping == Mapping::Transparent && lang_changed {
                    // a change of language needs an element to carry it
                    mapping = Mapping::Role(if inline { "Span" } else { "Div" });
                }
                let role = match mapping {
                    Mapping::Skip => return,
                    Mapping::Transparent => {
                        for child in TreeLinked::children(dom) {
                            self.walk(&child, parent, lang, link);
                        }
                        return;
                    }
                    Mapping::Role(role) => role,
                };

                let node = self.add_node(parent, role, inline);
                if lang_changed && !element_lang.is_empty() {
                    self.tree.nodes[node as usize].lang = Some(element_lang.clone());
                }
                self.describe(node, dom, element, &style);

                match role {
                    "Formula" => {
                        // the formula is painted as a whole
                        if let Some(entity) = entity {
                            self.add_content(node, entity);
                        }
                        return;
                    }
                    "LI" => {
                        let marker = style.font.list_style_type.marker_text(1).is_some();
                        if let (true, Some(entity)) = (marker, entity) {
                            let label = self.add_node(node, "Lbl", false);
                            self.add_content(label, entity);
                        }
                        let body = self.add_node(node, "LBody", false);
                        for child in TreeLinked::children(dom) {
                            self.walk(&child, body, &element_lang, link);
                        }
                        return;
                    }
                    _ => {}
                }
                let link = if role == "Link" { Some(node) } else { link };
                for child in TreeLinked::children(dom) {
                    self.walk(&child, node, &element_lang, link);
                }
            }
            ParsedData::Document(_) | ParsedData::DocumentFragment => {
                for child in TreeLinked::children(dom) {
                    self.walk(&child, parent, lang, link);
                }
            }
            _ => {}
        }
    }

    /// Adds the attributes and descriptions of the element to its node.
    fn describe(
        &mut self,
        node: StructNodeId,
        dom: &DomRef,
        element: &ElementData,
        style: &ComputedValues,
    ) {
        let struct_node = &mut self.tree.nodes[node as usize];
        match struct_node.role {
            "L" => struct_node.attributes.push((
                "List",
                "ListNumbering",
                AttributeValue::Name(Name::from(list_numbering(style.font.list_style_type))),
            )),
            "TH" | "TD" => {
                if struct_node.role == "TH" {
                    struct_node.attributes.push((
                        "Table",
                        "Scope",
                        AttributeValue::Name(Name::from(header_scope(dom, element))),
                    ));
                }
                for (name, key) in [("colspan", "ColSpan"), ("rowspan", "RowSpan")] {
                    let span = attribute(element, name)
                        .and_then(|span| span.trim().parse::<i64>().ok())
                        .filter(|&span| span > 1);
                    if let Some(span) = span {
                        struct_node
                            .attributes
                            .push(("Table", key, AttributeValue::Integer(span)));
                    }
                }
            }
            "Formula" => struct_node.alt = Some(formula_alt(dom, element)),
            "Figure" => struct_node.alt = attribute(element, "alt"),
            "Link" => {
                let mut text = String::new();
                text_content(dom, &mut text);
                let text = collapse_whitespace(&text);
                struct_node.link_text = Some(if text.is_empty() {
                    attribute(element, "href").unwrap_or_default()
                } else {
                    text
                });
            }
            "Span" if &*element.name.local == "abbr" => {
                struct_node.expansion = attribute(element, "title")
            }
            _ => {}
        }
        if let Some(label) = attribute(element, "aria-label").filter(|l| !l.trim().is_empty()) {
            if struct_node.alt.is_none() && matches!(struct_node.role, "Figure" | "Formula") {
                struct_node.alt = Some(label);
            }
        }
    }
}

fn find_title(dom: &DomRef) -> Option<String> {
    if element_name(dom).as_deref() == Some("title") {
        let mut text = String::new();
        text_content(dom, &mut text);
        let text = collapse_whitespace(&text);
        return (!text.is_empty()).then_some(text);
    }
    TreeLinked::children(dom).find_map(|child| find_title(&child))
}

fn find_root_lang(dom: &DomRef) -> Option<String> {
    TreeLinked::children(dom).find_map(|child| {
        let ParsedData::Element(element) = &child.html_data else {
            return None;
        };
        attribute(element, "lang")
            .or_else(|| {
                element
                    .attributes
                    .borrow()
                    .get(&QualName::new(None, ns!(xml), LocalName::from("lang")))
                    .map(|attribute| attribute.value.to_string())
            })
            .map(|lang| lang.trim().to_owned())
            .filter(|lang| is_language_tag(lang))
    })
}

/// Whether `tag` has the syntax of a language identifier (ISO 32000-1 14.9.2, RFC 3066), which
/// PDF/UA requires of every `Lang` (ISO 14289-1 7.2). A `lang` attribute that is not one (like
/// the `$lang$` of an unfilled template) gives no language.
fn is_language_tag(tag: &str) -> bool {
    let mut subtags = tag.split('-');
    let primary = subtags.next().unwrap_or("");
    (1..=8).contains(&primary.len())
        && primary.bytes().all(|b| b.is_ascii_alphabetic())
        && subtags.all(|subtag| {
            (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

impl StructureTree {
    /// Builds the structure tree of the DOM. The layout entities must have been assigned.
    pub fn build(dom: &DomRef) -> Self {
        let mut builder = Builder {
            tree: StructureTree {
                nodes: vec![StructNode::new("Document", None)],
                title: find_title(dom),
                lang: find_root_lang(dom),
                ..Default::default()
            },
            implicit_paragraphs: HashMap::new(),
        };
        // the language that `Lang` entries are relative to is the one of the computed style of
        // the root element (normalised the same way)
        let root_lang = builder
            .tree
            .lang
            .as_deref()
            .map(crate::style::properties::computed::Language::from_tag)
            .and_then(|lang| lang.tag().map(str::to_owned))
            .unwrap_or_default();
        builder.walk(dom, 0, &root_lang, None);
        builder.tree
    }

    /// The tag for the content that `entity` paints, at the logical `offset` in it.
    pub fn content_tag(&self, entity: Entity, offset: usize) -> Option<ContentTag> {
        self.content
            .get(&entity.id())
            .map(|&(node, slot)| ContentTag {
                node,
                slot,
                offset: offset as u32,
            })
    }

    /// The `Link` node of a hyperlink that contains `entity`.
    pub fn link_node(&self, entity: Entity) -> Option<StructNodeId> {
        self.links.get(&entity.id()).copied()
    }

    pub fn role(&self, node: StructNodeId) -> &'static str {
        self.nodes[node as usize].role
    }

    /// The tree as indented text (`role [lang] (n content slots)`), for tests and debugging.
    pub fn outline(&self) -> String {
        fn write(tree: &StructureTree, node: StructNodeId, depth: usize, out: &mut String) {
            let struct_node = &tree.nodes[node as usize];
            out.push_str(&"  ".repeat(depth));
            out.push_str(struct_node.role);
            if let Some(lang) = &struct_node.lang {
                out.push_str(&format!(" lang={}", lang));
            }
            for (_, key, value) in &struct_node.attributes {
                match value {
                    AttributeValue::Name(name) => out.push_str(&format!(" {}={}", key, name)),
                    AttributeValue::Integer(n) => out.push_str(&format!(" {}={}", key, n)),
                    other => out.push_str(&format!(" {}={:?}", key, other)),
                }
            }
            let content = struct_node
                .kids
                .iter()
                .filter(|kid| matches!(kid, Kid::Content(_)))
                .count();
            if content > 0 {
                out.push_str(&format!(" #{}", content));
            }
            out.push('\n');
            for kid in &struct_node.kids {
                if let Kid::Node(kid) = kid {
                    write(tree, *kid, depth + 1, out);
                }
            }
        }
        let mut out = String::new();
        if !self.nodes.is_empty() {
            write(self, 0, 0, &mut out);
        }
        out
    }
}

/// A marked-content sequence: `(offset, page, mcid)`.
type MarkedContent = (u32, usize, i64);

/// Assigns MCIDs to the tagged content while the pages are written, and writes the structure
/// tree when all pages are done.
#[derive(Debug)]
pub struct TagRecorder<'t> {
    tree: &'t StructureTree,
    num_pages: usize,
    /// The structure node of every MCID of every page.
    page_mcids: Vec<Vec<StructNodeId>>,
    /// The marked-content sequences of every content slot.
    slots: HashMap<u32, Vec<MarkedContent>>,
    /// The link annotations of every `Link` node: `(page, annotation, parent tree key)`.
    annotations: HashMap<StructNodeId, Vec<(usize, IndirectReference<Annotation>, i64)>>,
    next_annotation_key: i64,
}

impl<'t> TagRecorder<'t> {
    pub fn new(tree: &'t StructureTree, num_pages: usize) -> Self {
        TagRecorder {
            tree,
            num_pages,
            page_mcids: vec![vec![]; num_pages],
            slots: HashMap::new(),
            annotations: HashMap::new(),
            // the keys `0..num_pages` are the pages
            next_annotation_key: num_pages as i64,
        }
    }

    /// Assigns an MCID to tagged content on `page`. Returns the structure type to mark the
    /// content with and the MCID.
    pub fn mark(&mut self, page: usize, tag: ContentTag) -> (&'static str, i64) {
        let mcids = &mut self.page_mcids[page];
        let mcid = mcids.len() as i64;
        mcids.push(tag.node);
        self.slots
            .entry(tag.slot)
            .or_default()
            .push((tag.offset, page, mcid));
        (self.tree.role(tag.node), mcid)
    }

    /// The key in the parent tree for an annotation of the `Link` node `node`. The annotation
    /// is registered with [`TagRecorder::add_annotation`] once it is written.
    pub fn annotation_key(&mut self) -> i64 {
        let key = self.next_annotation_key;
        self.next_annotation_key += 1;
        key
    }

    pub fn add_annotation(
        &mut self,
        node: StructNodeId,
        page: usize,
        annotation: IndirectReference<Annotation>,
        key: i64,
    ) {
        self.annotations
            .entry(node)
            .or_default()
            .push((page, annotation, key));
    }

    /// Writes the structure tree and registers it in the document catalog.
    pub fn finish(
        self,
        context: &mut DocumentContext,
        pages: &[IndirectReference<Page>],
    ) -> std::io::Result<()> {
        let tree = self.tree;
        let count = tree.nodes.len();
        if count == 0 {
            return Ok(());
        }

        // a node is kept if it has content; children have larger ids than their parents
        let mut has_content = vec![false; count];
        for id in (0..count).rev() {
            let node = &tree.nodes[id];
            has_content[id] = self.annotations.contains_key(&(id as StructNodeId))
                || node.kids.iter().any(|kid| match *kid {
                    Kid::Node(kid) => has_content[kid as usize],
                    Kid::Content(slot) => self.slots.contains_key(&slot),
                });
        }
        let mut keep = has_content.clone();
        keep[0] = true;
        for id in 0..count {
            if keep[id] || !is_table_part(tree.nodes[id].role) {
                continue;
            }
            // empty cells and rows of a table that has content keep the table regular
            let mut ancestor = tree.nodes[id].parent;
            while let Some(parent) = ancestor {
                let parent = parent as usize;
                if tree.nodes[parent].role == "Table" {
                    keep[id] = has_content[parent];
                    break;
                }
                ancestor = tree.nodes[parent].parent;
            }
        }

        let root: IndirectReference<StructTreeRoot> = context.create_reference();
        let references: Vec<Option<IndirectReference<StructElem>>> = keep
            .iter()
            .map(|&keep| keep.then(|| context.create_reference()))
            .collect();

        for (id, node) in tree.nodes.iter().enumerate() {
            let Some(reference) = references[id] else {
                continue;
            };
            let parent = match node.parent {
                Some(parent) => StructParent::Elem(
                    references[parent as usize].expect("the parent of a kept node is kept"),
                ),
                None => StructParent::Root(root),
            };
            let mut elem = StructElem::new(struct_type(node.role), parent);

            // the page of the first marked content is the page of the element
            let mut kids: Vec<Kid> = node.kids.clone();
            kids.retain(|kid| match *kid {
                Kid::Node(kid) => keep[kid as usize],
                Kid::Content(slot) => self.slots.contains_key(&slot),
            });
            for kid in kids {
                match kid {
                    Kid::Node(kid) => {
                        elem.k.push(StructKid::Elem(references[kid as usize].unwrap()))
                    }
                    Kid::Content(slot) => {
                        let mut sequences = self.slots[&slot].clone();
                        sequences.sort_by_key(|&(offset, page, mcid)| (offset, page, mcid));
                        for (_, page, mcid) in sequences {
                            let page_reference = pages[page];
                            if elem.pg.is_none() {
                                elem.pg = Some(page_reference);
                            }
                            elem.k.push(if elem.pg == Some(page_reference) {
                                StructKid::Mcid(mcid)
                            } else {
                                StructKid::MarkedContent(MarkedContentReference {
                                    pg: Some(page_reference),
                                    mcid,
                                })
                            });
                        }
                    }
                }
            }
            if let Some(annotations) = self.annotations.get(&(id as StructNodeId)) {
                for &(page, annotation, _) in annotations {
                    elem.k.push(StructKid::Object(ObjectReference {
                        pg: Some(pages[page]),
                        obj: annotation,
                    }));
                }
            }

            let mut owners: Vec<AttributeObject> = vec![];
            for (owner, key, value) in &node.attributes {
                let object = match owners.iter().position(|object| object.owner.as_str() == *owner) {
                    Some(index) => &mut owners[index],
                    None => {
                        owners.push(AttributeObject {
                            owner: Name::from(*owner),
                            attributes: Default::default(),
                        });
                        owners.last_mut().unwrap()
                    }
                };
                object.attributes.insert(key.to_string(), value.clone());
            }
            elem.a = owners;
            elem.lang = node.lang.clone().map(TextString);
            elem.alt = node.alt.clone().map(TextString);
            elem.e = node.expansion.clone().map(TextString);
            context.write_reserved_object(reference, elem)?;
        }

        // the parent tree: the pages, then the annotations
        let mut parent_tree = NumberTree::default();
        for (page, mcids) in self.page_mcids.iter().enumerate() {
            let elements = mcids
                .iter()
                .map(|&node| references[node as usize])
                .collect();
            parent_tree
                .nums
                .insert(page as i64, ParentTreeEntry::Page(elements));
        }
        let mut annotation_entries: Vec<(i64, StructNodeId)> = self
            .annotations
            .iter()
            .flat_map(|(&node, annotations)| annotations.iter().map(move |a| (a.2, node)))
            .collect();
        annotation_entries.sort();
        for (key, node) in annotation_entries {
            if let Some(reference) = references[node as usize] {
                parent_tree
                    .nums
                    .insert(key, ParentTreeEntry::Object(reference));
            }
        }
        debug_assert_eq!(self.page_mcids.len(), self.num_pages);

        context.write_reserved_object(
            root,
            StructTreeRoot {
                k: vec![references[0].unwrap()],
                parent_tree,
                parent_tree_next_key: self.next_annotation_key,
                role_map: Default::default(),
            },
        )?;
        context.set_struct_tree_root(root);
        // without `/MarkInfo << /Marked true >>` the document is not a tagged PDF (ISO 32000-1
        // 14.8), and readers that check the flag ignore the structure tree
        context.set_marked(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{self, LayoutEngine};
    use crate::style::device::Device;
    use crate::style::properties::computed::PixelLength;

    fn structure(html: &str) -> StructureTree {
        let dom = crate::parser::generate_render_tree(html);
        let device = Device {
            width: PixelLength(793.7),
            height: PixelLength(1123.0),
        };
        let world = layout::layout(
            dom,
            device,
            false,
            LayoutEngine::Taffy,
            std::path::Path::new("."),
        )
        .unwrap();
        (*specs::WorldExt::read_resource::<StructureTree>(&world)).clone()
    }

    #[test]
    fn headings_and_paragraphs() {
        let tree = structure("<h1>Title</h1><p>Some <em>emphasised</em> text.</p>");
        assert_eq!(tree.outline(), "Document\n  H1 #1\n  P #3\n");
    }

    #[test]
    fn text_in_divisions_gets_an_implicit_paragraph() {
        let tree = structure("<div>loose <b>text</b><p>para</p>more</div>");
        assert_eq!(tree.outline(), "Document\n  Div\n    P #2\n    P #1\n    P #1\n");
    }

    #[test]
    fn lists_have_labels_and_bodies() {
        let tree = structure("<ol><li>one</li><li>two</li></ol>");
        assert_eq!(
            tree.outline(),
            "Document\n  L ListNumbering=Decimal\n    LI\n      Lbl #1\n      LBody #1\n    LI\n      Lbl #1\n      LBody #1\n"
        );
    }

    #[test]
    fn table_headers_get_a_scope() {
        let tree = structure(
            "<table><tr><th>a</th><th>b</th></tr><tr><th>row</th><td colspan=2>c</td></tr></table>",
        );
        assert_eq!(
            tree.outline(),
            "Document\n  Table\n    TBody\n      TR\n        TH Scope=Column #1\n        TH Scope=Column #1\n      TR\n        TH Scope=Row #1\n        TD ColSpan=2 #1\n"
        );
    }

    #[test]
    fn language_changes_are_recorded() {
        let tree = structure(
            "<html lang=en><p>English <span lang=de>Deutsch</span></p><p lang=fr>Français</p></html>",
        );
        assert_eq!(tree.lang.as_deref(), Some("en"));
        assert_eq!(
            tree.outline(),
            "Document\n  P #1\n    Span lang=de #1\n  P lang=fr #1\n"
        );
    }

    #[test]
    fn links_and_hidden_content() {
        let tree = structure(
            "<p>See <a href='https://example.com'>the <em>site</em></a><span aria-hidden=true>*</span></p>",
        );
        assert_eq!(tree.outline(), "Document\n  P #1\n    Link #2\n");
        let link = tree.nodes.iter().find(|node| node.role == "Link").unwrap();
        assert_eq!(link.link_text.as_deref(), Some("the site"));
    }

    #[test]
    fn title_and_skipped_elements() {
        let tree = structure(
            "<head><title> My  Doc </title><style>p {}</style></head><p>x</p><p style='display:none'>y</p>",
        );
        assert_eq!(tree.title.as_deref(), Some("My Doc"));
        assert_eq!(tree.outline(), "Document\n  P #1\n");
    }

    #[test]
    fn language_tags_are_checked() {
        for tag in ["en", "de-CH", "zh-Hant", "sr-Latn-RS", "x-klingon"] {
            assert!(is_language_tag(tag), "{tag}");
        }
        for tag in ["", "$lang$", "en_US", "en-", "-en", "toolongprimary"] {
            assert!(!is_language_tag(tag), "{tag}");
        }
    }
}
