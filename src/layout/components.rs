use derive_more::From;
use specs::{Component, DenseVecStorage, Entity, NullStorage, World};

use super::text::font::GlyphIndex;
use crate::style::properties::computed::{ComputedValues, PixelLength};

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

/// Describes the position of a box, where (x, y) specifies the top-left corner
/// and width and height specify the dimensions of the content rect.
///
/// The origin of the coordinate system is usually the parent element's top left
/// corner.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Position {
    /// Horizontal coordinate of the top left corner.
    pub x: PixelLength,
    /// Vertical coordinate of the top left corner.
    pub y: PixelLength,
    /// Width of the box.
    pub width: PixelLength,
    /// Height of the box.
    pub height: PixelLength,
    /// Z-Index
    pub z: ZIndex,
    /// page number,
    pub page: usize,
}

impl Component for Position {
    type Storage = DenseVecStorage<Self>;
}

/// The position of a box relative to the smallest containing stacking context.
#[derive(Debug, Copy, Clone, Default)]
pub struct StackingContextRelativePosition {
    pub x: PixelLength,
    pub y: PixelLength,
}

impl Component for StackingContextRelativePosition {
    type Storage = DenseVecStorage<Self>;
}

impl Component for ComputedValues {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Text {
    pub glyphs: Vec<GlyphIndex>,
    pub font: super::text::Font,
    /// Size of an EM box.
    pub font_size: PixelLength,
    /// Distance from the top content edge of the box to the baseline of the text.
    pub baseline: PixelLength,
}

impl Component for Text {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextRun {
    pub text: String,
    pub font: super::text::Font,
    /// Size of an EM box.
    pub font_size: PixelLength,
    pub line_height: PixelLength,
    pub baseline: PixelLength,
}

impl Component for TextRun {
    type Storage = DenseVecStorage<Self>;
}

/// A component that indicates that an entity is considered a collapsible margin.
#[derive(Debug, Default, Copy, Clone)]
pub struct CanCollapse;

impl Component for CanCollapse {
    type Storage = NullStorage<Self>;
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct EdgeValues<T> {
    pub top: T,
    pub left: T,
    pub bottom: T,
    pub right: T,
}

/// Contains information needed to draw the background and outline of an element.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Decoration {
    pub padding: EdgeValues<PixelLength>,
    pub border: EdgeValues<PixelLength>,
    pub margin: EdgeValues<PixelLength>,
    pub background_color: crate::style::properties::computed::RGBA,
}

impl Default for Decoration {
    fn default() -> Self {
        use crate::style::properties::computed::RGBA;
        Decoration {
            padding: Default::default(),
            border: Default::default(),
            margin: Default::default(),
            background_color: RGBA::transparent(),
        }
    }
}

impl Component for Decoration {
    type Storage = DenseVecStorage<Self>;
}

/// Entities with bigger `ZIndex` are drawn on top of elements with lower `ZIndex`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ZIndex(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Breakpoint {
    offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineBreakPossibilities {
    pub breakpoints: Vec<Breakpoint>,
}

impl Component for LineBreakPossibilities {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum TextOperator {
    ChangeFont(super::text::Font, PixelLength),
    Glyphs(Vec<GlyphIndex>),
    NewLine { leading: PixelLength },
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Paragraph {
    pub ops: Vec<TextOperator>,
}

impl Component for Paragraph {
    type Storage = DenseVecStorage<Self>;
}
