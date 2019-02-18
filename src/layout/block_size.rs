
use super::flow::FlowRef;

#[derive(Debug)]
pub struct BlockSizeContext {}

impl BlockSizeContext {
    pub(super) fn calculate_block_size(&self, flow: FlowRef) {
        flow.assign_block_size(self)
    }
}