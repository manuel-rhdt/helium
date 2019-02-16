use super::flow::FlowRef;
use crate::traverse::Traversal;

pub struct LayoutSizeComputation {}

impl Traversal<FlowRef> for LayoutSizeComputation {
    fn preorder_traverse(&mut self, _node: FlowRef) {
        // crate::style::style_for_element(node);
    }

    fn postorder_traverse(&mut self, node: FlowRef) {}
}
