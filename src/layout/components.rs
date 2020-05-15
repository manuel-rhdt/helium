use derive_more::From;
use specs::{Component, DenseVecStorage, Entity, NullStorage, World};

use super::text::font::GlyphIndex;
use crate::style::properties::computed::{ComputedValues, PixelLength};
use lemon_pdf::font::FontUnit;
use lemon_pdf_derive::PdfFormat;

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

// #[derive(Debug, Clone, Default)]
// pub struct Text {
//     pub glyphs: Vec<GlyphIndex>,
//     pub font: super::text::Font,
//     /// Size of an EM box.
//     pub font_size: PixelLength,
//     /// Distance from the top content edge of the box to the baseline of the text.
//     pub baseline: PixelLength,
// }

// impl Component for Text {
//     type Storage = DenseVecStorage<Self>;
// }

#[derive(Debug, Clone, Default)]
pub struct TextRun {
    pub text: String,
    pub font_name: String,
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

impl<T> EdgeValues<T> {
    pub fn all(fill: T) -> Self
    where
        T: Clone,
    {
        EdgeValues {
            top: fill.clone(),
            left: fill.clone(),
            bottom: fill.clone(),
            right: fill,
        }
    }
}

/// Contains information needed to draw the background and outline of an element.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Decoration {
    pub padding: EdgeValues<PixelLength>,
    pub border: EdgeValues<PixelLength>,
    pub border_color: EdgeValues<crate::style::properties::computed::RGBA>,
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
            border_color: EdgeValues::all(RGBA::transparent()),
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

#[derive(Debug, Clone)]
pub struct Glyphs(pub Vec<GlyphIndex>);

impl lemon_pdf::PdfFormat for Glyphs {
    fn write(&self, f: &mut lemon_pdf::object::Formatter) -> std::io::Result<()> {
        use lemon_pdf::object::WriteEscaped;
        use std::io::Write;

        if !self.0.is_empty() {
            write!(f, "<")?;
            for GlyphIndex(glyph) in &self.0 {
                f.write_hex_escaped(&(*glyph as u16).to_be_bytes())?;
            }
            write!(f, ">")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PdfFormat)]
pub enum KernedGlyphs {
    Glyphs(Glyphs),
    Kern(FontUnit),
}

#[derive(Debug, Clone)]
pub enum TextOperator {
    SetFont(String, PixelLength),
    Glyphs(Glyphs),
    KernedGlyphs(Vec<KernedGlyphs>),
    SetWordSpacing(PixelLength),
    NewLine {
        x_offset: PixelLength,
        leading: PixelLength,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Paragraph {
    pub ops: Vec<TextOperator>,
}

impl Component for Paragraph {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockLevelElement {
    ContainerStart(Entity),
    Leaf(Entity),
    ContainerEnd(Entity),
}

impl BlockLevelElement {
    /// Returns the entity associated with the `BlockLevelElement`.
    pub fn entity(self) -> Entity {
        use BlockLevelElement::*;
        match self {
            ContainerStart(entity) | Leaf(entity) | ContainerEnd(entity) => entity,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TextList {
    pub list: Vec<BlockLevelElement>,
}
