// Copyright 2021 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The XML documents that the SVG parser reads.
//!
//! Upstream `usvg` reads a `roxmltree::Document`. This fork reads a [`Document`]: a tree that
//! owns its strings, which has the part of the API of `roxmltree` that the parser uses. Two
//! things make it more than a copy:
//!
//! - A [`Document`] can be built from any tree with a [`DocumentBuilder`], for example the DOM
//!   of an HTML document with `<svg>` elements in it.
//! - The CSS declarations that apply to an element are stored with the element
//!   ([`Node::css_declarations`]). The code that builds the document resolves them, with the
//!   selector matching of its own, so the parser does not need a CSS engine. For SVG files,
//!   [`Document::from_roxmltree`] resolves them with `simplecss`.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// An opaque number that the code which builds a document can attach to an element, to find
/// the element again (for the children of a `foreignObject`, for example).
pub type Handle = u64;

/// A string that is either borrowed or shared. The same as in `roxmltree`.
#[derive(Clone, Debug)]
pub enum StringStorage<'input> {
    /// A string slice.
    Borrowed(&'input str),
    /// A shared string.
    Owned(Arc<str>),
}

impl<'input> StringStorage<'input> {
    /// Creates a value from a type that can be converted into a shared string.
    pub fn new_owned<T: Into<Arc<str>>>(s: T) -> Self {
        StringStorage::Owned(s.into())
    }

    /// The string.
    pub fn as_str(&self) -> &str {
        match self {
            StringStorage::Borrowed(s) => s,
            StringStorage::Owned(s) => s,
        }
    }
}

impl std::fmt::Display for StringStorage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::ops::Deref for StringStorage<'_> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

/// A CSS declaration that applies to an element.
#[derive(Clone, Debug, PartialEq)]
pub struct CssDeclaration {
    /// The property, like `fill`.
    pub name: String,
    /// The value, like `#ff0000`.
    pub value: String,
    /// Whether the declaration is `!important`.
    pub important: bool,
}

/// An attribute of an element.
#[derive(Clone, Debug)]
pub struct AttributeData {
    /// The namespace of the attribute, if it has one.
    pub namespace: Option<Arc<str>>,
    /// The local name.
    pub name: Arc<str>,
    /// The value.
    pub value: Arc<str>,
}

#[derive(Debug)]
enum Kind {
    Root,
    Element {
        namespace: Option<Arc<str>>,
        name: Arc<str>,
        attributes: Vec<AttributeData>,
        css: Vec<CssDeclaration>,
        handle: Option<Handle>,
    },
    Text(Arc<str>),
}

#[derive(Debug)]
struct NodeData {
    parent: Option<usize>,
    children: Vec<usize>,
    kind: Kind,
}

/// An XML document.
#[derive(Debug)]
pub struct Document<'input> {
    nodes: Vec<NodeData>,
    marker: PhantomData<&'input ()>,
}

/// An XML node: the root, an element or text.
#[derive(Debug)]
pub struct Node<'a, 'input: 'a> {
    doc: &'a Document<'input>,
    id: usize,
}

impl Clone for Node<'_, '_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for Node<'_, '_> {}

impl PartialEq for Node<'_, '_> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.doc, other.doc)
    }
}

impl Eq for Node<'_, '_> {}

/// The name of an element: its namespace and its local name.
#[derive(Clone, Copy, Debug)]
pub struct ExpandedName<'a> {
    namespace: Option<&'a str>,
    name: &'a str,
}

impl<'a> ExpandedName<'a> {
    /// The namespace.
    pub fn namespace(&self) -> Option<&'a str> {
        self.namespace
    }

    /// The local name.
    pub fn name(&self) -> &'a str {
        self.name
    }
}

impl<'a> From<&'a str> for ExpandedName<'a> {
    fn from(name: &'a str) -> Self {
        ExpandedName {
            namespace: None,
            name,
        }
    }
}

impl<'a> From<(&'a str, &'a str)> for ExpandedName<'a> {
    fn from((namespace, name): (&'a str, &'a str)) -> Self {
        ExpandedName {
            namespace: Some(namespace),
            name,
        }
    }
}

/// An attribute of a node.
#[derive(Clone, Copy, Debug)]
pub struct Attribute<'a, 'input: 'a> {
    data: &'a AttributeData,
    marker: PhantomData<&'input ()>,
}

impl<'a, 'input: 'a> Attribute<'a, 'input> {
    /// The namespace.
    pub fn namespace(&self) -> Option<&'a str> {
        self.data.namespace.as_deref()
    }

    /// The local name.
    pub fn name(&self) -> &'a str {
        &self.data.name
    }

    /// The value.
    pub fn value(&self) -> &'a str {
        &self.data.value
    }

    /// The value as it is stored.
    pub fn value_storage(&self) -> StringStorage<'input> {
        StringStorage::Owned(self.data.value.clone())
    }
}

