use crate::style::Style;
use crate::tree::{Links, TreeLinked};
use crate::display_list::DisplayList;
use super::inline_size::InlineSizeContext;
use super::block_size::BlockSizeContext;

use log::warn;

use std::fmt::Debug;
use std::rc::{Rc, Weak};
use std::ops::Deref;
use std::cell::Cell;

#[derive(Clone)]
pub struct FlowRef(pub(super) Rc<dyn Flow>);

impl Debug for FlowRef {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        f.debug_struct("FlowRef")
            .field("flow", &self.0)
            .field("children", &self.children().collect::<Vec<_>>())
            .finish()
    }
}

impl PartialEq for FlowRef {
    fn eq(&self, other: &FlowRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for FlowRef {
    type Target = dyn Flow;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl Eq for FlowRef {}

/// This trait defines a common interface that all Flow types obey.
pub trait Flow: Debug {
    /// Returns a reference to the contained `FlowBase`.
    /// 
    /// Every flow must contain a `FlowBase`.
    fn flow_base(&self) -> &FlowBase;

    /// Creates a `FlowRef` from a flow.
    /// 
    /// This function is useful during the construction of the flow tree.
    fn into_ref(self) -> FlowRef where Self: 'static + Sized {
        FlowRef(Rc::new(self))
    }

    /// A convenience method to get the parent flow.
    fn parent_flow(&self) -> Option<FlowRef> {
        self.flow_base().links.parent().as_ref().and_then(FlowRef::from_weak)
    }

    /// Calculates and assigns the inline size of the flow.
    /// 
    /// It is assumed that the paren't inline size was already calculated.
    fn assign_inline_sizes(&self, _context: &InlineSizeContext) {
        warn!("assign_inline_sizes is not implemented for {:?}", self)
    }

    /// Calculates and assigns the block size of the flow (usually the height).
    ///
    /// This is called during the bottom-up layout traversal and the flow can
    /// assume that all its children already have calculated their block sizes.
    fn assign_block_size(&self, _context: &BlockSizeContext) {
        warn!("assign_block_size is not implemented for {:?}", self)
    }

    /// Called before children are drawn.
    fn build_display_list(&self, _list: &mut DisplayList) {}
}

#[derive(Debug, Clone)]
pub struct FlowBase {
    pub(super) links: Links<Rc<dyn Flow>, Weak<dyn Flow>>,

    pub style: Style,
    
    /// The inline size of a flow (aka width in ltr layout).
    /// 
    /// This quantity is computed during the top-down layout traversal. During
    /// this traversal children generally compute their inline metrics based on
    /// the parent's inline size.
    pub inline_size: Cell<f32>,
}

impl FlowBase {
    pub fn new(style: Style) -> Self {
        FlowBase {
            links: Default::default(),
            style,
            inline_size: 0.0.into(),
        }
    }
}

impl TreeLinked for FlowRef {
    type StrongLink = Rc<dyn Flow>;
    type WeakLink = Weak<dyn Flow>;

    fn links(&self) -> &Links<Self::StrongLink, Self::WeakLink> {
        &self.0.flow_base().links
    }

    fn strong_link(self) -> Self::StrongLink {
        self.0
    }

    fn weak_link(&self) -> Self::WeakLink {
        Rc::downgrade(&self.0)
    }

    fn from_strong(strong: &Self::StrongLink) -> Self {
        FlowRef(strong.clone())
    }

    fn from_weak(weak: &Self::WeakLink) -> Option<Self> {
        weak.upgrade().map(FlowRef)
    }
}

#[cfg(test)]
mod test {

}
