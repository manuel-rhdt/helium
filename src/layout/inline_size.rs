use super::flow::FlowRef;
use super::block_size::BlockSizeContext;
use crate::traverse::Traversal;

#[derive(Debug)]
pub struct LayoutSizeComputation {
    inline_context: InlineSizeContext,
    block_context: BlockSizeContext,
}

impl Traversal<FlowRef> for LayoutSizeComputation {
    /// Compute the inline sizes of all elements.
    fn preorder_traverse(&mut self, node: FlowRef) {
        self.inline_context.calculate_inline_width(node)
    }

    fn postorder_traverse(&mut self, node: FlowRef) {
        self.block_context.calculate_block_size(node)
    }
}

#[derive(Debug)]
pub struct InlineSizeContext {
    pub page_size: f32,
}

impl InlineSizeContext {
    fn calculate_inline_width(&self, node: FlowRef) {
        node.0.assign_inline_sizes(self)
    }
}