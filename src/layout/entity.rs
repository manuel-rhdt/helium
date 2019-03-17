use specs::{Entity, Component, DenseVecStorage, World};
use derive_more::From;

use crate::style::properties::computed::{CssFloat, ComputedValues};

pub type LayoutContext = World;

#[derive(Debug, Copy, Clone, From)]
pub struct LayoutEntity(pub Entity);

#[derive(Debug)]
pub struct ContainingBlock {
    pub entity: LayoutEntity,
}

impl Component for ContainingBlock {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Position {
    pub x: CssFloat,
    pub y: CssFloat,
    pub width: CssFloat,
    pub height: CssFloat
}

impl Component for Position {
    type Storage = DenseVecStorage<Self>;
}

impl Component for ComputedValues {
    type Storage = DenseVecStorage<Self>;
}