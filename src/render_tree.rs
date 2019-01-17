use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct RenderTree(Rc<RenderTreeNode>);

impl RenderTree {
    pub fn new() -> RenderTree {
        RenderTree(Default::default())
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderTreeNode {
    frame: Frame,
    content: Option<Content>,
}

#[derive(Debug, Clone, Default)]
pub struct Frame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone)]
pub enum Content {
    Text(String),
    SubTree(RenderTree),
}
