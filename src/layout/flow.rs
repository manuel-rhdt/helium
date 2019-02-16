use crate::style::Style;
use crate::tree::{Links, TreeLinked};
use std::fmt::Debug;
use std::rc::{Rc, Weak};

#[derive(Debug, Clone)]
pub struct FlowRef(pub(super) Rc<dyn Flow + 'static>);

impl PartialEq for FlowRef {
    fn eq(&self, other: &FlowRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for FlowRef {}

pub trait Flow: Debug {
    fn flow_base(&self) -> &FlowBase;
}

#[derive(Debug, Clone)]
pub struct FlowBase {
    links: Links<Rc<dyn Flow>, Weak<dyn Flow>>,

    pub style: Style,
}

impl FlowBase {
    pub fn new(style: Style) -> Self {
        FlowBase {
            links: Default::default(),
            style,
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
