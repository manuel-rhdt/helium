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
    /// The `Link` element of the structure tree that the link belongs to.
    pub node: Option<crate::tagging::StructNodeId>,
}

/// An entry of the document outline (the bookmarks of the PDF): an element with a `bookmark-level`
/// (by default the headings).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineEntry {
    /// The level of the entry (`bookmark-level`): 1 for `h1`, up to 6 for `h6`.
    pub level: u8,
    /// Whether the entries below this one are hidden when the PDF is opened (`bookmark-state`).
    pub closed: bool,
    /// The text of the heading.
    pub title: String,
    /// The start of the heading.
    pub destination: GoToDestination,
}

/// The entries of the outline in document order, filled once the positions are final.
#[derive(Debug, Clone, Default)]
pub struct Outline(pub Vec<OutlineEntry>);

/// The named destinations of the document: where the element with every `id` starts. A PDF viewer
/// that opens `document.pdf#id` jumps there.
#[derive(Debug, Clone, Default)]
pub struct NamedDestinations(pub std::collections::BTreeMap<String, GoToDestination>);

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

/// The offsets of `position: relative` (`left`, `right`, `top`, `bottom`) of an inline-level box.
///
/// Boxes in a block, flex or grid container are shifted by the layout itself. An inline box is
/// not a box of the layout, so the pieces of text and the atomic boxes inside it carry the
/// offsets of every relatively positioned inline box that they are in, and the line boxes shift
/// them. The offsets are added up.
#[derive(Debug, Clone, Default)]
pub struct RelativeOffset(pub Vec<Inset>);

impl Component for RelativeOffset {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Copy, Clone)]
pub struct Inset {
    pub left: crate::style::properties::computed::GenericSize,
    pub right: crate::style::properties::computed::GenericSize,
    pub top: crate::style::properties::computed::GenericSize,
    pub bottom: crate::style::properties::computed::GenericSize,
}

impl RelativeOffset {
    /// The physical move of a box on a line of a box with this `flow` whose size along the line
    /// is `line_size`. The size of a vertical box across the lines is not known here, so its
    /// percentages of `left` and `right` count as `auto`; the ones of `top` and `bottom` refer
    /// to the height, which is the size along the lines.
    pub fn resolve_in_line(
        &self,
        flow: crate::layout::writing_mode::Flow,
        line_size: PixelLength,
        rtl: bool,
    ) -> (PixelLength, PixelLength) {
        if flow.is_vertical() {
            self.resolve(None, Some(line_size), rtl)
        } else {
            self.resolve(Some(line_size), None, rtl)
        }
    }

    /// How far the box is moved to the right and down, for a containing block that is `width`
    /// wide and `height` high (if its height is not `auto`) and whose direction is right-to-left
    /// if `rtl`.
    ///
    /// A percentage of `top` or `bottom` refers to the height of the containing block; if that
    /// is `auto`, the value behaves like `auto` (CSS 2.2, 9.3.2). The same holds for the width,
    /// which is unknown for the lines of a vertical box.
    pub fn resolve(
        &self,
        width: Option<PixelLength>,
        height: Option<PixelLength>,
        rtl: bool,
    ) -> (PixelLength, PixelLength) {
        use crate::style::properties::computed::{GenericSize, LengthPercentage};
        let axis = |size: GenericSize, container: Option<PixelLength>| match (size, container) {
            (GenericSize::Auto, _) => None,
            (GenericSize::LengthPercentage(LengthPercentage::PixelLength(len)), _) => Some(len.0),
            (GenericSize::LengthPercentage(lp), Some(container)) => Some(lp.absolute(container).0),
            _ => None,
        };
        let (mut dx, mut dy) = (0.0, 0.0);
        for inset in &self.0 {
            // over-constrained: `left` wins in a left-to-right container, `right` in the other
            dx += match (axis(inset.left, width), axis(inset.right, width)) {
                (Some(left), Some(_)) if !rtl => left,
                (Some(_), Some(right)) => -right,
                (Some(left), None) => left,
                (None, Some(right)) => -right,
                (None, None) => 0.0,
            };
            // over-constrained: `top` wins
            dy += match (axis(inset.top, height), axis(inset.bottom, height)) {
                (Some(top), _) => top,
                (None, Some(bottom)) => -bottom,
                (None, None) => 0.0,
            };
        }
        (PixelLength(dx), PixelLength(dy))
    }
}

