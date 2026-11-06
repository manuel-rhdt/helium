use derive_more::From;
use specs::{Component, DenseVecStorage, Entity, NullStorage, World};
use std::collections::HashMap;

use super::text::font::GlyphIndex;
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength};
use lemon_pdf::font::FontUnit;
use lemon_pdf_derive::PdfFormat;

pub type LayoutContext = World;

#[derive(Debug, Copy, Clone, From)]
pub struct LayoutEntity(pub Entity);

#[derive(Debug, Copy, Clone, From, Default)]
pub struct RootNode(pub Option<Entity>);

/// Maps a layout entity (by id) to the href of the hyperlink that contains it, if any.
///
/// The map is built once after DOM traversal: every node with a layout entity that lies inside
/// an `<a href="...">` element is assigned the href of that element. It is used while creating
/// the line boxes to find the clickable area of every hyperlink.
#[derive(Debug, Clone, Default)]
pub struct EntityHrefs(pub std::collections::HashMap<u32, String>);

impl EntityHrefs {
    pub fn href_of(&self, entity: Entity) -> Option<&str> {
        self.0.get(&entity.id()).map(String::as_str)
    }
}

/// The destination of an internal (cross-reference) link: a page and a point on that page.
#[derive(Debug, Clone, PartialEq)]
pub struct GoToDestination {
    /// The zero-based index of the destination page.
    pub page: usize,
    /// The horizontal position of the destination, in the page's coordinate system.
    pub x: PixelLength,
    /// The vertical position of the destination, in the page's coordinate system (top-left origin).
    pub y: PixelLength,
}

/// The clickable region of a hyperlink on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Hyperlink {
    /// The page the link is on.
    pub page: usize,
    /// The rectangle of the link in the page's coordinate system (top-left origin).
    pub rect: crate::display_list::Rect,
    /// The URI the link points to. For cross-references within the document this is the fragment
    /// (without the leading `#`), and the resolved target is in [`Hyperlink::destination`].
    pub href: String,
    /// Where the link jumps to inside the document, if it is a cross-reference (`<a href="#...">`).
    pub destination: Option<GoToDestination>,
}

/// All hyperlinks found in the document, collected while the line boxes are created.
#[derive(Debug, Clone, Default)]
pub struct Hyperlinks(pub Vec<Hyperlink>);

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

/// The number of columns and rows a table cell spans (the `colspan` and `rowspan` attributes).
/// Cells without the component span one column and one row.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct TableCellSpan {
    pub columns: u16,
    /// 0 spans the rest of the row group.
    pub rows: u16,
}

impl Default for TableCellSpan {
    fn default() -> Self {
        TableCellSpan {
            columns: 1,
            rows: 1,
        }
    }
}

impl Component for TableCellSpan {
    type Storage = DenseVecStorage<Self>;
}

/// The `<col>` and `<colgroup>` elements of a table, in column order.
///
/// Columns do not generate boxes, so they are not part of the box tree; their widths are
/// recorded here when the table is built and used to size the table's columns.
#[derive(Debug, Clone, Default)]
pub struct TableColumns {
    pub columns: Vec<TableColumn>,
}

/// One `<col>`, or a `<colgroup>` without `<col>` children: the columns it covers and its width.
#[derive(Debug, Copy, Clone)]
pub struct TableColumn {
    /// The number of columns the element covers (its `span` attribute).
    pub span: u16,
    /// The `width` of the element, which is a minimum width for the columns it covers.
    pub width: Option<GenericSize>,
}

impl Component for TableColumns {
    type Storage = DenseVecStorage<Self>;
}

/// The marker of a list item, drawn outside of the item's first line.
#[derive(Debug, Clone)]
pub struct ListMarker {
    pub text: String,
    pub font_name: String,
    pub font_size: PixelLength,
    pub color: crate::style::properties::computed::RGBA,
    pub fallback: crate::layout::text::fallback::TextFonts,
}

impl Component for ListMarker {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, Default)]
pub struct TextRun {
    pub text: String,
    pub font_name: String,
    /// Size of an EM box.
    pub font_size: PixelLength,
    pub line_height: PixelLength,
    pub baseline: PixelLength,
    pub color: crate::style::properties::computed::RGBA,
    pub decoration: crate::style::properties::specified::TextDecoration,
    pub hyphens: crate::style::properties::specified::Hyphens,
    pub manual_hyphen_points: Vec<usize>,
    /// The language of the text, for shaping, fallback fonts and hyphenation.
    pub lang: crate::style::properties::computed::Language,
    /// The font description that decides the fallback fonts of the text.
    pub fallback: crate::layout::text::fallback::TextFonts,
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

/// Marks a box that is split across pages. Its decoration is painted by the
/// [`DecorationFragment`]s instead, one for every page the box is on.
#[derive(Debug, Default, Copy, Clone)]
pub struct Fragmented;

impl Component for Fragmented {
    type Storage = NullStorage<Self>;
}

/// The part of the decoration of a [`Fragmented`] box that is on one page.
#[derive(Debug, Default, Copy, Clone)]
pub struct DecorationFragment;

impl Component for DecorationFragment {
    type Storage = NullStorage<Self>;
}

/// Contains information needed to draw the background and outline of an element.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Decoration {
    pub padding: EdgeValues<PixelLength>,
    pub border: EdgeValues<PixelLength>,
    pub border_color: EdgeValues<crate::style::properties::computed::RGBA>,
    pub margin: EdgeValues<PixelLength>,
    pub background_color: crate::style::properties::computed::RGBA,
    /// The borders of a cell of a table with `border-collapse: collapse`, which are painted
    /// instead of `border` (see [`CollapsedEdge`]). The table itself and its rows have them too,
    /// with nothing to paint, because the cells paint all borders.
    pub collapsed: Option<EdgeValues<CollapsedEdge>>,
}

