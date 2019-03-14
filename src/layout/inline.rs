use super::flow::{Flow, FlowBase, FlowDescription, FlowType};
use super::fragment::Fragment;
use crate::style::properties::computed::ComputedValues;

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

    fn description(&self) -> FlowDescription {
        FlowDescription {
            flow_type: FlowType::Inline
        }
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
