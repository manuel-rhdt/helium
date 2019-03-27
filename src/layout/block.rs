use specs::{Component, Entity, VecStorage};

#[derive(Debug, Clone)]
pub struct BlockContainer {
    pub elements: Vec<Entity>,
}

impl Component for BlockContainer {
    type Storage = VecStorage<Self>;
}
