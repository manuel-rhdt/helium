pub mod block;
pub mod construct;
pub mod flow;
pub mod fragment;
pub mod inline;
pub mod inline_size;
pub mod block_size;

/*
#[derive(Debug, Copy, Clone)]
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

#[derive(Debug, Clone, Default)]
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

pub fn layout(mut tree: FlowRef) -> DisplayNode<FlowRef> {
    // create a parent page
    let mut page = Flow::with_content(
        None,
        StyleBuilder::new()
            .size(PageSize::Size {
                width: Px(600.0),
                height: Px(1200.0),
            })
            .build(),
    );
    let page = page.into_ref();
    tree.insert_parent(page.clone());

    // layout page
    let mut sink = DisplayNode::new(DisplayNodeInner {
        frame: Default::default(),
        render_node: page.clone(),
    });
    layout_node(page, &mut sink);

    sink

    // TODO
    // whenever page is full create a new page (currently simply put blocks onto new pages if they don't fit)
}

pub trait LayoutNode: Sized + Clone {
    type ChildrenIter: Iterator<Item = Self>;

    fn width(&self) -> LengthSpec;
    fn parent(&self) -> Option<Self>;

    fn children(&self) -> Self::ChildrenIter;
}

impl LayoutNode for FlowRef {
    type ChildrenIter = rctree::Children<Flow>;

    fn width(&self) -> LengthSpec {
        self.borrow().style.width
    }

    fn parent(&self) -> Option<Self> {
        self.parent()
    }

    fn children(&self) -> Self::ChildrenIter {
        self.children()
    }
}

pub trait LayoutSink<T>: Sized {
    fn append_child(&mut self, frame: Frame, rnode: T) -> Self;

    fn parent(&self) -> Option<Self>;

    fn frame(&self) -> Frame;
}

impl<T> LayoutSink<T> for DisplayNode<T> {
    fn append_child(&mut self, frame: Frame, rnode: T) -> Self {
        let inner = DisplayNodeInner::new(frame, rnode);
        let node = DisplayNode::new(inner);
        self.append(node.clone());
        node
    }

    fn parent(&self) -> Option<Self> {
        self.parent()
    }

    fn frame(&self) -> Frame {
        self.borrow().frame
    }
}

/// implements 10.3.3
///
/// * `node` is the node in the render tree, whose layout should be calculated
/// * `sink` is a cursor to the current position in the display tree
fn layout_node<T: LayoutNode>(node: T, sink: &mut impl LayoutSink<T>) {
    let width = if let LengthSpec::Length(len) = node.width() {
        // if computed width is not `auto`, use it!
        len.to_px()
    } else {
        // use parent's width
        sink.frame().content_width
    };

    let frame = Frame {
        content_width: width,
        ..Default::default()
    };

    let mut child_sink = sink.append_child(frame, node.clone());

    for child in node.children() {
        layout_node(child, &mut child_sink);
    }
}
*/
