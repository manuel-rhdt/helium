use crate::style::Style;
use crate::tree::{Links, TreeLinked};
use super::inline_size::InlineSizeContext;
use super::block_size::BlockSizeContext;

use std::fmt::Debug;
use std::rc::{Rc, Weak};
use std::ops::Deref;
use std::cell::Cell;

#[derive(Debug, Clone)]
pub struct FlowRef(pub(super) Rc<dyn Flow>);

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

pub trait Flow: Debug {
    fn flow_base(&self) -> &FlowBase;

    fn parent_flow(&self) -> Option<FlowRef> {
        self.flow_base().links.parent().as_ref().and_then(FlowRef::from_weak)
    }

    /// Calculates and assigns the inline size of the flow.
    /// 
    /// It is assumed that the paren't inline size was already calculated.
    fn assign_inline_sizes(&self, _context: &InlineSizeContext) {
        unimplemented!("assign_inline_sizes")
    }

    fn assign_block_size(&self, _context: &BlockSizeContext) {
        unimplemented!("assign_block_sizes")
    }
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

// macro_rules! render_tree {
//     (<node>{$content:expr}</node>) => {
//         RenderNode::with_content(Some(FlowType::Text(($content).into()))).into_ref()
//     };
//     (<node>$(<node>{$children:tt}</node>)+</node>) => {
//         {
//             let node = RenderNode::with_content(Some(FlowType::Element(vec![]))).into_ref();
//             $(
//                 let child = render_tree!(<node>{$children}</node>);
//                 node.append_child(child);
//             )+
//             node
//         }
//     };
// }

#[cfg(test)]
mod test {
    // #[test]
    // fn test_insert_parent() {
    //     let render_tree = render_tree!(<node>{ "Hello" }</node>);
    // }

    // #[test]
    // fn test_render_tree_macro() {
    //     let render_tree = render_tree!(<node>{ "Hello" }</node>);
    //     assert!(render_tree.is_well_formed());
    //     let render_tree =
    //         render_tree!(<node><node>{ "Hello" }</node><node>{ "World!" }</node></node>);
    //     assert!(render_tree.is_well_formed());
    // }
}
