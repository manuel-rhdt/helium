use super::flow::{Flow, FlowBase, FlowRef};
use super::fragment::Fragment;
use crate::style::properties::computed::ComputedValues;

use std::rc::Rc;

#[derive(Debug)]
pub struct InlineFlow {
    base: FlowBase,
    fragments: Vec<Fragment>,
    lines: Vec<()>,
}

impl Flow for InlineFlow {
    fn flow_base(&self) -> &FlowBase {
        &self.base
    }
}

impl InlineFlow {
    pub fn with_fragments(fragments: Vec<Fragment>, style: ComputedValues) -> Self {
        InlineFlow {
            base: FlowBase::new(style),
            fragments,
            lines: Default::default(),
        }
    }
}