/// The size of a `<canvas>` in pixels, which is a replaced box with nothing drawn in it.
#[derive(Debug, Copy, Clone)]
pub struct CanvasSize {
    pub width: f32,
    pub height: f32,
    /// Whether the box has no ratio of its own (an `<iframe>`): a side that is `auto` is the
    /// natural size, not what the ratio gives.
    pub ratioless: bool,
}

impl Component for CanvasSize {
    type Storage = DenseVecStorage<Self>;
}

/// The `vertical-align` of an inline-level box, with what is needed to resolve it against the
/// parent box (CSS 2.2, 10.8.1).
#[derive(Debug, Copy, Clone)]
pub struct AlignStep {
    pub align: crate::style::properties::computed::VerticalAlign,
    /// The `line-height` of the box, which percentages refer to.
    pub line_height: PixelLength,
    /// The ascent and descent of the box of the element (from its baseline to the top and
    /// bottom of its line-height box). An `inline-block` box is as high as its margin box,
    /// which is only known in the layout: it has `None`.
    pub own: Option<(PixelLength, PixelLength)>,
    /// The font of the parent: the ascent and descent of its content area, its x-height and its
    /// size.
    pub parent_ascent: PixelLength,
    pub parent_descent: PixelLength,
    pub parent_x_height: PixelLength,
    pub parent_font_size: PixelLength,
}

/// The `vertical-align` of an inline box, which the content of the box takes along.
#[derive(Debug, Copy, Clone)]
pub struct ElementAlign(pub AlignStep);

impl Component for ElementAlign {
    type Storage = DenseVecStorage<Self>;
}

/// The `vertical-align` values of the inline boxes that a text run or an atomic inline-level box
/// is in, the innermost first. The boxes are aligned in their parents, so the offsets add up.
#[derive(Debug, Clone, Default)]
pub struct VerticalAlignment(pub Vec<AlignStep>);

impl Component for VerticalAlignment {
    type Storage = DenseVecStorage<Self>;
}

/// Where a box goes on its line, after its `vertical-align` is resolved.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LinePlacement {
    /// Raised above the baseline of the line by this much (lowered if negative).
    Raise(PixelLength),
    /// The box (of this ascent and descent) is aligned with the top (or the bottom) of the line
    /// box; what is in it is moved by `inner` in addition.
    Top { inner: PixelLength, ascent: PixelLength, descent: PixelLength },
    Bottom { inner: PixelLength, ascent: PixelLength, descent: PixelLength },
}

