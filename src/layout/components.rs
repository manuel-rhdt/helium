use derive_more::From;
use specs::{Component, DenseVecStorage, Entity, NullStorage, World};

use super::text::font::GlyphIndex;
use crate::style::properties::computed::{ComputedValues, PixelLength};
use lemon_pdf::font::FontUnit;
use lemon_pdf_derive::PdfFormat;

pub type LayoutContext = World;

#[derive(Debug, Copy, Clone, From)]
pub struct LayoutEntity(pub Entity);

#[derive(Debug, Copy, Clone, From, Default)]
pub struct RootNode(pub Option<Entity>);

#[derive(Debug)]
pub struct ContainingBlock {
    pub entity: Entity,
}

impl Component for ContainingBlock {
    type Storage = DenseVecStorage<Self>;
}

/// Describes the position of a box, where (x, y) specifies the top-left corner
/// and width and height specify the dimensions of the content rect.
///
/// The origin of the coordinate system is usually the page's top left
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

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum IntrinsicSize {
    Width(PixelLength),
    Height(PixelLength),
    Size {
        width: PixelLength,
        height: PixelLength,
    },
    SizeWithBaseline {
        width: PixelLength,
        height: PixelLength,
        baseline: PixelLength,
    },
}

impl Default for IntrinsicSize {
    fn default() -> Self {
        IntrinsicSize::Size {
            width: PixelLength(0.0),
            height: PixelLength(0.0),
        }
    }
}

impl IntrinsicSize {
    pub fn width(self) -> Option<PixelLength> {
        match self {
            IntrinsicSize::Width(width)
            | IntrinsicSize::Size { width, .. }
            | IntrinsicSize::SizeWithBaseline { width, .. } => Some(width),
            _ => None,
        }
    }

    pub fn height(self) -> Option<PixelLength> {
        match self {
            IntrinsicSize::Height(height)
            | IntrinsicSize::Size { height, .. }
            | IntrinsicSize::SizeWithBaseline { height, .. } => Some(height),
            _ => None,
        }
    }
}

impl Component for IntrinsicSize {
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

#[derive(Debug, Copy, Clone)]
pub struct TransformMatrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: PixelLength,
    pub f: PixelLength,
}

impl TransformMatrix {
    pub fn identity() -> Self {
        TransformMatrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: PixelLength(0.0),
            f: PixelLength(0.0),
        }
    }

    pub fn translation(e: PixelLength, f: PixelLength) -> Self {
        TransformMatrix {
            e,
            f,
            ..TransformMatrix::identity()
        }
    }

    pub fn uniform_scale(scale: f32) -> Self {
        TransformMatrix {
            a: scale,
            b: 0.0,
            c: 0.0,
            d: scale,
            ..TransformMatrix::identity()
        }
    }

    pub fn apply(self, other: TransformMatrix) -> TransformMatrix {
        TransformMatrix {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn inv_apply(self, other: TransformMatrix) -> TransformMatrix {
        other.apply(self)
    }
}

#[derive(Debug, Clone)]
pub enum TextOperator {
    SetFont(String, PixelLength),
    Glyphs(Glyphs),
    KernedGlyphs(Vec<KernedGlyphs>),
    SetWordSpacing(PixelLength),
    SetTransform(TransformMatrix),
    NewLine {
        x_offset: PixelLength,
        leading: PixelLength,
    },
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    pub start_pos: (PixelLength, PixelLength),
    pub ops: Vec<TextOperator>,
}

impl Paragraph {
    pub fn new(start_x: PixelLength, start_y: PixelLength) -> Self {
        Paragraph {
            start_pos: (start_x, start_y),
            ops: vec![],
        }
    }
}

impl Component for Paragraph {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockLevelElement {
    ContainerStart(Entity),
    /// Marks the position in the flow where an absolutely positioned box would have been had it
    /// been `position: static` (its "static position"). The box itself is not part of the flow.
    Absolute(Entity),
    Leaf(Entity),
    ContainerEnd(Entity),
}

impl BlockLevelElement {
    /// Returns the entity associated with the `BlockLevelElement`.
    pub fn entity(self) -> Entity {
        use BlockLevelElement::*;
        match self {
            ContainerStart(entity) | Absolute(entity) | Leaf(entity) | ContainerEnd(entity) => entity,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TextList {
    pub list: Vec<BlockLevelElement>,
}

#[derive(Debug, Clone)]
pub struct MathExpression {
    pub expression: math_render::MathExpression,
    pub font_name: String,
}

impl Component for MathExpression {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug)]
pub struct MathBox {
    pub math_box: math_render::math_box::MathBox,
    pub font_name: String,
    pub font_size: PixelLength,
    pub font_scale: PixelLength,
}

impl Component for MathBox {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Default)]
pub struct VectorGraphics {
    pub ops: Vec<GraphicsOp>,
}

impl Component for VectorGraphics {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug)]
pub enum GraphicsOp {
    Line {
        from_x: PixelLength,
        from_y: PixelLength,
        to_x: PixelLength,
        to_y: PixelLength,
        thickness: PixelLength,
    },
}

/// The entities of one absolutely positioned box and its in-flow descendants.
///
/// Absolutely positioned boxes are laid out separately from the main flow, so they are not part
/// of the [`TextList`] the paginator works on. Instead they are placed on the page of their
/// containing block.
#[derive(Debug, Clone, Default)]
pub struct AbsoluteGroup {
    /// The entity whose page the group is placed on: the positioned ancestor establishing the
    /// containing block or, if the box is positioned relative to the initial containing block
    /// by its static position, the content in front of it in the flow. `None` for the first
    /// page.
    pub containing_block: Option<Entity>,
    /// All entities that got a position while laying out the group, each of them once.
    pub entities: Vec<Entity>,
}

/// All groups of absolutely positioned boxes. A group always comes after the group of its
/// containing block.
#[derive(Debug, Default)]
pub struct AbsoluteGroups {
    pub groups: Vec<AbsoluteGroup>,
}

/// The size and margins of the pages the layout was done for.
#[derive(Debug, Copy, Clone, Default)]
pub struct PageGeometry {
    pub width: PixelLength,
    pub height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}
