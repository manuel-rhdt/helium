use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, ExpandedName, QualName};

use crate::tree::DomRef;
use crate::tree::{NodeData, NodeRef, TreeLinked};

use std::borrow::Cow;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub type Attributes = BTreeMap<QualName, Attribute>;

/// Node data specific to the node type.
#[derive(Debug, PartialEq, Clone)]
pub enum ParsedData {
    /// Element node
    Element(ElementData),

    /// Text node
    Text(RefCell<String>),

    /// Comment node
    Comment(String),

    /// Processing instruction node
    ProcessingInstruction(String, String),

    /// Doctype node
    Doctype(Doctype),

    /// Document node
    Document(DocumentData),

    /// Document fragment node
    DocumentFragment,
}

/// Data specific to doctype nodes.
#[derive(Debug, PartialEq, Clone)]
pub struct Doctype {
    /// The name of the doctype
    pub name: String,

    /// The public ID of the doctype
    pub public_id: String,

    /// The system ID of the doctype
    pub system_id: String,
}

/// Data specific to element nodes.
#[derive(Debug, PartialEq, Clone)]
pub struct ElementData {
    /// The namespace and local name of the element, such as `ns!(html)` and `body`.
    pub name: QualName,

    /// The attributes of the elements.
    pub attributes: RefCell<Attributes>,

    /// If the element is an HTML `<template>` element,
    /// the document fragment node that is the root of template contents.
    pub template_contents: Option<NodeRef<NodeData>>,
}

/// Data specific to document nodes.
#[derive(Debug, PartialEq, Clone)]
pub struct DocumentData {
    pub(crate) _quirks_mode: Cell<QuirksMode>,
}

impl DocumentData {
    /// The quirks mode of the document, as determined by the HTML parser.
    #[inline]
    pub fn quirks_mode(&self) -> QuirksMode {
        self._quirks_mode.get()
    }
}

trait DomAccess: Sized {
    fn as_text(&self) -> Option<&RefCell<String>>;
    fn as_document(&self) -> Option<&DocumentData>;
    fn as_element(&self) -> Option<&ElementData>;
    fn new_doctype(
        name: impl Into<String>,
        public_id: impl Into<String>,
        system_id: impl Into<String>,
    ) -> Self;

    fn new_text(text: impl Into<String>) -> Self;
    fn new_element<I>(name: QualName, attributes: I) -> Self
    where
        I: IntoIterator<Item = (QualName, Attribute)>;

    fn new_comment(text: impl Into<String>) -> Self;
}

impl DomAccess for NodeRef<NodeData> {
    fn as_text(&self) -> Option<&RefCell<String>> {
        match &self.html_data {
            ParsedData::Text(text) => Some(text),
            _ => None,
        }
    }

    fn as_document(&self) -> Option<&DocumentData> {
        match &self.html_data {
            ParsedData::Document(doc) => Some(doc),
            _ => None,
        }
    }

    fn as_element(&self) -> Option<&ElementData> {
        match &self.html_data {
            ParsedData::Element(elem) => Some(elem),
            _ => None,
        }
    }

    fn new_text(text: impl Into<String>) -> Self {
        NodeRef::new(NodeData::from_html_data(ParsedData::Text(RefCell::new(
            text.into(),
        ))))
    }

    fn new_element<I>(name: QualName, attributes: I) -> Self
    where
        I: IntoIterator<Item = (QualName, Attribute)>,
    {
        NodeRef::new(NodeData::from_html_data(ParsedData::Element(ElementData {
            template_contents: None,
            name,
            attributes: RefCell::new(attributes.into_iter().collect()),
        })))
    }

    fn new_comment(text: impl Into<String>) -> Self {
        NodeRef::new(NodeData::from_html_data(ParsedData::Comment(text.into())))
    }

    fn new_doctype(
        name: impl Into<String>,
        public_id: impl Into<String>,
        system_id: impl Into<String>,
    ) -> Self {
        NodeRef::new(NodeData::from_html_data(ParsedData::Doctype(Doctype {
            name: name.into(),
            public_id: public_id.into(),
            system_id: system_id.into(),
        })))
    }
}

pub struct Sink {
    document_node: NodeRef<NodeData>,
    on_parse_error: Option<Box<FnMut(Cow<'static, str>)>>,
}

impl TreeSink for Sink {
    type Output = NodeRef<NodeData>;

