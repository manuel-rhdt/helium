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

impl ElementData {
    /// The element that stands for the generated content of `::before` or `::after`, which the
    /// layout adds to the tree (it has no namespace and a name that no element can have).
    pub fn pseudo(pseudo: crate::style::parse::PseudoElement) -> Self {
        use crate::style::parse::PseudoElement;
        ElementData {
            name: QualName::new(
                None,
                html5ever::ns!(),
                html5ever::LocalName::from(match pseudo {
                    PseudoElement::Before => "::before",
                    PseudoElement::After => "::after",
                    PseudoElement::Marker => "::marker",
                }),
            ),
            attributes: Default::default(),
            template_contents: None,
        }
    }

    /// Which pseudo-element this is, if it is not an element of the document.
    pub fn pseudo_element(&self) -> Option<crate::style::parse::PseudoElement> {
        use crate::style::parse::PseudoElement;
        if self.name.ns != html5ever::ns!() {
            return None;
        }
        match &*self.name.local {
            "::before" => Some(PseudoElement::Before),
            "::after" => Some(PseudoElement::After),
            "::marker" => Some(PseudoElement::Marker),
            _ => None,
        }
    }
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

/// The deepest nesting of elements that a document has, as in browsers (Chrome has 512). Recursion
/// over the tree (the style cascade, the layout, the drop of the tree) needs stack for every
/// level, so a document that nests deeper would overflow it. An element that would be deeper is
/// added to the deepest ancestor that can still have children: the tree is flattened there.
pub const MAX_DEPTH: usize = 512;

/// Where a child of `parent` is added: `parent`, or its nearest ancestor that is above [`MAX_DEPTH`].
fn clamp_parent(parent: &NodeRef<NodeData>) -> NodeRef<NodeData> {
    let mut depth = 0;
    let mut ancestor = parent.parent();
    while let Some(node) = ancestor {
        depth += 1;
        ancestor = node.parent();
    }
    let mut target = parent.clone();
    while depth >= MAX_DEPTH {
        match target.parent() {
            Some(above) => target = above,
            None => break,
        }
        depth -= 1;
    }
    target
}

pub struct Sink {
    document_node: NodeRef<NodeData>,
    on_parse_error: RefCell<Option<Box<dyn FnMut(Cow<'static, str>)>>>,
}

impl TreeSink for Sink {
    type Output = NodeRef<NodeData>;

    fn finish(self) -> Self::Output {
        self.document_node
    }

    type Handle = NodeRef<NodeData>;
    type ElemName<'a> = ExpandedName<'a>;

    #[inline]
    fn parse_error(&self, message: Cow<'static, str>) {
        if let Some(handler) = self.on_parse_error.borrow_mut().as_mut() {
            handler(message)
        }
    }

    #[inline]
    fn get_document(&self) -> Self::Handle {
        self.document_node.clone()
    }

    #[inline]
    fn set_quirks_mode(&self, mode: QuirksMode) {
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
    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> ExpandedName<'a> {
        target.as_element().unwrap().name.expanded()
    }

    /// `<annotation-xml encoding="text/html">` (or `application/xhtml+xml`) holds HTML: without this,
    /// the parser treats its children as breaking out of the formula.
    /// https://html.spec.whatwg.org/multipage/parsing.html#html-integration-point
    fn is_mathml_annotation_xml_integration_point(&self, handle: &Self::Handle) -> bool {
        let Some(element) = handle.as_element() else {
            return false;
        };
        element.name.ns == html5ever::ns!(mathml)
            && &*element.name.local == "annotation-xml"
            && element
                .attributes
                .borrow()
                .get(&QualName::new(None, html5ever::ns!(), html5ever::local_name!("encoding")))
                .is_some_and(|encoding| {
                    encoding.value.eq_ignore_ascii_case("text/html")
                        || encoding.value.eq_ignore_ascii_case("application/xhtml+xml")
                })
    }

    #[inline]
    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        // The prefix of an element (`<s:svg>`, from an XML parser) has no meaning once its
        // namespace is known, and the rest of Umbruch compares whole names (`QualName` equality
        // includes the prefix), so it is dropped here.
        let name = QualName::new(None, name.ns, name.local);
        // the content of a `<template>` is a tree of its own, which is not part of the document
        let template_contents = flags
            .template
            .then(|| NodeRef::new(NodeData::from_html_data(ParsedData::DocumentFragment)));
        NodeRef::new(NodeData::from_html_data(ParsedData::Element(ElementData {
            template_contents,
            name,
            attributes: RefCell::new(
                attrs
                    .into_iter()
                    .map(|attr| {
                        let Attribute { name, .. } = &attr;
                        (name.clone(), attr)
                    })
                    .collect(),
            ),
        })))
    }