impl<'a, 'input: 'a> Node<'a, 'input> {
    fn data(&self) -> &'a NodeData {
        &self.doc.nodes[self.id]
    }

    fn node(&self, id: usize) -> Node<'a, 'input> {
        Node { doc: self.doc, id }
    }

    /// The document of the node.
    pub fn document(&self) -> &'a Document<'input> {
        self.doc
    }

    /// Whether the node is an element.
    pub fn is_element(&self) -> bool {
        matches!(self.data().kind, Kind::Element { .. })
    }

    /// Whether the node is text.
    pub fn is_text(&self) -> bool {
        matches!(self.data().kind, Kind::Text(_))
    }

    /// The text of a text node.
    pub fn text(&self) -> Option<&'a str> {
        match &self.data().kind {
            Kind::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The name of an element; empty for the other nodes.
    pub fn tag_name(&self) -> ExpandedName<'a> {
        match &self.data().kind {
            Kind::Element {
                namespace, name, ..
            } => ExpandedName {
                namespace: namespace.as_deref(),
                name,
            },
            _ => ExpandedName {
                namespace: None,
                name: "",
            },
        }
    }

    /// Whether the node is an element with this name.
    pub fn has_tag_name<'n, N: Into<ExpandedName<'n>>>(&self, name: N) -> bool {
        let name = name.into();
        self.is_element()
            && self.tag_name().name() == name.name
            && (name.namespace.is_none() || self.tag_name().namespace() == name.namespace)
    }

    /// The attributes of an element.
    pub fn attributes(&self) -> impl Iterator<Item = Attribute<'a, 'input>> + 'a {
        let attributes: &'a [AttributeData] = match &self.data().kind {
            Kind::Element { attributes, .. } => attributes,
            _ => &[],
        };
        attributes.iter().map(|data| Attribute {
            data,
            marker: PhantomData,
        })
    }

    /// The attribute with a name. Without namespace, the first attribute with the local name
    /// is returned, whatever its namespace is, as in `roxmltree`.
    pub fn attribute_node<'n, N: Into<ExpandedName<'n>>>(
        &self,
        name: N,
    ) -> Option<Attribute<'a, 'input>> {
        let name = name.into();
        self.attributes().find(|attribute| {
            attribute.name() == name.name
                && (name.namespace.is_none() || attribute.namespace() == name.namespace)
        })
    }

    /// The value of the attribute with a name.
    pub fn attribute<'n, N: Into<ExpandedName<'n>>>(&self, name: N) -> Option<&'a str> {
        self.attribute_node(name).map(|attribute| attribute.value())
    }

    /// The CSS declarations that apply to the element, in the order in which they are applied
    /// (later declarations override earlier ones). The `style` attribute is not part of them.
    pub fn css_declarations(&self) -> &'a [CssDeclaration] {
        match &self.data().kind {
            Kind::Element { css, .. } => css,
            _ => &[],
        }
    }

    /// The handle that the builder attached to the element.
    pub fn handle(&self) -> Option<Handle> {
        match &self.data().kind {
            Kind::Element { handle, .. } => *handle,
            _ => None,
        }
    }

    /// The parent.
    pub fn parent(&self) -> Option<Node<'a, 'input>> {
        self.data().parent.map(|id| self.node(id))
    }

    /// The parent, if it is an element.
    pub fn parent_element(&self) -> Option<Node<'a, 'input>> {
        self.parent().filter(|parent| parent.is_element())
    }

    /// The children.
    pub fn children(self) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
        let doc = self.doc;
        self.data()
            .children
            .iter()
            .map(move |&id| Node { doc, id })
    }

    /// The previous sibling that is an element.
    pub fn prev_sibling_element(&self) -> Option<Node<'a, 'input>> {
        let parent = self.parent()?;
        let siblings = &parent.data().children;
        let position = siblings.iter().position(|&id| id == self.id)?;
        siblings[..position]
            .iter()
            .rev()
            .map(|&id| self.node(id))
            .find(|node| node.is_element())
    }

    /// The node and all nodes below it, in document order.
    pub fn descendants(self) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
        let doc = self.doc;
        let mut out = Vec::new();
        let mut stack = vec![self.id];
        while let Some(id) = stack.pop() {
            out.push(Node { doc, id });
            stack.extend(doc.nodes[id].children.iter().rev());
        }
        out.into_iter()
    }
}

