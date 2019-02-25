
use super::flow::FlowRef;

#[derive(Debug)]
pub struct BlockSizeContext {}

impl BlockSizeContext {
    pub(super) fn calculate_block_size(&self, flow: FlowRef) {
        let flow_ref = flow.clone();
        flow.assign_block_size(flow_ref, self)
    }
}