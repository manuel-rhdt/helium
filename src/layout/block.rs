use super::block_size::BlockSizeContext;
use super::flow::{Flow, FlowBase, FlowRef, FlowDescription, FlowType};
use super::fragment::Fragment;
use super::inline_size::InlineSizeContext;
use crate::default_cell_ext::DefaultCellExt;
use crate::display_list::DisplayList;
use crate::style::properties::computed::{ComputedValues, GenericSize};
use crate::tree::TreeLinked;

use std::cell::Cell;
use std::fmt;

pub struct BlockFlow {
    base: FlowBase,
    fragment: Cell<Option<Fragment>>,
}

impl fmt::Debug for BlockFlow {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("BlockFlow")
            .field("base", &self.base)
            .finish()
    }
}

impl Flow for BlockFlow {
    fn flow_base(&self) -> &FlowBase {
        &self.base
    }

    fn description(&self) -> FlowDescription {
        FlowDescription {
            flow_type: FlowType::Block
        }
    }

    fn assign_inline_sizes(&self, self_node: FlowRef, context: &InlineSizeContext) {
        let size =
            if let GenericSize::LengthPercentage(length_percentage) = self.style().position.width {
                // use specified width, if any
                length_percentage.length().map(|x| x.0)
            } else {
                None
            };

        // when no width was specified, use parent's width, falling back to the page width if there is no parent
        let size =
            size.unwrap_or_else(|| get_parent_inline_size(self_node).unwrap_or(context.device.width));

        self.base.inline_size.set(size);

        self.fragment.mutate(|fragment| {
            fragment.as_mut().map(|f| f.content_size.width = size);
        });
    }

    fn assign_block_size(&self, self_node: FlowRef, _context: &BlockSizeContext) {
        let mut block_size = 0.0;

        for child_flow in self_node.children() {
            child_flow.flow_base().set_block_offset(block_size);
            block_size += child_flow.flow_base().block_size.get();
        }

        self.flow_base().block_size.set(block_size);

        self.fragment.mutate(|fragment| {
            fragment.as_mut().map(|f| f.content_size.height = 30.0);
        });
    }

    fn build_display_list(&self, list: &mut DisplayList) {
        if let Some(fragment) = self.fragment.take() {
            list.append_fragment(fragment)
        }
    }
}

impl BlockFlow {
    pub fn new(fragment: Option<Fragment>, style: ComputedValues) -> Self {
        BlockFlow {
            base: FlowBase::new(style),
            fragment: Cell::new(fragment),
        }
    }
}

/// Returns the inline size of the containing box or `None` if called with the
/// root node.
fn get_parent_inline_size(node: FlowRef) -> Option<f32> {
    node.parent()
        .map(|parent| parent.flow_base().inline_size.get())
}
