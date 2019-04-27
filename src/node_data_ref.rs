// concept and code adapted from kuchiki

use crate::tree::NodeRef;

use std::fmt;
use std::ops::Deref;

/// Holds a strong reference to a node, but dereferences to some component inside of it.
#[derive(Eq)]
pub struct NodeDataRef<Node, Data> {
    keep_alive: NodeRef<Node>,
    reference: *const Data,
}

impl<Node, Data> NodeDataRef<Node, Data> {
    /// Create a `NodeDataRef` for a component in a given node.
    pub fn new(rc: NodeRef<Node>, f: impl FnOnce(&Node) -> &Data) -> NodeDataRef<Node, Data> {
        NodeDataRef {
            reference: f(&*rc),
            keep_alive: rc,
        }
    }

    /// Create a `NodeDataRef` for and a component that may or may not be in a given node.
    pub fn new_opt(
        rc: NodeRef<Node>,
        f: impl FnOnce(&Node) -> Option<&Data>,
    ) -> Option<NodeDataRef<Node, Data>> {
        f(&*rc).map(|r| r as *const Data).map(move |r| NodeDataRef {
            reference: r,
            keep_alive: rc,
        })
    }

    /// Access the corresponding node.
    pub fn as_node(&self) -> &NodeRef<Node> {
        &self.keep_alive
    }
}

impl<Node, Data> Deref for NodeDataRef<Node, Data> {
    type Target = Data;
    fn deref(&self) -> &Data {
        unsafe { &*self.reference }
    }
}

// #[derive(PartialEq)] would compare both fields
impl<Node, Data> PartialEq for NodeDataRef<Node, Data> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.keep_alive == other.keep_alive
    }
}

// #[derive(Clone)] would have an unnecessary `T: Clone` bound
impl<Node: Clone, Data> Clone for NodeDataRef<Node, Data> {
    #[inline]
    fn clone(&self) -> Self {
        NodeDataRef {
            keep_alive: self.keep_alive.clone(),
            reference: self.reference,
        }
    }
}

impl<Node, Data: fmt::Debug> fmt::Debug for NodeDataRef<Node, Data> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Debug::fmt(&**self, f)
    }
}
