use std::cell::RefCell;
use std::ops::Deref;
use std::rc::{Rc, Weak};

#[derive(Debug, Clone)]
pub struct RenderTree(Rc<RenderTreeNode>);

impl RenderTree {
    pub fn new() -> RenderTree {
        RenderTree(Default::default())
    }

    fn append_child(&self) {
        let new_node = RenderTreeNode {
            parent: Rc::downgrade(&self.0).into(),
            content: None.into(),
        };
        let node_ref = RenderTree(Rc::new(new_node));

        if !self.set_content(Content::Children(vec![node_ref])) {}
    }

    fn insert_parent(&self) {
        let parent_node = RenderTreeNode {
            parent: self.parent.clone(),
            content: Some(Content::Children(vec![self.clone()])).into(),
        };
        let parent_ref = RenderTree(Rc::new(parent_node));

        if let Some(old_parent) = self.parent.borrow().upgrade().map(RenderTree) {
            if let Some(Content::Children(children)) = &mut *old_parent.content.borrow_mut() {
                if let Some(idx) = children.iter().position(|child| Rc::ptr_eq(&child.0, &old_parent.0)) {
                    children[idx] = parent_ref.clone();
                } else {
                    unreachable!()
                }   
            } else {
                // our parent cannot have no children
                unreachable!()
            }
        }

        *self.parent.borrow_mut() = Rc::downgrade(&parent_ref.0);
    }
}

impl Deref for RenderTree {
    type Target = RenderTreeNode;

    fn deref(&self) -> &RenderTreeNode {
        &self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderTreeNode {
    pub parent: RefCell<Weak<RenderTreeNode>>,
    pub content: RefCell<Option<Content>>,
}

impl RenderTreeNode {
    /// Tries to set the new content if the node is currently empty.
    ///
    /// Does nothing (and returns `false`) if the node is not empty.
    fn set_content(&self, new: Content) -> bool {
        if self.content.borrow().is_none() {
            *self.content.borrow_mut() = Some(new);
            true
        } else {
            false
        }
    }

    fn is_empty(&self) -> bool {
        self.content.borrow().is_none()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Frame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    Children(Vec<RenderTree>),
}
