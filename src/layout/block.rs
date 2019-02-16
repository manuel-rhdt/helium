use super::flow::{Flow, FlowBase, FlowRef};
use super::fragment::Fragment;
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
