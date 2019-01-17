use std::rc::Rc;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct RenderTree(Rc<RenderTreeNode>);

impl RenderTree {
    pub fn new() -> RenderTree {
        RenderTree(Default::default())
    }

    pub fn node(&self) -> &RenderTreeNode {
        &self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderTreeNode {
    pub content: RefCell<Option<Content>>,
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
    Children(Vec<RenderTree>),
}
