use rctree::Node;

use crate::style::{Display, Style};

pub trait InsertParent {
    fn insert_parent(&mut self, new_parent: Self);
}

pub type RenderNodeRef = Node<RenderNode>;

impl InsertParent for RenderNodeRef {
    fn insert_parent(&mut self, mut new_parent: Self) {
        self.insert_after(new_parent.clone());
        self.detach();
        new_parent.append(self.clone());
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderNode {
    pub node_type: Option<NodeType>,
    pub style: Style,
}

impl RenderNode {
    pub fn with_content(content: Option<NodeType>, style: Style) -> Self {
        RenderNode {
            node_type: content,
            style: style,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.node_type.is_none()
    }

    pub fn is_text(&self) -> bool {
        match self.node_type {
            Some(NodeType::Text(_)) => true,
            _ => false,
        }
    }

    pub fn formatting_context(&self) -> FormattingContext {
        match self.style {
            Style {
                display: Display::Inline,
                ..
            } => FormattingContext::Inline,
            _ => FormattingContext::Block,
        }
    }

    pub fn make_page(&mut self) {
        let new_node = match self.node_type.take() {
            Some(NodeType::Text(_)) => panic!("A text node cannot become a page"),
            Some(NodeType::Element(children)) | Some(NodeType::Page(children)) => {
                NodeType::Page(children)
            }
            None => panic!("An empty node cannot beome a page"),
        };
        self.node_type = Some(new_node);
    }

    pub fn into_ref(self) -> RenderNodeRef {
        RenderNodeRef::new(self)
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