impl VerticalAlignment {
    /// Resolves the alignment of an entity whose own box is `own` (ascent and descent).
    pub fn place(&self, own: (PixelLength, PixelLength)) -> LinePlacement {
        use crate::style::properties::computed::VerticalAlign;
        // from the outermost box inwards: `raise` is the baseline of the box that is aligned
        let mut raise = 0.0f32;
        let mut line_aligned: Option<(bool, PixelLength, PixelLength)> = None;
        for step in self.0.iter().rev() {
            let (ascent, descent) = step.own.unwrap_or(own);
            match step.align {
                VerticalAlign::Baseline => {}
                VerticalAlign::Sub => raise -= step.parent_font_size.0 * 0.2,
                VerticalAlign::Super => raise += step.parent_font_size.0 / 3.0,
                VerticalAlign::LengthPercentage(lp) => raise += lp.absolute(step.line_height).0,
                // the middle of the box is at the baseline plus half the x-height of the parent
                VerticalAlign::Middle => {
                    raise += step.parent_x_height.0 / 2.0 - (ascent.0 - descent.0) / 2.0
                }
                VerticalAlign::TextTop => raise += step.parent_ascent.0 - ascent.0,
                VerticalAlign::TextBottom => raise += descent.0 - step.parent_descent.0,
                VerticalAlign::Top | VerticalAlign::Bottom => {
                    // the baseline of this box is found from the line box; what is inside moves
                    // relative to it
                    line_aligned = Some((step.align == VerticalAlign::Top, ascent, descent));
                    raise = 0.0;
                }
            }
        }
        match line_aligned {
            None => LinePlacement::Raise(PixelLength(raise)),
            Some((true, ascent, descent)) => {
                LinePlacement::Top { inner: PixelLength(raise), ascent, descent }
            }
            Some((false, ascent, descent)) => {
                LinePlacement::Bottom { inner: PixelLength(raise), ascent, descent }
            }
        }
    }
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

/// What a text is set in: the font, its size and color, and the fonts and the features that
/// shaping needs (letter-spacing, OpenType features, fallback fonts). Text that is not part of a
/// `TextRun` (the markers of list items, the margin boxes of pages) takes it from its style in
/// one place, `EntityConstructor::text_face`.
#[derive(Debug, Clone, Default)]
pub struct TextFace {
    pub font_name: String,
    pub font_size: PixelLength,
    pub color: crate::style::properties::computed::PaintColor,
    pub fallback: crate::layout::text::fallback::TextFonts,
}

/// The marker of a list item, drawn outside of the item's first line.
#[derive(Debug, Clone)]
pub struct ListMarker {
    pub text: String,
    pub face: TextFace,
    /// The text comes from the `content` of a `::marker`, which sets the whole marker: there is
    /// no space between it and the item.
    pub explicit: bool,
}

impl Component for ListMarker {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Clone, Default)]
pub struct TextRun {
    pub text: String,
    /// What the text is set in: the font, the size of an EM box, the color and the fonts that
    /// decide the fallback fonts and the features of the text.
    pub face: TextFace,
    pub line_height: PixelLength,
    pub baseline: PixelLength,
    pub decoration: crate::style::properties::specified::TextDecoration,
    /// The background color of the inline box that the text is in (the nearest ancestor that has
    /// one, up to the block): painted behind the text, over the em box of its font.
    pub background: Option<crate::style::properties::computed::PaintColor>,
    /// The border and the vertical padding of the inline box that the text is in.
    pub edges: Option<InlineBoxEdges>,
    pub hyphens: crate::style::properties::specified::Hyphens,
    pub manual_hyphen_points: Vec<usize>,
    /// The language of the text, for shaping, fallback fonts and hyphenation.
    pub lang: crate::style::properties::computed::Language,
}

/// The border and the padding of an inline box, as far as they are painted behind and around a
/// piece of its text: the borders at the top and the bottom go over every piece, the one at the
/// left or the right only over the run at that end of the box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineBoxEdges {
    /// The width and the color of the borders: top, right, bottom, left.
    pub border: [(f32, crate::style::properties::computed::PaintColor); 4],
    pub padding_top: f32,
    pub padding_bottom: f32,
    /// This run is the one for the padding and the border at the start (the end) of the box, in
    /// the direction of the text.
    pub start: bool,
    pub end: bool,
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

/// The part of the decoration of a [`Fragmented`] box that is on one page. It holds the box it
/// is a part of, whose style gives the images of the background.
#[derive(Debug, Copy, Clone)]
pub struct DecorationFragment(pub Entity);

impl Component for DecorationFragment {
    type Storage = DenseVecStorage<Self>;
}

/// Marks the box of a rule between two columns of a multi-column container: its
/// [`Decoration`] is the colour of the rule.
#[derive(Debug, Default, Copy, Clone)]
pub struct ColumnRule;

