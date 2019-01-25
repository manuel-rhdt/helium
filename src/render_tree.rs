use crate::style::{Display, Style};

use std::cell::RefCell;
use std::ops::Deref;
use std::rc::{Rc, Weak};

#[derive(Default, Debug, Clone)]
pub struct RenderNodeRef(Rc<RenderNode>);

impl RenderNodeRef {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_content(content: Option<NodeType>, style: Style) -> Self {
        RenderNodeRef(Rc::new(RenderNode::with_content(content, style)))
    }

    pub fn append_child(&self, node_ref: RenderNodeRef) -> RenderNodeRef {
        // text nodes cannot have children
        assert!(!self.is_text());

        *node_ref.parent.borrow_mut() = Rc::downgrade(&self.0).into();

        if !self.set_content(NodeType::Element(vec![node_ref.clone()])) {
            match &mut *self.node_type.borrow_mut() {
                Some(NodeType::Element(children)) => children.push(node_ref.clone()),
                _ => unreachable!(),
            }
        }

        node_ref
    }

    /// Insert a new parent above the current node and return a reference to it.
    pub fn insert_parent(&self) -> RenderNodeRef {
        let parent_node = RenderNode {
            parent: self.parent.clone(),
            node_type: Some(NodeType::Element(vec![self.clone()])).into(),
            ..Default::default()
        };
        let parent_ref = RenderNodeRef(Rc::new(parent_node));

        if let Some(old_parent) = self.parent.borrow().upgrade().map(RenderNodeRef) {
            if let Some(NodeType::Element(children)) = &mut *old_parent.node_type.borrow_mut() {
                // look through the parent's children to find ourselves and replace us with the new parent.
                if let Some(idx) = children
                    .iter()
                    .position(|child| Rc::ptr_eq(&child.0, &old_parent.0))
                {
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

        parent_ref
    }

    // useful for tests
    pub(crate) fn is_well_formed(&self) -> bool {
        match &*self.node_type.borrow() {
            Some(NodeType::Element(children)) => {
                for child in children {
                    if child.parent_ref().is_none() {
                        return false;
                    }
                    if !Rc::ptr_eq(&child.parent_ref().unwrap().0, &self.0) {
                        return false;
                    }
                    if !child.is_well_formed() {
                        return false;
                    }
                }
                true
            }
            _ => true,
        }
    }
}

impl Deref for RenderNodeRef {
    type Target = RenderNode;

    fn deref(&self) -> &RenderNode {
        &self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderNode {
    pub parent: RefCell<Weak<RenderNode>>,
    pub node_type: RefCell<Option<NodeType>>,
    pub style: RefCell<Style>,
}

impl RenderNode {
    pub fn with_content(content: Option<NodeType>, style: Style) -> Self {
        RenderNode {
            node_type: RefCell::new(content),
            style: RefCell::new(style),
            ..Default::default()
        }
    }

    pub fn parent_ref(&self) -> Option<RenderNodeRef> {
        self.parent
            .borrow()
            .upgrade()
            .map(|parent| RenderNodeRef(parent))
    }

    /// Tries to set the new content if the node is currently empty.
    ///
    /// Does nothing (and returns `false`) if the node is not empty.
    fn set_content(&self, new: NodeType) -> bool {
        if self.node_type.borrow().is_none() {
            *self.node_type.borrow_mut() = Some(new);
            true
        } else {
            false
        }
    }

    pub fn set_style(&self, style_setter: impl FnOnce(&Style) -> Style) {
        let new_style = style_setter(&*self.style.borrow());
        *self.style.borrow_mut() = new_style;
    }

    pub fn is_empty(&self) -> bool {
        self.node_type.borrow().is_none()
    }

    pub fn is_text(&self) -> bool {
        match &*self.node_type.borrow() {
            Some(NodeType::Text(_)) => true,
            _ => false,
        }
    }

    pub fn formatting_context(&self) -> FormattingContext {
        match *self.style.borrow() {
            Style {
                display: Display::Inline,
                ..
            } => FormattingContext::Inline,
            _ => FormattingContext::Block,
        }
    }

    pub fn make_page(&self) {
        let new_node = match self.node_type.borrow_mut().take() {
            Some(NodeType::Text(_)) => panic!("A text node cannot become a page"),
            Some(NodeType::Element(children)) | Some(NodeType::Page(children)) => {
                NodeType::Page(children)
            }
            None => panic!("An empty node cannot beome a page"),
        };
        *self.node_type.borrow_mut() = Some(new_node);
    }

    pub fn into_ref(self) -> RenderNodeRef {
        RenderNodeRef(Rc::new(self))
    }
}

#[derive(Debug, Clone)]
pub enum NodeType {
    Text(String),
    Element(Vec<RenderNodeRef>),
    Page(Vec<RenderNodeRef>),
}

// macro_rules! render_tree {
//     (<node>{$content:expr}</node>) => {
//         RenderNode::with_content(Some(NodeType::Text(($content).into()))).into_ref()
//     };
//     (<node>$(<node>{$children:tt}</node>)+</node>) => {
//         {
//             let node = RenderNode::with_content(Some(NodeType::Element(vec![]))).into_ref();
//             $(
//                 let child = render_tree!(<node>{$children}</node>);
//                 node.append_child(child);
//             )+
//             node
//         }
//     };
// }

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormattingContext {
    Block,
    Inline,
}

#[cfg(test)]
mod test {
    use super::*;

    // #[test]
    // fn test_insert_parent() {
    //     let render_tree = render_tree!(<node>{ "Hello" }</node>);
    // }

    // #[test]
    // fn test_render_tree_macro() {
    //     let render_tree = render_tree!(<node>{ "Hello" }</node>);
    //     assert!(render_tree.is_well_formed());
    //     let render_tree =
    //         render_tree!(<node><node>{ "Hello" }</node><node>{ "World!" }</node></node>);
    //     assert!(render_tree.is_well_formed());
    // }
}
