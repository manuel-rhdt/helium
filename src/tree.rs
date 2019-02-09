use std::cell::Cell;
use std::fmt::Debug;
use std::ops::Deref;
use std::rc::{Rc, Weak};

use crate::layout::flow::FlowRef;

pub type DomRef = NodeRef<NodeData>;

#[derive(Debug, Clone)]
pub struct NodeRef<T>(Rc<T>);

impl<T> PartialEq for NodeRef<T> {
    fn eq(&self, other: &NodeRef<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for NodeRef<T> {}

#[derive(Default)]
pub struct Links<StrongLink, WeakLink> {
    parent: Cell<Option<WeakLink>>,
    next: Cell<Option<StrongLink>>,
    previous: Cell<Option<WeakLink>>,
    first_child: Cell<Option<StrongLink>>,
    last_child: Cell<Option<WeakLink>>,
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

impl<StrongLink: Debug, WeakLink> Debug for Links<StrongLink, WeakLink> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let first_child = self.first_child.take();
        write!(f, "{{\n{:?}\n}}, ", first_child)?;
        self.first_child.set(first_child);

        let next = self.next.take();
        write!(f, "{:?},\n", next)?;
        self.next.set(next);

        Ok(())
    }
}

pub trait TreeLinked: Sized + Clone + Eq {
    /// A reference to another node. While a `StrongLink` pointing to a node exists, the node can't be destroyed.
    type StrongLink: Clone;

    /// A reference to another node in the tree that will be broken when the referred-to node gets destroyed.
    type WeakLink: Clone;

    fn links(&self) -> &Links<Self::StrongLink, Self::WeakLink>;

    fn strong_link(self) -> Self::StrongLink;

    fn weak_link(&self) -> Self::WeakLink;

    /// Construct a node from a weak link. If the weak link is broken return None.
    fn from_weak(weak: &Self::WeakLink) -> Option<Self>;

    /// Construct a node from a strong link.
    fn from_strong(strong: &Self::StrongLink) -> Self;

    /// Returns a reference to the parent node or `None` if this is a root node.
    fn parent(&self) -> Option<Self> {
        let parent = self.links().parent.take();
        let parent_ref = parent.as_ref().and_then(Self::from_weak);
        self.links().parent.set(parent);
        parent_ref
    }

    fn next_sibling(&self) -> Option<Self> {
        let sibling = self.links().next.take();
        let sibling_ref = sibling.as_ref().map(Self::from_strong);
        self.links().next.set(sibling);
        sibling_ref
    }

    fn previous_sibling(&self) -> Option<Self> {
        let sibling = self.links().previous.take();
        let sibling_ref = sibling.as_ref().and_then(Self::from_weak);
        self.links().previous.set(sibling);
        sibling_ref
    }

    fn first_child(&self) -> Option<Self> {
        let child = self.links().first_child.take();
        let child_ref = child.as_ref().map(Self::from_strong);
        self.links().first_child.set(child);
        child_ref
    }

    fn last_child(&self) -> Option<Self> {
        let child = self.links().last_child.take();
        let child_ref = child.as_ref().and_then(Self::from_weak);
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

        let next_sibling_ref = next_sibling.as_ref().map(Self::from_strong);
        let previous_sibling_ref = previous_sibling.as_ref().and_then(Self::from_weak);

        next_sibling_ref.map(|node| node.links().previous.set(previous_sibling.clone()));
        previous_sibling_ref.map(|node| node.links().next.set(next_sibling.clone()));

        if let Some(parent) = self
            .links()
            .parent
            .take()
            .as_ref()
            .and_then(Self::from_weak)
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
        } else {
            self.parent().map(|parent| {
                parent
                    .links()
                    .first_child
                    .set(Some(new_sibling.clone().strong_link()))
            });
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
}

impl TreeLinked for NodeRef<NodeData> {
    type StrongLink = Rc<NodeData>;
    type WeakLink = Weak<NodeData>;

    fn links(&self) -> &Links<Self::StrongLink, Self::WeakLink> {
        &self.0.links
    }

    fn from_strong(strong: &Self::StrongLink) -> Self {
        NodeRef(strong.clone())
    }

    fn from_weak(weak: &Self::WeakLink) -> Option<Self> {
        weak.upgrade().map(NodeRef)
    }

    fn strong_link(self) -> Self::StrongLink {
        self.0
    }

    fn weak_link(&self) -> Self::WeakLink {
        Rc::downgrade(&self.0)
    }
}

pub struct NodeData {
    links: Links<Rc<NodeData>, Weak<NodeData>>,

    pub html_data: crate::parser::ParsedData,

    pub contributed_flows: Cell<Option<FlowRef>>,
}

impl Clone for NodeData {
    fn clone(&self) -> Self {
        let contributed_flows = self.contributed_flows.take();
        self.contributed_flows.set(contributed_flows.clone());
        NodeData {
            links: self.links.clone(),
            html_data: self.html_data.clone(),
            contributed_flows: Cell::new(contributed_flows),
        }
    }
}

impl Debug for NodeData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("NodeData")
            .field("links", &self.links)
            .field("html_data", &self.html_data)
            //.field("contributed_flows", &self.contributed_flows.clone())
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
            contributed_flows: None.into(),
        }
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

#[derive(Debug, Clone)]
pub struct SiblingIter<Node> {
    current: Option<Node>,
}

impl<Node: TreeLinked> Iterator for SiblingIter<Node> {
    type Item = Node;

    fn next(&mut self) -> Option<Node> {
        self.current = self.current.as_ref().and_then(|node| node.next_sibling());
        self.current.clone()
    }
}