impl<'input> Document<'input> {
    /// The root node, the parent of the root element.
    pub fn root<'a>(&'a self) -> Node<'a, 'input> {
        Node { doc: self, id: 0 }
    }

    /// All nodes in document order.
    pub fn descendants<'a>(&'a self) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
        Node { doc: self, id: 0 }.descendants()
    }

    /// Reads an SVG file that `roxmltree` has parsed. The CSS of its `<style>` elements and of
    /// `injected_stylesheet` is resolved with `simplecss`.
    pub fn from_roxmltree(
        xml: &roxmltree::Document<'input>,
        injected_stylesheet: Option<&str>,
    ) -> Document<'input> {
        const SVG_NS: &str = "http://www.w3.org/2000/svg";
        let style_texts: Vec<String> = xml
            .descendants()
            .filter(|n| n.has_tag_name("style"))
            .filter(|n| {
                n.attribute("type")
                    .is_none_or(|t| t.is_empty() || t.eq_ignore_ascii_case("text/css"))
            })
            .map(|n| {
                n.children()
                    .filter(|c| c.is_text())
                    .filter_map(|c| c.text())
                    .collect()
            })
            .collect();

        let mut sheet = simplecss::StyleSheet::new();
        // Injected style sheets do not override internal ones (we mimic the logic of
        // rsvg-convert), so we need to parse it first.
        if let Some(injected_stylesheet) = injected_stylesheet {
            sheet.parse_more(injected_stylesheet);
        }
        for text in &style_texts {
            sheet.parse_more(text);
        }

        let mut builder = DocumentBuilder::new();
        let mut map: HashMap<roxmltree::NodeId, usize> = HashMap::new();
        map.insert(xml.root().id(), 0);
        for node in xml.root().descendants().skip(1) {
            let parent = match node.parent().and_then(|parent| map.get(&parent.id())) {
                Some(&parent) => parent,
                None => continue,
            };
            let id = if node.is_element() {
                let attributes = node
                    .attributes()
                    .map(|attribute| AttributeData {
                        namespace: attribute.namespace().map(Arc::from),
                        name: Arc::from(attribute.name()),
                        value: Arc::from(attribute.value()),
                    })
                    .collect();
                let css = sheet
                    .rules
                    .iter()
                    .filter(|rule| rule.selector.matches(&RoxmlNode(node)))
                    .flat_map(|rule| rule.declarations.iter())
                    .map(|declaration| CssDeclaration {
                        name: declaration.name.to_owned(),
                        value: declaration.value.to_owned(),
                        important: declaration.important,
                    })
                    .collect();
                let tag = node.tag_name();
                // The namespace of an SVG element is checked when the element is read.
                let _ = SVG_NS;
                builder.element(parent, tag.namespace(), tag.name(), attributes, css, None)
            } else if node.is_text() {
                match node.text() {
                    Some(text) => builder.text(parent, text),
                    None => continue,
                }
            } else {
                continue;
            };
            map.insert(node.id(), id);
        }
        builder.build()
    }
}

struct RoxmlNode<'a, 'input: 'a>(roxmltree::Node<'a, 'input>);

impl simplecss::Element for RoxmlNode<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(RoxmlNode)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(RoxmlNode)
    }

    fn has_local_name(&self, local_name: &str) -> bool {
        self.0.tag_name().name() == local_name
    }

    fn attribute_matches(&self, local_name: &str, operator: simplecss::AttributeOperator) -> bool {
        match self.0.attribute(local_name) {
            Some(value) => operator.matches(value),
            None => false,
        }
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass) -> bool {
        match class {
            simplecss::PseudoClass::FirstChild => self.prev_sibling_element().is_none(),
            // TODO: lang
            _ => false, // Since we are querying a static SVG we can ignore other pseudo-classes.
        }
    }
}

/// Builds a [`Document`] node by node.
#[derive(Debug)]
pub struct DocumentBuilder {
    nodes: Vec<NodeData>,
}

impl Default for DocumentBuilder {
    fn default() -> Self {
        DocumentBuilder::new()
    }
}

impl DocumentBuilder {
    /// Starts a document that has only the root. The root has the id 0.
    pub fn new() -> Self {
        DocumentBuilder {
            nodes: vec![NodeData {
                parent: None,
                children: Vec::new(),
                kind: Kind::Root,
            }],
        }
    }

    /// The id of the root node.
    pub fn root(&self) -> usize {
        0
    }

    /// Appends an element to `parent` and returns its id.
    ///
    /// `css` are the declarations of the style sheets that apply to the element, in the order
    /// of the cascade; the `style` attribute is one of the `attributes`. `handle` is returned
    /// by [`Node::handle`].
    pub fn element(
        &mut self,
        parent: usize,
        namespace: Option<&str>,
        name: &str,
        attributes: Vec<AttributeData>,
        css: Vec<CssDeclaration>,
        handle: Option<Handle>,
    ) -> usize {
        self.push(
            parent,
            Kind::Element {
                namespace: namespace.map(Arc::from),
                name: Arc::from(name),
                attributes,
                css,
                handle,
            },
        )
    }

    /// Appends text to `parent`.
    pub fn text(&mut self, parent: usize, text: &str) -> usize {
        self.push(parent, Kind::Text(Arc::from(text)))
    }

    fn push(&mut self, parent: usize, kind: Kind) -> usize {
        let id = self.nodes.len();
        self.nodes.push(NodeData {
            parent: Some(parent),
            children: Vec::new(),
            kind,
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Finishes the document.
    pub fn build<'input>(self) -> Document<'input> {
        Document {
            nodes: self.nodes,
            marker: PhantomData,
        }
    }
}
