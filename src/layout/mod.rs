use rctree::Node;

use crate::style::{Length::*, LengthSpec, PageSize, StyleBuilder};
use crate::RenderNodeRef;

pub mod flow;

#[derive(Debug, Clone)]
pub struct Frame {
    /// Is set to false if this node should not be considered during rendering.
    visible: bool,

    /// Absolute horizontal position
    origin_left: f32,
    /// Absolute vertical position
    origin_top: f32,

    /// Width of the context box.
    content_width: f32,
    /// Height of the context box.
    content_height: f32,
    /// Horizontal offset of the context box relative to the block container.
    content_left: f32,
    /// Vertical offset of the context box relative to the block container.
    content_top: f32,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            visible: true,
            origin_left: 0.0,
            origin_top: 0.0,
            content_width: 0.0,
            content_height: 0.0,
            content_left: 0.0,
            content_top: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DisplayNodeInner<RenderNode> {
    frame: Frame,
    render_node: RenderNode,
}

impl<T> DisplayNodeInner<T> {
    fn new(frame: Frame, rnode: T) -> Self {
        DisplayNodeInner {
            frame,
            render_node: rnode,
        }
    }
}

pub type DisplayNode<T> = Node<DisplayNodeInner<T>>;

pub fn layout(tree: RenderNodeRef) -> DisplayNode<RenderNodeRef> {
    // create a parent page
    let page = tree.insert_parent();
    page.make_page();
    page.set_style(|_| {
        StyleBuilder::new()
            .size(PageSize::Size {
                width: Px(600.0),
                height: Px(1200.0),
            })
            .build()
    });

    // layout page
    layout_node(page)

    // whenever page is full create a new page (currently simply put blocks onto new pages if they don't fit)
}

pub trait LayoutNode: Sized {
    fn width(&self) -> LengthSpec;
    fn parent(&self) -> Option<Self>;
}

impl LayoutNode for RenderNodeRef {
    fn width(&self) -> LengthSpec {
        self.style.borrow().width
    }

    fn parent(&self) -> Option<Self> {
        self.parent_ref()
    }
}

pub trait LayoutSink: Sized {
    fn append_child(&self, frame: Frame);

    fn parent(&self) -> Option<Self>;

    fn frame(&self) -> Frame;
}

impl LayoutSink for DisplayNode<RenderNodeRef> {}

// implements 10.3.3
fn layout_node<T: LayoutNode>(node: T, sink: impl LayoutSink) {
    let width = if let LengthSpec::Length(len) = node.width() {
        len
    } else {
        let width_spec = node.parent().expect("Block layout requires parent").width();
        if let LengthSpec::Length(len) = width_spec {
            len
        } else {
            Px(0.0)
        }
    };

    let frame = Frame {
        content_width: width.to_px(),
        ..Default::default()
    };

    let node = DisplayNode::new(DisplayNodeInner::new(frame, node));

    node
}
