use std::cell::Cell;
use std::fmt::Debug;
use std::ops::Deref;
use std::rc::{Rc, Weak};

use crate::dot::{Connection, DotGraphSerializer};
use crate::node_data_ref::NodeDataRef;
use crate::parser::{ElementData, ParsedData};
use crate::style::properties::computed::ComputedValues;

pub type DomRef = NodeRef<NodeData>;
pub type ElementRef = NodeDataRef<NodeData, ElementData>;

#[derive(Clone)]
pub struct NodeRef<T>(Rc<T>);

impl Debug for NodeRef<NodeData> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("NodeRef").field("value", &self.0).finish()
    }
}

impl<T> PartialEq for NodeRef<T> {
    fn eq(&self, other: &NodeRef<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for NodeRef<T> {}

/// A structure that contains the necessary relationships to describe a tree structure.
///
/// `Links` contains both weak and strong links as follows
/// * `parent` is a weak link to the parent
/// * `next` is a strong link to the following sibling
/// * `previous` is a weak link to the previous sibling
/// * `first_child` is a strong link to the first child
/// * `last_child` is a weak link to the last child
pub struct Links<StrongLink, WeakLink> {
    parent: Cell<Option<WeakLink>>,
    next: Cell<Option<StrongLink>>,
    previous: Cell<Option<WeakLink>>,
    first_child: Cell<Option<StrongLink>>,
    last_child: Cell<Option<WeakLink>>,
}

impl<T, U> Default for Links<T, U> {
    fn default() -> Self {
        Links {
            parent: None.into(),
            next: None.into(),
            previous: None.into(),
            first_child: None.into(),
            last_child: None.into(),
        }
    }
}

impl<T: Clone, U: Clone> Clone for Links<T, U> {
    fn clone(&self) -> Self {
        let parent = self.parent.take();
        let next = self.next.take();
        let previous = self.previous.take();
        let first_child = self.first_child.take();
        let last_child = self.last_child.take();
        let new_links = Links {
            parent: parent.clone().into(),
            next: next.clone().into(),
            previous: previous.clone().into(),
            first_child: first_child.clone().into(),
            last_child: last_child.clone().into(),
        };
        self.parent.set(parent);
        self.next.set(next);
        self.previous.set(previous);
        self.first_child.set(first_child);
        self.last_child.set(last_child);
        new_links
    }
}

impl<NodeRef, StrongLink, WeakLink> Links<StrongLink, WeakLink>
where
    StrongLink: Link<Target = NodeRef>,
    WeakLink: Link<Target = Option<NodeRef>>,
{
    /// Returns the ref to the parent, if set.
    pub fn parent(&self) -> Option<NodeRef> {
        let parent = self.parent.take();
        self.parent.set(parent.clone());
        parent.as_ref().and_then(WeakLink::get)
    }
}

impl<StrongLink: Debug, WeakLink> Debug for Links<StrongLink, WeakLink> {
    fn fmt(&self, _f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        // let first_child = self.first_child.take();
        // write!(f, "{{\n{:?}\n}}, ", first_child)?;
        // self.first_child.set(first_child);

        // let next = self.next.take();
        // write!(f, "{:?},\n", next)?;
        // self.next.set(next);

        Ok(())
    }
}

/// A trait to describe a tree made up of nodes and so-called "links" between
/// them.
///
/// This trait is implemented for nodes in the tree and provides methods to
/// retrieve related nodes through convenient functions.
pub trait TreeLinked: Sized + Clone + Eq {
    /// A reference to another node. While a `StrongLink` pointing to a node
    /// exists, the node will be kept alive.
    type StrongLink: Link<Target = Self>;

    /// A reference to another node in the tree that will be broken when the
    /// referred-to node gets destroyed.
    type WeakLink: Link<Target = Option<Self>>;

    fn links(&self) -> &Links<Self::StrongLink, Self::WeakLink>;

    fn strong_link(self) -> Self::StrongLink;

    fn weak_link(&self) -> Self::WeakLink;

    /// Returns a reference to the parent node or `None` if this is a root node.
    fn parent(&self) -> Option<Self> {
        self.links().parent()
    }

    fn next_sibling(&self) -> Option<Self> {
        let sibling = self.links().next.take();
        let sibling_ref = sibling.as_ref().map(Self::StrongLink::get);
        self.links().next.set(sibling);
        sibling_ref
    }

    fn previous_sibling(&self) -> Option<Self> {
        let sibling = self.links().previous.take();
        let sibling_ref = sibling.as_ref().and_then(Self::WeakLink::get);
        self.links().previous.set(sibling);
        sibling_ref
    }

    fn first_child(&self) -> Option<Self> {
        let child = self.links().first_child.take();
        let child_ref = child.as_ref().map(Self::StrongLink::get);
        self.links().first_child.set(child);
        child_ref
    }

    fn last_child(&self) -> Option<Self> {
        let child = self.links().last_child.take();
        let child_ref = child.as_ref().and_then(Self::WeakLink::get);
        self.links().last_child.set(child);
        child_ref
    }

    /// Returns true if this is a root node.
    fn is_root(&self) -> bool {
        self.parent().is_none()
    }

    /// Removes the node from its parent and siblings. After calling this function the node will be
    /// a root node.
    fn detach(&self) {
        let next_sibling = self.links().next.take();
        let previous_sibling = self.links().previous.take();

        let next_sibling_ref = next_sibling.as_ref().map(Self::StrongLink::get);
        let previous_sibling_ref = previous_sibling.as_ref().and_then(Self::WeakLink::get);

        if let Some(node) = next_sibling_ref {
            node.links().previous.set(previous_sibling.clone());
        }
        if let Some(node) = previous_sibling_ref {
            node.links().next.set(next_sibling.clone());
        }

        if let Some(parent) = self
            .links()
            .parent
            .take()
            .as_ref()
            .and_then(Self::WeakLink::get)
        {
            if parent.first_child().as_ref() == Some(self) {
                parent.links().first_child.set(next_sibling);
            }
            if parent.last_child().as_ref() == Some(self) {
                parent.links().last_child.set(previous_sibling);
            }
        }
    }

    fn insert_before(&self, new_sibling: Self) {
        new_sibling.detach();

        if let Some(parent) = self.parent() {
            new_sibling.links().parent.set(Some(parent.weak_link()))
        }

        if let Some(previous) = self.previous_sibling() {
            new_sibling.links().previous.set(Some(previous.weak_link()));
            previous
                .links()
                .next
                .set(Some(new_sibling.clone().strong_link()));
        } else if let Some(parent) = self.parent() {
            parent
                .links()
                .first_child
                .set(Some(new_sibling.clone().strong_link()))
        }

        self.links().previous.set(Some(new_sibling.weak_link()));
        new_sibling
            .links()
            .next
            .set(Some(self.clone().strong_link()));
    }

    fn append(&self, new_child: Self) {
        new_child.detach();
        new_child.links().parent.set(Some(self.weak_link()));

        if let Some(last) = self.last_child() {
            // we already have children
            self.links().last_child.set(Some(new_child.weak_link()));
            new_child.links().previous.set(Some(last.weak_link()));
            last.links().next.set(Some(new_child.strong_link()));
        } else {
            self.links().last_child.set(Some(new_child.weak_link()));
            self.links().first_child.set(Some(new_child.strong_link()));
        }
    }

    fn children(&self) -> SiblingIter<Self> {
        SiblingIter {
            current: self.first_child(),
        }
    }

    fn following_siblings(&self) -> SiblingIter<Self> {
        SiblingIter {
            current: Some(self.clone()),
        }
    }

    fn preceding_siblings(&self) -> PrecedingSiblingIter<Self> {
        PrecedingSiblingIter {
            current: Some(self.clone()),
        }
    }
}

/// A trait to abstract over both, weak and strong links in a tree.
pub trait Link: Clone {
    type Target;

    fn get(&self) -> Self::Target;
}

impl Link for Rc<NodeData> {
    type Target = NodeRef<NodeData>;

    fn get(&self) -> Self::Target {
        NodeRef(self.clone())
    }
}

impl Link for Weak<NodeData> {
    type Target = Option<NodeRef<NodeData>>;

    fn get(&self) -> Self::Target {
        self.upgrade().map(NodeRef)
    }
}

impl TreeLinked for NodeRef<NodeData> {
    type StrongLink = Rc<NodeData>;
    type WeakLink = Weak<NodeData>;

    fn links(&self) -> &Links<Self::StrongLink, Self::WeakLink> {
        &self.0.links
    }

    fn strong_link(self) -> Self::StrongLink {
        self.0
    }

    fn weak_link(&self) -> Self::WeakLink {
        Rc::downgrade(&self.0)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct OpaqueNode(pub usize);

pub trait GetOpaqueNode {
    fn get_opaque(&self) -> OpaqueNode;
}

impl GetOpaqueNode for NodeRef<NodeData> {
    fn get_opaque(&self) -> OpaqueNode {
        let node_data: *const NodeData = &**self;
        OpaqueNode(node_data as usize)
    }
}

pub struct NodeData {
    links: Links<Rc<NodeData>, Weak<NodeData>>,

    pub html_data: crate::parser::ParsedData,

    computed_values: Cell<Option<ComputedValues>>,

    pub layout_entity: Cell<Option<crate::layout::components::LayoutEntity>>,
}

impl Clone for NodeData {
    fn clone(&self) -> Self {
        let computed_values = self.computed_values.take();
        self.computed_values.set(computed_values.clone());

        NodeData {
            links: self.links.clone(),
            html_data: self.html_data.clone(),
            computed_values: Cell::new(computed_values),
            layout_entity: self.layout_entity.clone(),
        }
    }
}

impl Debug for NodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("NodeData")
            .field("html_data", &self.html_data)
            .finish()
    }
}

impl NodeData {
    pub fn from_html_data(html_data: crate::parser::ParsedData) -> Self {
        NodeData {
            links: Links {
                parent: None.into(),
                next: None.into(),
                previous: None.into(),
                first_child: None.into(),
                last_child: None.into(),
            },
            html_data,
            computed_values: None.into(),
            layout_entity: None.into(),
        }
    }

    pub fn style(&self) -> Option<ComputedValues> {
        let vals = self.computed_values.take();
        self.computed_values.set(vals.clone());
        vals
    }

    pub fn set_style(&self, style: ComputedValues) {
        self.computed_values.set(Some(style))
    }
}

impl<Data> Deref for NodeRef<Data> {
    type Target = Data;

    fn deref(&self) -> &Data {
        &*self.0
    }
}

impl<Data> NodeRef<Data> {
    pub fn new(node_data: Data) -> Self {
        NodeRef(Rc::new(node_data))
    }
}

impl DomRef {
    /// If this node is an element, return a strong reference to element-specific data.
    pub fn into_element_ref(self) -> Option<ElementRef> {
        NodeDataRef::new_opt(self, |node| match &node.html_data {
            ParsedData::Element(element_data) => Some(element_data),
            _ => None,
        })
    }

    pub fn serialize_dot_graph(&self, out: &mut dyn std::io::Write) -> std::io::Result<()> {
        let mut serializer = DotGraphSerializer::new(out);
        serializer.begin_graph("G")?;
        serializer.push_node(self.clone(), Connection::Root)?;
        self.serialize_children_and_flows(&mut serializer)?;
        serializer.end_graph()?;
        Ok(())
    }

    fn serialize_children_and_flows(
        &self,
        serializer: &mut DotGraphSerializer<&mut dyn std::io::Write>,
    ) -> std::io::Result<()> {
        for child in self.children() {
            serializer.push_node(child.clone(), Connection::DescendantOf(self.get_opaque()))?;
            child.serialize_children_and_flows(serializer)?;
        }
        Ok(())
    }
}

use crate::{
    dot::{GraphDisplayStyle, Shape, Style},
    layout::components::LayoutEntity,
};

impl crate::dot::GraphStyle for DomRef {
    fn graph_style(&self) -> GraphDisplayStyle {
        let entity = self.layout_entity.get().map(|LayoutEntity(ent)| ent.id());
        let entity = if let Some(ent) = entity {
            format!("Entity({}): ", ent)
        } else {
            format!("")
        };

        match &self.html_data {
            ParsedData::Element(ElementData { name, .. }) => GraphDisplayStyle::new()
                .label(format!("{}{}", entity, name.local))
                .shape(Shape::Box),
            ParsedData::Doctype(_) => GraphDisplayStyle::new().label("doctype"),
            ParsedData::Document(_) => GraphDisplayStyle::new()
                .label("document")
                .style(Style::Filled(0.5, 0.5, 0.5)),
            ParsedData::Text(text) => {
                GraphDisplayStyle::new().label(format!("{}{:?}", entity, text.borrow().clone()))
            }
            _ => Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SiblingIter<Node> {
    current: Option<Node>,
}

impl<Node: TreeLinked> Iterator for SiblingIter<Node> {
    type Item = Node;

    fn next(&mut self) -> Option<Node> {
        let return_value = self.current.clone();
        self.current = self.current.as_ref().and_then(TreeLinked::next_sibling);
        return_value
    }
}

#[derive(Debug, Clone)]
pub struct PrecedingSiblingIter<Node> {
    current: Option<Node>,
}

impl<Node: TreeLinked> Iterator for PrecedingSiblingIter<Node> {
    type Item = Node;

    fn next(&mut self) -> Option<Node> {
        let return_value = self.current.clone();
        self.current = self.current.as_ref().and_then(TreeLinked::previous_sibling);
        return_value
    }
}
