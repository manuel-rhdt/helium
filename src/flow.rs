use rctree::Node;

use crate::style::{Display, Style};

pub trait InsertParent {
    fn insert_parent(&mut self, new_parent: Self);
}

pub type FlowRef = Node<Flow>;

impl InsertParent for FlowRef {
    fn insert_parent(&mut self, mut new_parent: Self) {
        self.insert_after(new_parent.clone());
        self.detach();
        new_parent.append(self.clone());
    }
}

#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub flow_type: Option<FlowType>,
    pub style: Style,
}

impl Flow {
    pub fn with_content(content: Option<FlowType>, style: Style) -> Self {
        Flow {
            flow_type: content,
            style: style,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.flow_type.is_none()
    }

    pub fn is_text(&self) -> bool {
        match self.flow_type {
            Some(FlowType::Text(_)) => true,
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

    pub fn into_ref(self) -> FlowRef {
        FlowRef::new(self)
    }
}

#[derive(Debug, Clone)]
pub enum FlowType {
    Text(String),
    Block,
    Inline,
}

pub enum Fragment {}

// macro_rules! render_tree {
//     (<node>{$content:expr}</node>) => {
//         RenderNode::with_content(Some(FlowType::Text(($content).into()))).into_ref()
//     };
//     (<node>$(<node>{$children:tt}</node>)+</node>) => {
//         {
//             let node = RenderNode::with_content(Some(FlowType::Element(vec![]))).into_ref();
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