impl Component for ColumnRule {
    type Storage = NullStorage<Self>;
}

/// Contains information needed to draw the background and outline of an element.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Decoration {
    pub padding: EdgeValues<PixelLength>,
    pub border: EdgeValues<PixelLength>,
    pub border_color: EdgeValues<crate::style::properties::computed::PaintColor>,
    pub border_style: EdgeValues<crate::style::properties::specified::BorderStyle>,
    pub margin: EdgeValues<PixelLength>,
    pub background_color: crate::style::properties::computed::PaintColor,
    /// The borders of a cell of a table with `border-collapse: collapse`, which are painted
    /// instead of `border` (see [`CollapsedEdge`]). The table itself and its rows have them too,
    /// with nothing to paint, because the cells paint all borders.
    pub collapsed: Option<EdgeValues<CollapsedEdge>>,
    /// The radii of the corners of the border edge (`border-radius`).
    pub radius: CornerRadii,
    /// Whether the box has a background image, which the style of the box describes.
    pub has_background_image: bool,
}

/// The radius of each corner of a box, as computed. Percentages are resolved against the width
/// (horizontal radius) and height (vertical radius) of the border box when painting.
/// https://www.w3.org/TR/css-backgrounds-3/#border-radius
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct CornerRadii {
    pub top_left: crate::style::properties::computed::LengthPercentage,
    pub top_right: crate::style::properties::computed::LengthPercentage,
    pub bottom_right: crate::style::properties::computed::LengthPercentage,
    pub bottom_left: crate::style::properties::computed::LengthPercentage,
}

impl CornerRadii {
    /// Whether any corner has a radius that could be non-zero.
    pub fn is_rounded(&self) -> bool {
        use crate::style::properties::computed::LengthPercentage as Lp;
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
            .iter()
            .any(|lp| match lp {
                Lp::PixelLength(px) => px.0 > 0.0,
                _ => true,
            })
    }
}

/// A border between two cells (or a cell and the edge of the table) in the collapsing border
/// model. Its `width` is centered on the edge of the border box of the cell, so that half of it
/// is in the neighbouring cell. Each border is painted by one of the cells next to it only.
/// https://www.w3.org/TR/CSS22/tables.html#collapsing-borders
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct CollapsedEdge {
    pub width: PixelLength,
    pub color: crate::style::properties::computed::PaintColor,
    pub style: crate::style::properties::specified::BorderStyle,
    /// Whether this box paints the border.
    pub paint: bool,
    /// How far the line extends beyond the corners of the border box at its start (top or
    /// left) and end, to cover the joints with the borders across it.
    pub extend_start: PixelLength,
    pub extend_end: PixelLength,
}