/// A border between two cells (or a cell and the edge of the table) in the collapsing border
/// model. Its `width` is centered on the edge of the border box of the cell, so that half of it
/// is in the neighbouring cell. Each border is painted by one of the cells next to it only.
/// https://www.w3.org/TR/CSS22/tables.html#collapsing-borders
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct CollapsedEdge {
    pub width: PixelLength,
    pub color: crate::style::properties::computed::RGBA,
    /// Whether this box paints the border.
    pub paint: bool,
    /// How far the line extends beyond the corners of the border box at its start (top or
    /// left) and end, to cover the joints with the borders across it.
    pub extend_start: PixelLength,
    pub extend_end: PixelLength,
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
            collapsed: None,
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
        use std::io::Write;

        if !self.0.is_empty() {
            // A literal string is about half the size of a hex string. Only the
            // bytes that are special inside a literal string need an escape.
            let mut bytes = Vec::with_capacity(self.0.len() * 2 + 2);
            bytes.push(b'(');
            for GlyphIndex(glyph) in &self.0 {
                for byte in (*glyph as u16).to_be_bytes() {
                    match byte {
                        b'(' | b')' | b'\\' => bytes.extend_from_slice(&[b'\\', byte]),
                        b'\r' => bytes.extend_from_slice(b"\\r"),
                        _ => bytes.push(byte),
                    }
                }
            }
            bytes.push(b')');
            f.write_all(&bytes)?;
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
    SetColor(crate::style::properties::computed::RGBA),
    Glyphs(Glyphs),
    KernedGlyphs(Vec<KernedGlyphs>),
    SetWordSpacing(PixelLength),
    /// Raises the following glyphs above the baseline (PDF text rise, `Ts`). Shaping uses it
    /// for glyphs that the font positions vertically, like marks and cursive attachments.
    SetRise(PixelLength),
    SetTransform(TransformMatrix),
    /// Starts marked content whose `/ActualText` is the given text: extractors use it instead of
    /// the `ToUnicode` text of the glyphs that follow, up to `EndActualText`. Used for clusters
    /// that a per-glyph `ToUnicode` map cannot describe.
    BeginActualText(String),
    EndActualText,
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
            ContainerStart(entity) | Absolute(entity) | Leaf(entity) | ContainerEnd(entity) => {
                entity
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TextList {
    pub list: Vec<BlockLevelElement>,
}

/// A node of a MathML formula (an element inside, or including, `<math>`).
///
/// The math layout system reads these as a [`math_render::host::MathTree`].
#[derive(Debug, Clone)]
pub struct MathNode {
    pub kind: math_render::host::NodeKind,
    /// The rendered child elements.
    pub children: Vec<Entity>,
    /// The text of a token element (`mi`, `mn`, `mo`, `mtext`, `ms`) after text transforms.
    pub text: Option<String>,
    /// The attributes of the element, by local name.
    pub attributes: HashMap<String, String>,
    /// The font of this node (a key of `FontMap`).
    pub font_name: String,
}

impl Component for MathNode {
    type Storage = DenseVecStorage<Self>;
}

/// The laid out formula, stored on the `<math>` element.
#[derive(Debug)]
pub struct MathBox {
    /// Every node of the formula (including the root) in tree order, with the offset of its
    /// border box from the border box of the `<math>` element.
    pub nodes: Vec<(
        Entity,
        PixelLength,
        PixelLength,
        math_render::host::NodeLayout,
    )>,
}

impl Component for MathBox {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Default)]
pub struct VectorGraphics {
    pub ops: Vec<GraphicsOp>,
    /// The stroke color, or `None` to keep whatever color is current.
    pub color: Option<crate::style::properties::computed::RGBA>,
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
    /// The `inline-block` box this group is the contents of. Such a group is not placed with the
    /// paginator: its contents are laid out relative to the margin box of the box and moved
    /// to the final place once the line the box is on has been laid out.
    pub owner: Option<Entity>,
    /// The group that was open when this group was created.
    pub parent: Option<usize>,
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