    #[inline]
    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        NodeRef::new_comment(text)
    }

    #[inline]
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        NodeRef::new(NodeData::from_html_data(ParsedData::ProcessingInstruction(
            target.into(),
            data.into(),
        )))
    }

    #[inline]
    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => clamp_parent(parent).append(node),
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
    fn append_before_sibling(&self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
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
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.document_node
            .append(NodeRef::new_doctype(name, public_id, system_id))
    }

    #[inline]
    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
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
    fn remove_from_parent(&self, target: &Self::Handle) {
        target.detach()
    }

    #[inline]
    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        // FIXME: Can this be done more effciently in rctree,
        // by moving the whole linked list of children at once?
        for child in node.children() {
            new_parent.append(child)
        }
    }

    #[inline]
    fn mark_script_already_started(&self, _node: &Self::Handle) {
        // FIXME: Is this useful outside of a browser?
    }

    #[inline]
    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        target
            .as_element()
            .unwrap()
            .template_contents
            .clone()
            .unwrap()
    }

    fn append_based_on_parent_node(
        &self,
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

fn new_sink(on_parse_error: Option<Box<dyn FnMut(Cow<'static, str>)>>) -> Sink {
    Sink {
        document_node: NodeRef::new(NodeData::from_html_data(ParsedData::Document(
            DocumentData {
                _quirks_mode: QuirksMode::NoQuirks.into(),
            },
        ))),
        on_parse_error: RefCell::new(on_parse_error),
    }
}

fn parse_html() -> html5ever::Parser<Sink> {
    html5ever::parse_document(new_sink(None), html5ever::ParseOpts::default())
}

pub fn generate_render_tree(html: &str) -> DomRef {
    let parser = parse_html();
    parser.one(html)
}

/// Reads the text as XML (well-formed, with namespaces: `xmlns` and prefixes are resolved),
/// which is what a `.svg` or `.xhtml` file is. Unlike the HTML parser it does not repair: the
/// first error is returned.
pub fn generate_render_tree_xml(text: &str) -> Result<DomRef, String> {
    let errors = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = new_sink(Some(Box::new({
        let errors = errors.clone();
        move |message| errors.borrow_mut().push(message.into_owned())
    })));
    let document = xml5ever::driver::parse_document(sink, xml5ever::driver::XmlParseOpts::default())
        .one(text);
    let first = errors.borrow().first().cloned();
    match first {
        Some(message) => Err(format!("not well-formed XML: {message}")),
        None => Ok(document),
    }
}

/// Makes the tree of an XML document one that the layout can use, which expects an `html`
/// element with a `body`: an XHTML document is as it is, and an SVG document becomes the only
/// content of the body of a document whose page has the size of the drawing.
pub fn into_html_document(document: &DomRef) -> Result<DomRef, String> {
    use html5ever::{local_name, ns};
    let root = document
        .children()
        .find(|child| child.as_element().is_some())
        .ok_or("the document has no root element")?;
    let element = root.as_element().expect("an element");
    let is = |namespace, local| element.name.ns == namespace && element.name.local == local;
    if is(ns!(html), local_name!("html")) {
        return Ok(document.clone());
    }
    if !is(ns!(svg), local_name!("svg")) {
        return Err(format!(
            "the root element <{}> is neither SVG nor XHTML",
            element.name.local
        ));
    }
    let attribute = |name: &str| {
        let key = QualName::new(None, ns!(), html5ever::LocalName::from(name));
        element.attributes.borrow().get(&key).map(|attribute| attribute.value.to_string())
    };
    let css = crate::svg_input::page_css(
        attribute("width").as_deref(),
        attribute("height").as_deref(),
        attribute("viewBox").as_deref(),
    );
    let html_element = |name| NodeRef::new_element(QualName::new(None, ns!(html), name), []);
    let html = html_element(local_name!("html"));
    let head = html_element(local_name!("head"));
    let style = html_element(local_name!("style"));
    let body = html_element(local_name!("body"));
    style.append(NodeRef::new_text(css.as_str()));
    head.append(style);
    html.append(head);
    // (appending detaches the root element from the document)
    body.append(root);
    html.append(body);
    document.append(html);
    Ok(document.clone())
}
