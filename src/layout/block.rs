use super::flow::{Flow, FlowBase, FlowRef};
use super::fragment::Fragment;
use super::inline_size::InlineSizeContext;
use super::block_size::BlockSizeContext;
use crate::style::Style;
use crate::default_cell_ext::DefaultCellExt;
use crate::display_list::{DisplayList, DisplayItem, Point};

use std::rc::Rc;
use std::cell::Cell;
use std::fmt;

pub struct BlockFlow {
    base: FlowBase,
    fragment: Cell<Option<Fragment>>,
}

impl fmt::Debug for BlockFlow {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("BlockFlow").field("base", &self.base).finish()
    }
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

        self.fragment.mutate(|fragment| { fragment.as_mut().map(|f| f.content_size.width = parent_size); });
    }

    fn assign_block_size(&self, _context: &BlockSizeContext) {
        self.fragment.mutate(|fragment| { fragment.as_mut().map(|f| f.content_size.height = 30.0); });
    }

    fn build_display_list(&self, _list: &mut DisplayList) {

    }
}

impl BlockFlow {
    pub fn new(fragment: Option<Fragment>, style: Style) -> Self {
        BlockFlow {
            base: FlowBase::new(style),
            fragment: Cell::new(fragment),
        }
    }
}