    fn finish(self) -> Self::Output {
        self.document_node
    }

    type Handle = NodeRef<NodeData>;

    #[inline]
    fn parse_error(&mut self, message: Cow<'static, str>) {
        if let Some(ref mut handler) = self.on_parse_error {
            handler(message)
        }
    }

    #[inline]
    fn get_document(&mut self) -> Self::Handle {
        self.document_node.clone()
    }

    #[inline]
    fn set_quirks_mode(&mut self, mode: QuirksMode) {
        self.document_node
            .as_document()
            .unwrap()
            ._quirks_mode
            .set(mode)
    }

    #[inline]
    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    #[inline]
    fn elem_name<'a>(&self, target: &'a Self::Handle) -> ExpandedName<'a> {
        target.as_element().unwrap().name.expanded()
    }

    #[inline]
    fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        NodeRef::new_element(
            name,
            attrs.into_iter().map(|attr| {
                let Attribute { name, .. } = &attr;
                (name.clone(), attr)
            }),
        )
    }

    #[inline]
    fn create_comment(&mut self, text: StrTendril) -> Self::Handle {
        NodeRef::new_comment(text)
    }

    #[inline]
    fn create_pi(&mut self, target: StrTendril, data: StrTendril) -> Self::Handle {
        NodeRef::new(NodeData::from_html_data(ParsedData::ProcessingInstruction(
            target.into(),
            data.into(),
        )))
    }

    #[inline]
    fn append(&mut self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => parent.append(node),
            NodeOrText::AppendText(text) => {
                if let Some(last_child) = parent.last_child() {
                    if let Some(existing) = last_child.as_text() {
                        existing.borrow_mut().push_str(&text);
                        return;
                    }
                }
                parent.append(NodeRef::new_text(text))
            }
        }
    }

    #[inline]
    fn append_before_sibling(&mut self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => sibling.insert_before(node),
            NodeOrText::AppendText(text) => {
                if let Some(previous_sibling) = sibling.previous_sibling() {
                    if let Some(existing) = previous_sibling.as_text() {
                        existing.borrow_mut().push_str(&text);
                        return;
                    }
                }
                sibling.insert_before(NodeRef::new_text(text))
            }
        }
    }

    #[inline]
    fn append_doctype_to_document(
        &mut self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.document_node
            .append(NodeRef::new_doctype(name, public_id, system_id))
    }

    #[inline]
    fn add_attrs_if_missing(&mut self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let element = target.as_element().unwrap();
        let attributes = &element.attributes;

        for Attribute { name, value } in attrs {
            attributes
                .borrow_mut()
                .entry(name.clone())
                .or_insert_with(|| Attribute { name, value });
        }
    }

    #[inline]
    fn remove_from_parent(&mut self, target: &Self::Handle) {
        target.detach()
    }

    #[inline]
    fn reparent_children(&mut self, node: &Self::Handle, new_parent: &Self::Handle) {
        // FIXME: Can this be done more effciently in rctree,
        // by moving the whole linked list of children at once?
        for child in node.children() {
            new_parent.append(child)
        }
    }

    #[inline]
    fn mark_script_already_started(&mut self, _node: &Self::Handle) {
        // FIXME: Is this useful outside of a browser?
    }

    #[inline]
    fn get_template_contents(&mut self, target: &Self::Handle) -> Self::Handle {
        target
            .as_element()
            .unwrap()
            .template_contents
            .clone()
            .unwrap()
    }

    fn append_based_on_parent_node(
        &mut self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if element.parent().is_some() {
            self.append_before_sibling(element, child)
        } else {
            self.append(prev_element, child)
        }
    }
}

fn parse_html() -> html5ever::Parser<Sink> {
    let sink = Sink {
        document_node: NodeRef::new(NodeData::from_html_data(ParsedData::Document(
            DocumentData {
                _quirks_mode: QuirksMode::NoQuirks.into(),
            },
        ))),
        on_parse_error: None,
    };
    html5ever::parse_document(sink, html5ever::ParseOpts::default())
}

pub fn generate_render_tree(html: &str) -> DomRef {
    let parser = parse_html();
    parser.one(html)
}
