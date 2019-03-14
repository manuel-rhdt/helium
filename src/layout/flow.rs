use super::block_size::BlockSizeContext;
use super::inline_size::InlineSizeContext;
use crate::display_list::{DisplayList, Point};
use crate::dot::{Connection, DotGraphSerializer, GraphDisplayStyle, GraphStyle, Style, Shape};
use crate::style::properties::computed::ComputedValues;
use crate::tree::{GetOpaqueNode, Link, Links, OpaqueNode, TreeLinked};

use log::warn;

use std::cell::Cell;
use std::fmt::Debug;
use std::ops::Deref;
use std::rc::{Rc, Weak};

#[derive(Debug, Clone)]
pub struct FlowRef(Rc<dyn Flow>);

impl PartialEq for FlowRef {
    fn eq(&self, other: &FlowRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for FlowRef {}

impl Deref for FlowRef {
    type Target = dyn Flow;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FlowType {
    Block,
    Inline
}

#[derive(Debug, Clone)]
pub struct FlowDescription {
    pub flow_type: FlowType,
}

/// This trait defines a common interface that all Flow types obey.
pub trait Flow: Debug {
    /// Returns a reference to the contained `FlowBase`.
    ///
    /// Every flow must contain a `FlowBase`.
    fn flow_base(&self) -> &FlowBase;

    /// Returns a description of the flow.
    fn description(&self) -> FlowDescription;

    /// Returns a reference to the computed style properties for this flow.
    ///
    /// The computed style of a flow gets assigned during construction and is
    /// immutable thereafter.
    fn style(&self) -> &ComputedValues {
        self.flow_base().style()
    }

    /// Creates a `FlowRef` from a flow.
    ///
    /// This function is useful during the construction of the flow tree.
    fn into_ref(self) -> FlowRef
    where
        Self: 'static + Sized,
    {
        FlowRef(Rc::new(self))
    }

    /// Calculates and assigns the inline size of the flow.
    ///
    /// It is assumed that the paren't inline size was already calculated.
    fn assign_inline_sizes(&self, _self_node: FlowRef, _context: &InlineSizeContext) {
        warn!("assign_inline_sizes is not implemented for {:?}", self)
    }

    /// Calculates and assigns the block size of the flow (usually the height).
    ///
    /// This is called during the bottom-up layout traversal and the flow can
    /// assume that all its children already have calculated their block sizes.
    fn assign_block_size(&self, _self_node: FlowRef, _context: &BlockSizeContext) {
        warn!("assign_block_size is not implemented for {:?}", self)
    }

    /// Called before children are drawn.
    fn build_display_list(&self, _list: &mut DisplayList) {
        warn!("build_display_list is not implemented for {:?}", self)
    }
}

impl GraphStyle for FlowRef {
    fn graph_style(&self) -> GraphDisplayStyle {
        match self.description().flow_type {
            FlowType::Block => GraphDisplayStyle::new().label("BlockFlow").shape(Shape::Box).style(Style::Filled(1.0, 0.5, 0.5)),
            FlowType::Inline => GraphDisplayStyle::new().label("InlineFlow").style(Style::Filled(1.0, 0.5, 0.5))   
        }
    }
}

impl GetOpaqueNode for FlowRef {
    fn get_opaque(&self) -> OpaqueNode {
        let node_data: *const dyn Flow = &**self;
        OpaqueNode(node_data as *const () as usize)
    }
}

impl FlowRef {
    pub fn serialize_children(
        &self,
        serializer: &mut DotGraphSerializer<impl std::io::Write>,
    ) -> std::io::Result<()> {
        for child in self.children() {
            serializer.push_node(child.clone(), Connection::DescendantOf(self.get_opaque()))?;
            child.serialize_children(serializer)?;
        }
        Ok(())
    }
}

/// The "base class" of all flows.
///
/// This holds information common to all flow types.
#[derive(Debug, Clone)]
pub struct FlowBase {
    pub(super) links: Links<Rc<dyn Flow>, Weak<dyn Flow>>,

    style: ComputedValues,

    /// The inline size of a flow (aka width in ltr layout).
    ///
    /// This quantity is computed during the top-down layout traversal. During
    /// this traversal children generally compute their inline metrics based on
    /// the parent's inline size.
    pub inline_size: Cell<f32>,

    pub block_size: Cell<f32>,

    /// The offset from the parent's top left corner.
    pub offset: Cell<Point>,
}

impl FlowBase {
    /// Create a new `FlowBase` with a computed style.
    pub fn new(style: ComputedValues) -> Self {
        FlowBase {
            links: Default::default(),
            style,
            inline_size: Default::default(),
            block_size: Default::default(),
            offset: Default::default(),
        }
    }

    pub fn style(&self) -> &ComputedValues {
        &self.style
    }

    pub fn set_block_offset(&self, offset: f32) {
        let old = self.offset.get();
        self.offset.set(Point::new(old.x, offset))
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
}

impl Link for Rc<dyn Flow> {
    type Target = FlowRef;

    fn get(&self) -> Self::Target {
        FlowRef(self.clone())
    }
}

impl Link for Weak<dyn Flow> {
    type Target = Option<FlowRef>;

    fn get(&self) -> Self::Target {
        self.upgrade().map(FlowRef)
    }
}

#[cfg(test)]
mod test {}