impl Default for Decoration {
    fn default() -> Self {
        use crate::style::properties::computed::PaintColor;
        Decoration {
            padding: Default::default(),
            border: Default::default(),
            margin: Default::default(),
            border_color: EdgeValues::all(PaintColor::transparent()),
            border_style: EdgeValues::all(Default::default()),
            background_color: PaintColor::transparent(),
            collapsed: None,
            radius: Default::default(),
            has_background_image: false,
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

/// A glyph of a `TJ` operator with the kern that goes in front of it, in tenths of a font unit
/// (a thousandth of an em); zero for no kern. Kerns below a tenth are not worth writing.
#[derive(Debug, Copy, Clone)]
pub struct KernedGlyph {
    pub glyph: GlyphIndex,
    pub kern: i32,
    /// The byte offset in the text of its [`TextPiece`] of the first character that the glyph
    /// was shaped from, or [`NO_CLUSTER`]. Glyphs of the same cluster (a conjunct, a ligature)
    /// have the same offset.
    pub cluster: u32,
}

/// The `cluster` of a glyph that stands for no text (the parts of a stretched operator).
pub const NO_CLUSTER: u32 = u32::MAX;

/// A run of the glyphs of a [`TextOps`] with the text that they stand for. Backends work out
/// from it what text the glyphs of the page stand for (the `ToUnicode` map and the `/ActualText`
/// of PDF, the characters of a word in HTML): the display list says what text every glyph was
/// drawn for, and every backend decides how to carry that.
///
/// The glyphs of a piece are in the order in which they are drawn, from left to right. In
/// right-to-left text (`rtl`) that is the reverse of the order of the text.
#[derive(Debug, Clone)]
pub struct TextPiece {
    /// The glyphs of the piece: a range of [`TextOps::glyphs`].
    pub glyphs: std::ops::Range<u32>,
    /// The text of the glyphs: a range of [`TextOps::text`].
    pub text: std::ops::Range<u32>,
    pub rtl: bool,
    pub kind: PieceKind,
}

/// What the text of a [`TextPiece`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceKind {
    /// The characters that the glyphs were shaped from. The invisible formatting characters
    /// between two clusters have no glyphs and are not part of the text of either.
    Shaped,
    /// The text of a formula, which is kept exactly: its invisible operators say how the
    /// formula is read.
    Formula,
    /// Glyphs that the layout added to the text, like the hyphen at a line break. The text is
    /// what they stand for in the text (a soft hyphen), not the character that they are the
    /// glyphs of, so it says nothing about what the same glyph stands for elsewhere.
    Inserted,
}

/// The glyphs of a `TJ` operator: a range of [`TextOps::glyphs`], and the kern after the last one.
#[derive(Debug, Copy, Clone)]
pub struct GlyphRange {
    pub start: u32,
    pub end: u32,
    pub trailing_kern: i32,
}

/// An element of the array of a `TJ` operator.
#[derive(Debug, Copy, Clone)]
pub enum KernedItem<'a> {
    /// A string of glyphs that are not kerned.
    Glyphs(&'a [KernedGlyph]),
    Kern(FontUnit),
}

impl GlyphRange {
    /// Calls `f` for the elements of the array of the `TJ` operator, in order: a string of glyphs
    /// ends where the next glyph has a kern.
    pub fn for_each_item<'a>(self, glyphs: &'a [KernedGlyph], mut f: impl FnMut(KernedItem<'a>)) {
        let run = &glyphs[self.start as usize..self.end as usize];
        let mut from = 0;
        for (index, glyph) in run.iter().enumerate() {
            if glyph.kern != 0 {
                if index > from {
                    f(KernedItem::Glyphs(&run[from..index]));
                }
                f(KernedItem::Kern(FontUnit(glyph.kern as f64 / 10.0)));
                from = index;
            }
        }
        if run.len() > from {
            f(KernedItem::Glyphs(&run[from..]));
        }
        if self.trailing_kern != 0 {
            f(KernedItem::Kern(FontUnit(self.trailing_kern as f64 / 10.0)));
        }
    }

}

/// The operators of a text and the glyphs of its `TJ` operators, which are ranges of one `Vec`:
/// a `Vec` for every run of glyphs between two kerns was most of the memory of a long text.
/// Dereferences to the operators.
///
/// The `pieces` say what text the glyphs were shaped from, which is kept in `text`: it is the
/// part of the display list that tells a backend which characters it is showing. They are in the
/// order of the glyphs, which they do not have to cover completely.
#[derive(Debug, Default, Clone)]
pub struct TextOps {
    ops: Vec<TextOperator>,
    pub glyphs: Vec<KernedGlyph>,
    pub text: String,
    pub pieces: Vec<TextPiece>,
}

impl std::ops::Deref for TextOps {
    type Target = Vec<TextOperator>;
    fn deref(&self) -> &Vec<TextOperator> {
        &self.ops
    }
}

impl std::ops::DerefMut for TextOps {
    fn deref_mut(&mut self) -> &mut Vec<TextOperator> {
        &mut self.ops
    }
}

impl TextOps {
    /// Text operators that show no glyphs yet.
    pub fn from_ops(ops: Vec<TextOperator>) -> Self {
        TextOps { ops, ..TextOps::default() }
    }

    /// Takes the operators out and leaves the glyphs, the text and the pieces that they refer
    /// to, so that the operators can be rewritten without moving those.
    pub fn take_ops(&mut self) -> Vec<TextOperator> {
        std::mem::take(&mut self.ops)
    }

    /// Appends a `TJ` operator that shows one glyph.
    pub fn push_glyph(&mut self, glyph: GlyphIndex, cluster: u32) {
        let start = self.glyphs.len() as u32;
        self.glyphs.push(KernedGlyph { glyph, kern: 0, cluster });
        self.ops.push(TextOperator::KernedGlyphs(GlyphRange { start, end: start + 1, trailing_kern: 0 }));
    }

    /// Records that the glyphs `glyphs` (indices of [`TextOps::glyphs`]) stand for `characters`.
    /// The `cluster` of each of them is a byte offset in `characters`.
    pub fn push_piece(&mut self, glyphs: std::ops::Range<usize>, characters: &str, rtl: bool, kind: PieceKind) {
        if glyphs.is_empty() {
            return;
        }
        let start = self.text.len() as u32;
        self.text.push_str(characters);
        self.pieces.push(TextPiece {
            glyphs: glyphs.start as u32..glyphs.end as u32,
            text: start..self.text.len() as u32,
            rtl,
            kind,
        });
    }

    /// The text of a piece.
    pub fn piece_text(&self, piece: &TextPiece) -> &str {
        &self.text[piece.text.start as usize..piece.text.end as usize]
    }

    /// The name of the font that each glyph is drawn with, indexed like [`TextOps::glyphs`]. A
    /// glyph that no operator shows has no font (an empty name).
    pub fn glyph_fonts(&self) -> Vec<&str> {
        let mut fonts = vec![""; self.glyphs.len()];
        let mut font = "";
        for op in &self.ops {
            match op {
                TextOperator::SetFont(name, _) => font = name,
                TextOperator::KernedGlyphs(range) => fonts[range.start as usize..range.end as usize].fill(font),
                _ => {}
            }
        }
        fonts
    }

    /// Appends the operators of `other`, its glyphs and its text.
    pub fn append(&mut self, other: TextOps) {
        let glyph_base = self.glyphs.len() as u32;
        let text_base = self.text.len() as u32;
        self.glyphs.extend(other.glyphs);
        self.text.push_str(&other.text);
        self.pieces.extend(other.pieces.into_iter().map(|piece| TextPiece {
            glyphs: piece.glyphs.start + glyph_base..piece.glyphs.end + glyph_base,
            text: piece.text.start + text_base..piece.text.end + text_base,
            rtl: piece.rtl,
            kind: piece.kind,
        }));
        self.ops.extend(other.ops.into_iter().map(|mut op| {
            if let TextOperator::KernedGlyphs(range) = &mut op {
                range.start += glyph_base;
                range.end += glyph_base;
            }
            op
        }));
    }
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
    SetColor(crate::style::properties::computed::PaintColor),
    /// A `TJ` operator: glyphs, with kerns between some of them (see [`TextOps`]).
    KernedGlyphs(GlyphRange),
    SetWordSpacing(PixelLength),
    /// Raises the following glyphs above the baseline (PDF text rise, `Ts`). Shaping uses it
    /// for glyphs that the font positions vertically, like marks and cursive attachments.
    SetRise(PixelLength),
    SetTransform(TransformMatrix),
    /// Starts marked content that stands for the given text, up to the matching
    /// `EndMarkedContent`. The PDF writer adds it where the text of glyphs cannot be given by the
    /// `ToUnicode` map of a font (see `pdf::text::with_actual_text`); the layout does not: it
    /// says what its glyphs stand for with the pieces of [`TextOps`].
    BeginActualText(String),
    /// Starts marked content that is not part of the text of the document (a page number):
    /// extractors that honour `/Artifact` leave it out.
    BeginArtifact,
    /// Starts marked content that belongs to a node of the structure tree (tagged PDF), up to
    /// the matching `EndMarkedContent`.
    BeginTag(crate::tagging::ContentTag),
    /// Ends the marked content that `BeginActualText`, `BeginArtifact` or `BeginTag` started.
    EndMarkedContent,
    NewLine {
        x_offset: PixelLength,
        leading: PixelLength,
    },
    /// A glyph that is painted from the `COLR` table of its font (a colour emoji) instead of being
    /// drawn as text: `dx` is where the pen is, from the start of the piece, and `rise` the height
    /// above the baseline. The text only advances over the glyph; `LineBoxCreator` takes these
    /// operators out and places a [`ColorGlyph`] at the absolute position of the glyph.
    ColorGlyph {
        font: String,
        glyph: crate::layout::text::font::GlyphIndex,
        size: PixelLength,
        dx: PixelLength,
        rise: PixelLength,
    },
    /// The place of an atomic inline (a formula) whose own [`Paragraph`] is painted here, so
    /// that the content stream follows the logical order of the text. The embedded operators
    /// position their glyphs absolutely; afterwards the text line matrix is set back to `(x, y)`
    /// (absolute) and the font and color are set again by the operators that follow.
    Embed {
        entity: specs::Entity,
        x: PixelLength,
        y: PixelLength,
    },
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    pub start_pos: (PixelLength, PixelLength),
    pub ops: TextOps,
    /// The colour glyphs of the text, painted over it.
    pub color_glyphs: Vec<PlacedColorGlyph>,
}

impl Paragraph {
    pub fn new(start_x: PixelLength, start_y: PixelLength) -> Self {
        Paragraph {
            start_pos: (start_x, start_y),
            ops: TextOps::default(),
            color_glyphs: vec![],
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
/// The math layout system reads these as a [`formel::host::MathTree`].
#[derive(Debug, Clone)]
pub struct MathNode {
    pub kind: formel::host::NodeKind,
    /// The rendered child elements.
    pub children: Vec<Entity>,
    /// The text of a token element (`mi`, `mn`, `mo`, `mtext`, `ms`) after text transforms.
    pub text: Option<String>,
    /// The attributes of the element, by local name.
    pub attributes: HashMap<String, String>,
    /// The content of a token element that contains foreign content (an image in `<mtext>`):
    /// its text and the foreign nodes, in document order. `children` does not list the foreign
    /// nodes of such a token.
    pub segments: Option<Vec<MathSegment>>,
    /// The font of this node (a key of `FontMap`).
    pub font_name: String,
}

/// A piece of the content of a token element with foreign content.
#[derive(Debug, Clone)]
pub enum MathSegment {
    Text(String),
    Foreign(Entity),
}

impl MathNode {
    /// The child elements and the foreign nodes of the content of a token.
    pub fn child_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        let foreign = self.segments.iter().flatten().filter_map(|segment| match segment {
            MathSegment::Foreign(entity) => Some(*entity),
            MathSegment::Text(_) => None,
        });
        self.children.iter().copied().chain(foreign)
    }
}

impl Component for MathNode {
    type Storage = DenseVecStorage<Self>;
}

/// The image of an `<img>` element: a replaced box that is as big as the image, unless its
/// style says otherwise.
#[derive(Debug, Clone)]
pub struct ImageBox {
    pub image: std::sync::Arc<crate::image::LoadedImage>,
    /// The intrinsic size in pixels: the size of the image, or the size that the `width` and
    /// `height` attributes ask for.
    pub width: f32,
    pub height: f32,
    /// The image has a ratio (the one of `width` to `height`) but no size of its own: an `<svg>`
    /// element with a `viewBox` and no `width` and `height`. Its width is then the one that is
    /// available, and zero when the box is sized by its content.
    pub ratio_only: bool,
}

impl Component for ImageBox {
    type Storage = DenseVecStorage<Self>;
}

/// The root element and the body: the background of one of them is the background of the
/// canvas, which is the background of every page.
#[derive(Debug, Default, Copy, Clone)]
pub struct CanvasEntities {
    pub html: Option<Entity>,
    pub body: Option<Entity>,
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
        formel::host::NodeLayout,
    )>,
}

impl Component for MathBox {
    type Storage = DenseVecStorage<Self>;
}

/// A colour glyph (an emoji of a font with a `COLR` table) at the absolute position of its
/// origin on the baseline. It is painted over the text of its [`Paragraph`], where the glyph is
/// empty and only advances the pen.
#[derive(Debug, Clone)]
pub struct PlacedColorGlyph {
    pub font: String,
    pub glyph: crate::layout::text::font::GlyphIndex,
    pub size: PixelLength,
    pub x: PixelLength,
    pub baseline: PixelLength,
    /// The colour of the text, for the layers of the glyph that use it.
    pub color: crate::style::properties::computed::PaintColor,
}

/// Takes the [`TextOperator::ColorGlyph`] operators that were appended to `ops` from index `from`
/// (by `shape_piece`, for one piece of text) and returns them placed: `origin` is where the pen
/// was at the start of the piece, on the baseline. Every producer of a [`Paragraph`] calls this
/// for the text it shapes, and puts the result in [`Paragraph::color_glyphs`].
pub fn take_color_glyphs(
    ops: &mut TextOps,
    from: usize,
    origin: (PixelLength, PixelLength),
    color: crate::style::properties::computed::PaintColor,
) -> Vec<PlacedColorGlyph> {
    let mut placed = Vec::new();
    let mut index = from;
    while index < ops.len() {
        if !matches!(ops[index], TextOperator::ColorGlyph { .. }) {
            index += 1;
            continue;
        }
        let TextOperator::ColorGlyph { font, glyph, size, dx, rise } = ops.remove(index) else {
            unreachable!()
        };
        placed.push(PlacedColorGlyph {
            font,
            glyph,
            size,
            x: origin.0 + dx,
            baseline: origin.1 - rise,
            color,
        });
    }
    placed
}

#[derive(Debug, Default)]
pub struct VectorGraphics {
    pub ops: Vec<GraphicsOp>,
    /// The stroke color, or `None` to keep whatever color is current.
    pub color: Option<crate::style::properties::computed::PaintColor>,
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
    /// The box that the group is the content of, which is a leaf of the main flow (an orthogonal
    /// flow), and where its content box was when the group was written. The group moves to
    /// the final place of the box when it is known.
    pub anchor: Option<(Entity, taffy::geometry::Point<f32>)>,
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

/// The boxes that have a `transform` or an `opacity` below 1, and which box every entity is
/// inside of.
///
/// A transform or an opacity applies to the box and to everything inside of it. The display
/// list is flat, so the items of such a box are found by their entity: `scope` is the nearest
/// box with a transform or an opacity that an entity is in (or is itself).
#[derive(Debug, Clone, Default)]
pub struct TransformScopes {
    pub boxes: std::collections::HashMap<Entity, TransformBox>,
    pub scope: std::collections::HashMap<Entity, Entity>,
}

/// A box with a `transform` or an `opacity` below 1.
#[derive(Debug, Clone)]
pub struct TransformBox {
    /// The box with a transform or an opacity that this box is inside of.
    pub parent: Option<Entity>,
    /// The `opacity` of the box: its content is painted as a group that is made translucent as a
    /// whole when this is below 1.
    pub opacity: f32,
    pub transform: crate::style::properties::transform::Transform,
    pub origin: crate::style::properties::transform::TransformOrigin,
    /// For the lengths in `em` and `rem`.
    pub font_size: crate::style::properties::computed::FontSizes,
}
