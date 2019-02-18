use super::flow::{Flow, FlowBase, FlowRef};
use super::fragment::Fragment;
use super::inline_size::InlineSizeContext;
use crate::style::Style;

use std::rc::Rc;

#[derive(Debug)]
pub struct BlockFlow {
    base: FlowBase,
    fragment: Option<Fragment>,
}

impl Flow for BlockFlow {
    fn flow_base(&self) -> &FlowBase {
        &self.base
    }

    fn assign_inline_sizes(&self, context: &InlineSizeContext) {
        let parent_size = self
            .parent_flow()
            .map(|parent| parent.flow_base().inline_size.get())
            .unwrap_or(context.page_size);
        self.base.inline_size.set(parent_size);
    }
}

impl BlockFlow {
    pub fn new(fragment: Option<Fragment>, style: Style) -> Self {
        BlockFlow {
            base: FlowBase::new(style),
            fragment,
        }
    }

    pub fn into_ref(self) -> FlowRef {
        FlowRef(Rc::new(self))
    }
}
