use specs::{Entity, Component, DenseVecStorage, VecStorage, World};
use derive_more::From;

use crate::style::properties::computed::{PixelLength, ComputedValues};

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
    pub x: PixelLength,
    pub y: PixelLength,
    pub width: PixelLength,
    pub height: PixelLength
}

impl Component for Position {
    type Storage = DenseVecStorage<Self>;
}

impl Component for ComputedValues {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub characters: String
}

impl Component for Text {
    type Storage = VecStorage<Self>;
}