use crate::layout::components::ImageBox;
use crate::layout::components::{
    Decoration, DecorationFragment, EdgeValues, GraphicsOp, Hyperlink, Hyperlinks, Paragraph, Position, TextOperator,
    TransformMatrix, VectorGraphics, ZIndex,
};
use crate::layout::normal_formatting::Pages;
use crate::style::properties::computed::{ComputedValues, PixelLength, RGBA};
use crate::style::properties::background::{image_tiles, resolve_layer};
use crate::style::properties::specified::BackgroundBox;
use crate::style::properties::gradient::{BoxSize, Image, Position as BackgroundPosition, Resolved};
use crate::style::properties::specified::{BlendMode, BorderStyle};

use specs::prelude::*;
use std::ops::Deref;

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Point {
    pub x: PixelLength,
    pub y: PixelLength,
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Rect {
    pub x: PixelLength,
    pub y: PixelLength,
    pub width: PixelLength,
    pub height: PixelLength,
}

impl Rect {
    pub fn xmax(self) -> PixelLength {
        self.x + self.width
    }

    pub fn ymax(self) -> PixelLength {
        self.y + self.height
    }

    /// Create a rect exactly large enough to encompass `self` and `other`.
    pub fn union(self, other: Rect) -> Rect {
        let xmin = if self.x <= other.x { self.x } else { other.x };
        let ymin = if self.y <= other.y { self.y } else { other.y };
        let xmax = if self.xmax() >= other.xmax() {
            self.xmax()
        } else {
            other.xmax()
        };
        let ymax = if self.ymax() >= other.ymax() {
            self.ymax()
        } else {
            other.ymax()
        };
        Rect {
            x: xmin,
            y: ymin,
            width: xmax - xmin,
            height: ymax - ymin,
        }
    }
}

impl Point {
    pub fn new(x: PixelLength, y: PixelLength) -> Self {
        Point { x, y }
    }
}

/// One element of a path, in layout coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PathSegment {
    MoveTo(Point),
    LineTo(Point),
    CurveTo(Point, Point, Point),
    Close,
}

/// Fraction of the radius at which the control points of a Bézier curve approximating a quarter
/// ellipse are placed.
const KAPPA: f32 = 0.552_284_75;

/// A closed path that follows `rect` with elliptical corners of radius `(rx, ry)` in the order
/// top-left, top-right, bottom-right, bottom-left. The radii must already fit into `rect`.
pub fn rounded_rect_path(rect: Rect, radii: [(PixelLength, PixelLength); 4]) -> Vec<PathSegment> {
    let p = |x: PixelLength, y: PixelLength| Point::new(x, y);
    let [tl, tr, br, bl] = radii;
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.xmax(), rect.ymax());
    let mut path = vec![
        PathSegment::MoveTo(p(x0 + tl.0, y0)),
        PathSegment::LineTo(p(x1 - tr.0, y0)),
    ];
    // each corner is a curve from the end of one side to the start of the next
    path.push(PathSegment::CurveTo(
        p(x1 - tr.0 + tr.0 * KAPPA, y0),
        p(x1, y0 + tr.1 - tr.1 * KAPPA),
        p(x1, y0 + tr.1),
    ));
    path.push(PathSegment::LineTo(p(x1, y1 - br.1)));
    path.push(PathSegment::CurveTo(
        p(x1, y1 - br.1 + br.1 * KAPPA),
        p(x1 - br.0 + br.0 * KAPPA, y1),
        p(x1 - br.0, y1),
    ));
    path.push(PathSegment::LineTo(p(x0 + bl.0, y1)));
    path.push(PathSegment::CurveTo(
        p(x0 + bl.0 - bl.0 * KAPPA, y1),
        p(x0, y1 - bl.1 + bl.1 * KAPPA),
        p(x0, y1 - bl.1),
    ));
    path.push(PathSegment::LineTo(p(x0, y0 + tl.1)));
    path.push(PathSegment::CurveTo(
        p(x0, y0 + tl.1 - tl.1 * KAPPA),
        p(x0 + tl.0 - tl.0 * KAPPA, y0),
        p(x0 + tl.0, y0),
    ));
    path.push(PathSegment::Close);
    path
}


/// A side of a box, for the borders: where the outside is, and which tones of the colour the
/// styles `inset`, `outset`, `groove` and `ridge` use (they are lit from the top left).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    /// The direction away from the box, in page coordinates (y grows downwards).
    fn outward(self) -> (f32, f32) {
        match self {
            Side::Top => (0.0, -1.0),
            Side::Right => (1.0, 0.0),
            Side::Bottom => (0.0, 1.0),
            Side::Left => (-1.0, 0.0),
        }
    }

    fn is_top_left(self) -> bool {
        matches!(self, Side::Top | Side::Left)
    }
}

/// The two tones that the borders `inset`, `outset`, `groove` and `ridge` have: the light one
/// and the dark one. A colour that is light enough is the light tone and the dark one is derived
/// from it; for a dark colour it is the other way round, so that the tones still differ.
fn border_tones(color: RGBA) -> (RGBA, RGBA) {
    let value = color.red.max(color.green).max(color.blue) as f32 / 255.0;
    let scale = |channel: u8, factor: f32| (channel as f32 * factor).round().clamp(0.0, 255.0) as u8;
    let brighten = |channel: u8| (channel as f32 + (255.0 - channel as f32) * 0.4).round() as u8;
    if value >= 0.5 {
        // the dark tone takes a third of the value off, like the borders of the browsers do
        let factor = ((value - 0.33) / value).max(0.0);
        let dark = RGBA {
            red: scale(color.red, factor),
            green: scale(color.green, factor),
            blue: scale(color.blue, factor),
            ..color
        };
        (color, dark)
    } else {
        let light = RGBA {
            red: brighten(color.red),
            green: brighten(color.green),
            blue: brighten(color.blue),
            ..color
        };
        (light, color)
    }
}

/// The colour that a side of a border with `style` is painted in, if it has only one: `inset`
/// and `outset` borders are light on two sides and dark on the other two.
fn single_tone(style: BorderStyle, side: Side, color: RGBA) -> RGBA {
    let (light, dark) = border_tones(color);
    match style {
        BorderStyle::Inset => if side.is_top_left() { dark } else { light },
        BorderStyle::Outset => if side.is_top_left() { light } else { dark },
        _ => color,
    }
}

/// A rectangle as a path.
fn rect_subpath(x0: f32, y0: f32, x1: f32, y1: f32, path: &mut Vec<PathSegment>) {
    let p = |x: f32, y: f32| Point::new(PixelLength(x), PixelLength(y));
    path.push(PathSegment::MoveTo(p(x0, y0)));
    path.push(PathSegment::LineTo(p(x1, y0)));
    path.push(PathSegment::LineTo(p(x1, y1)));
    path.push(PathSegment::LineTo(p(x0, y1)));
    path.push(PathSegment::Close);
}

/// A circle as a path.
fn circle_subpath(cx: f32, cy: f32, radius: f32, path: &mut Vec<PathSegment>) {
    let p = |x: f32, y: f32| Point::new(PixelLength(x), PixelLength(y));
    let k = radius * KAPPA;
    path.push(PathSegment::MoveTo(p(cx + radius, cy)));
    path.push(PathSegment::CurveTo(p(cx + radius, cy + k), p(cx + k, cy + radius), p(cx, cy + radius)));
    path.push(PathSegment::CurveTo(p(cx - k, cy + radius), p(cx - radius, cy + k), p(cx - radius, cy)));
    path.push(PathSegment::CurveTo(p(cx - radius, cy - k), p(cx - k, cy - radius), p(cx, cy - radius)));
    path.push(PathSegment::CurveTo(p(cx + k, cy - radius), p(cx + radius, cy - k), p(cx + radius, cy)));
    path.push(PathSegment::Close);
}

/// The dashes of a border from `a` to `b`, which are on an axis, as a path of rectangles of the
/// given thickness, centered on the line. The dashes and the gaps between them are as long as
/// the length of the line allows, with a dash at both ends.
fn dashes_path(a: (f32, f32), b: (f32, f32), thickness: f32) -> Vec<PathSegment> {
    let length = (b.0 - a.0).abs() + (b.1 - a.1).abs();
    let dash = if thickness < 3.0 { 3.0 * thickness } else { 2.0 * thickness };
    let wanted_gap = dash;
    let count = (((length + wanted_gap) / (dash + wanted_gap)).round() as usize).max(1);
    let (dash, gap) = if count == 1 {
        (length, 0.0)
    } else {
        (dash, ((length - count as f32 * dash) / (count - 1) as f32).max(0.0))
    };
    let horizontal = a.1 == b.1;
    let (min_x, min_y) = (a.0.min(b.0), a.1.min(b.1));
    let half = thickness / 2.0;
    let mut path = vec![];
    for i in 0..count {
        let from = i as f32 * (dash + gap);
        let to = from + dash;
        if horizontal {
            rect_subpath(min_x + from, min_y - half, min_x + to, min_y + half, &mut path);
        } else {
            rect_subpath(min_x - half, min_y + from, min_x + half, min_y + to, &mut path);
        }
    }
    path
}

/// The dots of a border from `a` to `b`, which are on an axis: circles as wide as the border
/// that touch their neighbours' gaps of the same width.
fn dots_path(a: (f32, f32), b: (f32, f32), thickness: f32) -> Vec<PathSegment> {
    let length = (b.0 - a.0).abs() + (b.1 - a.1).abs();
    let count = ((length / (2.0 * thickness)).round() as usize).max(1);
    let pitch = length / count as f32;
    let horizontal = a.1 == b.1;
    let (min_x, min_y) = (a.0.min(b.0), a.1.min(b.1));
    let mut path = vec![];
    for i in 0..count {
        let along = (i as f32 + 0.5) * pitch;
        let (cx, cy) = if horizontal { (min_x + along, min_y) } else { (min_x, min_y + along) };
        circle_subpath(cx, cy, thickness / 2.0, &mut path);
    }
    path
}

/// Resolves the radii of the corners of a border box and scales them down so that they do not
/// overlap, as specified in https://www.w3.org/TR/css-backgrounds-3/#corner-overlap
fn resolve_radii(
    radius: &crate::layout::components::CornerRadii,
    width: PixelLength,
    height: PixelLength,
) -> [(PixelLength, PixelLength); 4] {
    use crate::style::properties::computed::LengthPercentage as Lp;
    let resolve = |lp: Lp, container: PixelLength| match lp {
        Lp::PixelLength(px) => px,
        Lp::Percentage(pc) => container * pc,
        Lp::Both { length, percentage } => length + container * percentage,
    };
    let resolve_corner = |lp: Lp| {
        let rx = resolve(lp, width);
        let ry = resolve(lp, height);
        (
            if rx.0 > 0.0 { rx } else { PixelLength(0.0) },
            if ry.0 > 0.0 { ry } else { PixelLength(0.0) },
        )
    };
    let mut r = [
        resolve_corner(radius.top_left),
        resolve_corner(radius.top_right),
        resolve_corner(radius.bottom_right),
        resolve_corner(radius.bottom_left),
    ];
    let mut factor = 1.0f32;
    let mut limit = |available: PixelLength, sum: PixelLength| {
        if sum.0 > 0.0 {
            factor = factor.min(available.0 / sum.0);
        }
    };
    limit(width, r[0].0 + r[1].0);
    limit(width, r[3].0 + r[2].0);
    limit(height, r[0].1 + r[3].1);
    limit(height, r[1].1 + r[2].1);
    if factor < 1.0 {
        let factor = factor.max(0.0);
        for (rx, ry) in &mut r {
            *rx = *rx * factor;
            *ry = *ry * factor;
        }
    }
    r
}

/// A closed path around `rect`.
/// The complementary error function (Abramowitz and Stegun 7.1.26, an error below 1.5e-7).
fn erfc(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let erf = 1.0 - poly * (-x * x).exp();
    if x >= 0.0 { 1.0 - erf } else { 1.0 + erf }
}

/// Radii that fit into a box of the size `width` by `height`: all are scaled down by the same
/// factor if two of them would meet on a side that is too short.
fn fit_radii(
    radii: [(PixelLength, PixelLength); 4],
    width: PixelLength,
    height: PixelLength,
) -> [(PixelLength, PixelLength); 4] {
    let [tl, tr, br, bl] = radii;
    let mut factor = 1.0_f32;
    for (sum, size) in [
        (tl.0.0 + tr.0.0, width.0),
        (bl.0.0 + br.0.0, width.0),
        (tl.1.0 + bl.1.0, height.0),
        (tr.1.0 + br.1.0, height.0),
    ] {
        if sum > size && sum > 0.0 {
            factor = factor.min(size / sum);
        }
    }
    radii.map(|(rx, ry)| (PixelLength(rx.0 * factor), PixelLength(ry.0 * factor)))
}

fn rect_path(rect: Rect) -> Vec<PathSegment> {
    let p = |x: PixelLength, y: PixelLength| Point::new(x, y);
    vec![
        PathSegment::MoveTo(p(rect.x, rect.y)),
        PathSegment::LineTo(p(rect.xmax(), rect.y)),
        PathSegment::LineTo(p(rect.xmax(), rect.ymax())),
        PathSegment::LineTo(p(rect.x, rect.ymax())),
        PathSegment::Close,
    ]
}

/// The gradients in the `background-image` of a style, in the order they are painted (the
/// first image is on top, so it is painted last). `area` is the padding box, which positions and
/// sizes the gradients, and `paint` the border box that they are painted into.
fn background_gradients(style: &ComputedValues, area: Rect, paint: Rect, content: Rect) -> Vec<Layer> {
    use crate::style::properties::specified::BackgroundBox;
    use crate::style::properties::color::color_to_rgba;
    let current = color_to_rgba(style.font.color);
    let size = |rect: Rect| BoxSize {
        x: rect.x.0,
        y: rect.y.0,
        width: rect.width.0,
        height: rect.height.0,
    };
    let clips = &style.background.background_clip;
    let (area_rect, paint_rect) = (area, paint);
    let (area, paint) = (size(area), size(paint));
    fn cycle<T: Clone>(values: &[T], index: usize, default: T) -> T {
        if values.is_empty() { default } else { values[index % values.len()].clone() }
    }
    let modes = &style.background.background_blend_mode;
    style
        .background
        .background_image
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(index, image)| match image {
            Image::Gradient(gradient) => {
                let background = &style.background;
                let clip = match cycle(clips, index, BackgroundBox::BorderBox) {
                    BackgroundBox::BorderBox => paint_rect,
                    BackgroundBox::PaddingBox => area_rect,
                    BackgroundBox::ContentBox => content,
                };
                let gradient = resolve_layer(
                    &gradient.with_current_color(current),
                    &cycle(&background.background_size, index, Default::default()),
                    &cycle(&background.background_position, index, BackgroundPosition {
                        x: Default::default(),
                        y: Default::default(),
                    }),
                    cycle(&background.background_repeat, index, Default::default()),
                    area,
                    size(clip),
                )?;
                let blend = if modes.is_empty() { BlendMode::Normal } else { modes[index % modes.len()] };
                Some(Layer::Gradient(GradientLayer { gradient, blend, clip }))
            }
            Image::Url(url) => {
                let image = url.load()?;
                let background = &style.background;
                let clip = match cycle(clips, index, BackgroundBox::BorderBox) {
                    BackgroundBox::BorderBox => paint_rect,
                    BackgroundBox::PaddingBox => area_rect,
                    BackgroundBox::ContentBox => content,
                };
                let tiles = image_tiles(
                    image.size(),
                    &cycle(&background.background_size, index, Default::default()),
                    &cycle(&background.background_position, index, BackgroundPosition {
                        x: Default::default(),
                        y: Default::default(),
                    }),
                    cycle(&background.background_repeat, index, Default::default()),
                    area,
                    size(clip),
                );
                if tiles.is_empty() {
                    return None;
                }
                let rect = |tile: BoxSize| Rect {
                    x: PixelLength(tile.x),
                    y: PixelLength(tile.y),
                    width: PixelLength(tile.width),
                    height: PixelLength(tile.height),
                };
                Some(Layer::Image { image: image.clone(), tiles: tiles.into_iter().map(rect).collect(), clip })
            }
            Image::None => None,
        })
        .collect()
}

/// A gradient of a background with the way it is blended with the layers below.
#[derive(Debug, Clone)]
pub struct GradientLayer {
    pub gradient: Resolved,
    pub blend: BlendMode,
    /// The box that the layer is clipped to (`background-clip`).
    pub clip: Rect,
}

/// A layer of a background.
#[derive(Debug, Clone)]
pub enum Layer {
    Gradient(GradientLayer),
    /// An image, drawn once in every tile.
    Image {
        image: std::sync::Arc<crate::image::LoadedImage>,
        tiles: Vec<Rect>,
        /// The box that the layer is clipped to (`background-clip`).
        clip: Rect,
    },
}

impl Layer {
    /// How the layer is blended with the ones below. An image is not blended.
    fn blend(&self) -> BlendMode {
        match self {
            Layer::Gradient(layer) => layer.blend,
            Layer::Image { .. } => BlendMode::Normal,
        }
    }
}

/// A background whose layers are blended. The layers only blend with each other, so they are
/// painted as one isolated group.
#[derive(Debug, Clone)]
pub struct BlendGroup {
    /// The (rounded) border box.
    pub clip: Vec<PathSegment>,
    pub bbox: Rect,
    pub color: RGBA,
    /// From bottom to top.
    pub layers: Vec<GradientLayer>,
}

#[derive(Debug, Clone)]
pub struct ColorGlyphItem {
    pub origin: Point,
    pub size: PixelLength,
    pub font: String,
    pub glyph: crate::layout::text::font::GlyphIndex,
    pub color: RGBA,
}

#[derive(Debug, Clone)]
pub enum DisplayItem {
    LineString {
        points: Vec<Point>,
        line_width: PixelLength,
    },
    FilledRect(Rect),
    /// Fills a closed path with the current fill colour.
    FillPath {
        path: Vec<PathSegment>,
        even_odd: bool,
    },
    /// Intersects the clip with the path, until the enclosing `Pop`.
    ClipPath {
        path: Vec<PathSegment>,
        even_odd: bool,
    },
    /// Paints a gradient over the current clip.
    Gradient(Box<Resolved>),
    /// Paints background layers that have a blend mode.
    BlendGroup(Box<BlendGroup>),
    /// Paints an image into a rectangle.
    Image {
        rect: Rect,
        image: std::sync::Arc<crate::image::LoadedImage>,
    },
    Color(RGBA),
    StrokeColor(RGBA),
    /// A colour glyph (an emoji) painted from the `COLR` table of its font, whose origin is on
    /// the baseline at the point.
    ColorGlyph(Box<ColorGlyphItem>),
    Paragraph(Paragraph),
    Translate(PixelLength, PixelLength),
    /// Applies a transformation (the CSS `transform`) to what follows, until the enclosing `Pop`.
    /// The matrix is in the coordinates of the page: pixels, y pointing down.
    Transform([f32; 6]),
    /// Paints the items up to the matching `EndGroup` as a group that is made translucent as a
    /// whole (the CSS `opacity`). `bbox` covers everything that the group paints.
    BeginGroup {
        opacity: f32,
        bbox: Rect,
    },
    EndGroup,
    Push,
    Pop,
}

#[derive(Debug, Clone)]
struct DisplayElement {
    z_index: ZIndex,
    // Retained for debugging which entity produced an item.
    #[allow(dead_code)]
    entity: Entity,
    item: DisplayItem,
}

/// The boxes with a transform that `entity` is inside of, the outermost first.
///
/// The entities that were made from the nodes of the document know them. The ones that the
/// layout made later (the paragraphs of the lines, markers, decorations) do not: they are in
/// the innermost box with a transform on the same page that contains their center.
fn transform_chain(
    entity: Entity,
    scopes: &crate::layout::components::TransformScopes,
    positions: &ReadStorage<Position>,
) -> Vec<Entity> {
    let mut current = scopes.scope.get(&entity).copied();
    if current.is_none() {
        if let Some(pos) = positions.get(entity) {
            let (x, y) = (pos.x.0 + pos.width.0 / 2.0, pos.y.0 + pos.height.0 / 2.0);
            let depth = |mut scope: Entity| {
                let mut depth = 0;
                while let Some(parent) = scopes.boxes.get(&scope).and_then(|b| b.parent) {
                    scope = parent;
                    depth += 1;
                }
                depth
            };
            current = scopes
                .boxes
                .keys()
                .copied()
                .filter(|&candidate| {
                    positions.get(candidate).is_some_and(|b| {
                        b.page == pos.page
                            // half-open: a paragraph has no height and sits at the top of its
                            // block, which is the bottom edge of the box above it
                            && x >= b.x.0
                            && x < b.x.0 + b.width.0
                            && y >= b.y.0
                            && y < b.y.0 + b.height.0
                    })
                })
                .max_by_key(|&candidate| depth(candidate));
        }
    }
    let mut chain = vec![];
    while let Some(scope) = current {
        chain.push(scope);
        current = scopes.boxes.get(&scope).and_then(|b| b.parent);
    }
    chain.reverse();
    chain
}

/// Groups the items that are inside of the same box with a transform (the `depth`-th of their
/// chains) between a transformation and a `Pop`, at the place of the first of them.
fn wrap_transforms(
    items: Vec<DisplayElement>,
    depth: usize,
    scopes: &crate::layout::components::TransformScopes,
    positions: &ReadStorage<Position>,
    page: Rect,
) -> Vec<DisplayElement> {
    enum Slot {
        Item(DisplayElement),
        Group(Entity),
    }
    let mut slots = vec![];
    let mut groups: std::collections::HashMap<Entity, Vec<DisplayElement>> = Default::default();
    for item in items {
        match transform_chain(item.entity, scopes, positions).get(depth) {
            None => slots.push(Slot::Item(item)),
            Some(&root) => {
                if !groups.contains_key(&root) {
                    slots.push(Slot::Group(root));
                }
                groups.entry(root).or_default().push(item);
            }
        }
    }
    let mut out = vec![];
    for slot in slots {
        match slot {
            Slot::Item(item) => out.push(item),
            Slot::Group(root) => {
                let inner = wrap_transforms(groups.remove(&root).unwrap_or_default(), depth + 1, scopes, positions, page);
                let (matrix, opacity) = scopes
                    .boxes
                    .get(&root)
                    .zip(positions.get(root))
                    .map(|(b, p)| {
                        let matrix =
                            b.transform.matrix(&b.origin, p.x.0, p.y.0, p.width.0, p.height.0, b.font_size);
                        (matrix, b.opacity)
                    })
                    .unwrap_or((None, 1.0));
                let translucent = opacity < 1.0;
                match inner.first().map(|first| first.z_index) {
                    Some(z) if matrix.is_some() || translucent => {
                        let element = |item| DisplayElement { z_index: z, entity: root, item };
                        // the group is outside of the transformation: it is a group of the
                        // page, which paints what is transformed
                        if translucent {
                            out.push(element(DisplayItem::BeginGroup { opacity, bbox: page }));
                        }
                        if let Some(matrix) = matrix {
                            out.push(element(DisplayItem::Push));
                            out.push(element(DisplayItem::Transform(matrix)));
                        }
                        out.extend(inner);
                        if matrix.is_some() {
                            out.push(element(DisplayItem::Pop));
                        }
                        if translucent {
                            out.push(element(DisplayItem::EndGroup));
                        }
                    }
                    _ => out.extend(inner),
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    items: Vec<DisplayElement>,
    /// Whether the items are in the order in which they are painted: the order of `items` is
    /// kept then, it is not sorted by the z index again.
    ordered: bool,
}

impl DisplayList {
    /// Wraps the items of the boxes that have a `transform` in the transformation, in the order
    /// of the painting. After this, the items are not sorted by the z index again.
    pub fn apply_transforms(
        &mut self,
        scopes: &crate::layout::components::TransformScopes,
        positions: &ReadStorage<Position>,
        page: Rect,
    ) {
        if scopes.boxes.is_empty() {
            return;
        }
        self.items.sort_by_key(|elem| ZIndex(elem.z_index.0));
        let items = std::mem::take(&mut self.items);
        self.items = wrap_transforms(items, 0, scopes, positions, page);
        self.ordered = true;
    }

    pub fn append(&mut self, item: DisplayItem, entity: Entity, z_index: ZIndex) {
        self.items.push(DisplayElement {
            item,
            entity,
            z_index,
        })
    }

    pub fn items(&mut self) -> impl ExactSizeIterator<Item = &DisplayItem> {
        if !self.ordered {
            self.items.sort_by_key(|elem| ZIndex(elem.z_index.0));
        }
        self.items.iter().map(|elem| &elem.item)
    }

    fn stroked_line(
        &mut self,
        start: Point,
        end: Point,
        width: PixelLength,
        color: RGBA,
        entity: Entity,
        z: ZIndex,
    ) {
        self.append(DisplayItem::Push, entity, z);
        self.append(DisplayItem::StrokeColor(color), entity, z);
        self.append(
            DisplayItem::LineString {
                points: vec![start, end],
                line_width: width,
            },
            entity,
            z,
        );
        self.append(DisplayItem::Pop, entity, z);
    }



    /// Paints a side of the border of a box as bands that end at the diagonals through the
    /// corners, where the sides meet in different tones. `edges` are the sides of the border box
    /// (`xmin`, `ymin`, `xmax`, `ymax`) and `widths` the widths of the borders (top, right,
    /// bottom, left). The bands are the parts of the width of the side that are given as
    /// fractions, from the outside to the inside, with the colour of each.
    #[allow(clippy::too_many_arguments)]
    fn border_bands(
        &mut self,
        side: Side,
        edges: (PixelLength, PixelLength, PixelLength, PixelLength),
        widths: (PixelLength, PixelLength, PixelLength, PixelLength),
        bands: &[(f32, f32, RGBA)],
        entity: Entity,
        z: ZIndex,
    ) {
        let (xmin, ymin, xmax, ymax) = edges;
        let (top, right, bottom, left) = widths;
        // the corners of the box inside of the border edge, `f` of the way to the padding edge
        let inset = |f: f32| (xmin + left * f, ymin + top * f, xmax - right * f, ymax - bottom * f);
        for &(from, to, color) in bands {
            let (x0, y0, x1, y1) = inset(from);
            let (ix0, iy0, ix1, iy1) = inset(to);
            let p = Point::new;
            let quad = match side {
                Side::Top => [p(x0, y0), p(x1, y0), p(ix1, iy0), p(ix0, iy0)],
                Side::Right => [p(x1, y0), p(x1, y1), p(ix1, iy1), p(ix1, iy0)],
                Side::Bottom => [p(x1, y1), p(x0, y1), p(ix0, iy1), p(ix1, iy1)],
                Side::Left => [p(x0, y1), p(x0, y0), p(ix0, iy0), p(ix0, iy1)],
            };
            let path = vec![
                PathSegment::MoveTo(quad[0]),
                PathSegment::LineTo(quad[1]),
                PathSegment::LineTo(quad[2]),
                PathSegment::LineTo(quad[3]),
                PathSegment::Close,
            ];
            self.append(DisplayItem::Push, entity, z);
            self.append(DisplayItem::Color(color), entity, z);
            self.append(DisplayItem::FillPath { path, even_odd: false }, entity, z);
            self.append(DisplayItem::Pop, entity, z);
        }
    }

    /// The bands of the border of one side of a box with the styles that have several (`double`,
    /// `groove`, `ridge`) or two tones (`inset`, `outset`), as `border_bands` takes them, if the
    /// style is one of these.
    fn toned_bands(side: Side, width: PixelLength, color: RGBA, style: BorderStyle) -> Option<Vec<(f32, f32, RGBA)>> {
        let (light, dark) = border_tones(color);
        Some(match style {
            BorderStyle::Double if width.0 >= 3.0 => vec![(0.0, 1.0 / 3.0, color), (2.0 / 3.0, 1.0, color)],
            BorderStyle::Inset | BorderStyle::Outset => vec![(0.0, 1.0, single_tone(style, side, color))],
            BorderStyle::Groove | BorderStyle::Ridge if width.0 >= 2.0 => {
                let outer_is_dark = side.is_top_left() == (style == BorderStyle::Groove);
                let (outer, inner) = if outer_is_dark { (dark, light) } else { (light, dark) };
                vec![(0.0, 0.5, outer), (0.5, 1.0, inner)]
            }
            _ => return None,
        })
    }

    /// Paints a straight border of `side` along the line from `start` to `end`, which is on an
    /// axis, as wide as `width` and centered on the line, in the `style`.
    #[allow(clippy::too_many_arguments)]
    pub fn border_line(
        &mut self,
        side: Side,
        start: Point,
        end: Point,
        width: PixelLength,
        color: RGBA,
        style: BorderStyle,
        entity: Entity,
        z: ZIndex,
    ) {
        let w = width.0;
        // (a fully transparent border is not painted; the box has the space for it)
        if w <= 0.0 || !style.is_visible() || color.alpha == 0 {
            return;
        }
        let (nx, ny) = side.outward();
        // a line moved by `offset` away from the box
        let stroke = |this: &mut Self, offset: f32, thickness: f32, color: RGBA| {
            let shift = |p: Point| Point::new(p.x + PixelLength(nx * offset), p.y + PixelLength(ny * offset));
            this.stroked_line(shift(start), shift(end), PixelLength(thickness), color, entity, z);
        };
        let (a, b) = ((start.x.0, start.y.0), (end.x.0, end.y.0));
        let (light, dark) = border_tones(color);
        match style {
            BorderStyle::None | BorderStyle::Hidden => {}
            BorderStyle::Solid => stroke(self, 0.0, w, color),
            // a third of the width for each line and for the gap, if that is wide enough to see
            BorderStyle::Double if w >= 3.0 => {
                stroke(self, w / 3.0, w / 3.0, color);
                stroke(self, -w / 3.0, w / 3.0, color);
            }
            BorderStyle::Double => stroke(self, 0.0, w, color),
            BorderStyle::Inset | BorderStyle::Outset => {
                stroke(self, 0.0, w, single_tone(style, side, color))
            }
            // a groove is dark on the outside at the top and left, light on the outside at
            // the bottom and right, and a ridge the other way round
            BorderStyle::Groove | BorderStyle::Ridge => {
                if w < 2.0 {
                    stroke(self, 0.0, w, single_tone(
                        if style == BorderStyle::Groove { BorderStyle::Inset } else { BorderStyle::Outset },
                        side,
                        color,
                    ));
                } else {
                    let outer_is_dark = side.is_top_left() == (style == BorderStyle::Groove);
                    let (outer, inner) = if outer_is_dark { (dark, light) } else { (light, dark) };
                    stroke(self, w / 4.0, w / 2.0, outer);
                    stroke(self, -w / 4.0, w / 2.0, inner);
                }
            }
            BorderStyle::Dashed | BorderStyle::Dotted => {
                let path = if style == BorderStyle::Dashed {
                    dashes_path(a, b, w)
                } else {
                    dots_path(a, b, w)
                };
                self.append(DisplayItem::Push, entity, z);
                self.append(DisplayItem::Color(color), entity, z);
                self.append(DisplayItem::FillPath { path, even_odd: false }, entity, z);
                self.append(DisplayItem::Pop, entity, z);
            }
        }
    }

    /// Paints the background colour and the gradients, clipped to the (rounded) border box
    /// `clip`. The colour is only painted here when the gradients are blended; it is the
    /// caller's job otherwise.
    fn blended_background(
        &mut self,
        bbox: Rect,
        color: RGBA,
        layers: &[Layer],
        clip: &[PathSegment],
        entity: Entity,
        z: ZIndex,
    ) {
        let gradients = layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Gradient(layer) => Some(layer.clone()),
                Layer::Image { .. } => None,
            })
            .collect();
        self.append(
            DisplayItem::BlendGroup(Box::new(BlendGroup { clip: clip.to_vec(), bbox, color, layers: gradients })),
            entity,
            z,
        );
        // the images are not part of the group, so they are not blended: they go over it
        let images: Vec<Layer> = layers.iter().filter(|layer| matches!(layer, Layer::Image { .. })).cloned().collect();
        self.gradients(&images, Some(clip), entity, z);
    }

    /// Paints an outer `box-shadow` of the border box `rect` with the radii `radii`: below the box
    /// and not inside of it.
    ///
    /// The blur is a Gaussian with the standard deviation half the blur radius. It is painted as
    /// a stack of shapes of one colour that grow by the blur radius from the inside out, with the
    /// transparency chosen so that the layers add up to the edge profile of the Gaussian. This is
    /// vector graphics, with no raster image and no soft mask.
    fn box_shadow(
        &mut self,
        rect: Rect,
        radii: [(PixelLength, PixelLength); 4],
        shadow: &crate::style::properties::computed::BoxShadow,
        color: RGBA,
        entity: Entity,
        z: ZIndex,
    ) {
        if color.alpha == 0 {
            return;
        }
        let blur = shadow.blur.max(0.0);
        let layers = if blur > 0.0 { ((blur * 2.0).ceil() as usize).clamp(4, 32) } else { 1 };
        let source = f32::from(color.alpha) / 255.0;
        // the area the shadow can reach, outside of which the clip is a hole around the box
        let reach = blur + shadow.spread.max(0.0) + shadow.x.abs().max(shadow.y.abs()) + 1.0;
        let outer = Rect {
            x: rect.x - PixelLength(reach),
            y: rect.y - PixelLength(reach),
            width: rect.width + PixelLength(2.0 * reach),
            height: rect.height + PixelLength(2.0 * reach),
        };
        let mut hole = rect_path(outer);
        hole.extend(rounded_rect_path(rect, radii));

        self.append(DisplayItem::Push, entity, z);
        self.append(DisplayItem::ClipPath { path: hole, even_odd: true }, entity, z);
        // the edge profile: the share of the colour at `distance` outside of the edge
        let profile = |distance: f32| 0.5 * erfc(distance / (blur / 2.0 * std::f32::consts::SQRT_2));
        // `achieved` is what the layers painted so far add up to, with the transparency of every
        // one rounded to 8 bits: the error of a layer is made good by the next ones
        let mut achieved = 0.0_f32;
        for layer in 0..layers {
            // from the outermost layer to the innermost: its edge is `offset` outside of the
            // edge of the shadow, and it reaches `step` inward from there
            let (offset, target) = if blur > 0.0 {
                let step = 2.0 * blur / layers as f32;
                let offset = blur - step * layer as f32;
                (offset, profile(offset - step / 2.0))
            } else {
                (0.0, 1.0)
            };
            let wanted = target * source;
            let needed = if achieved < 1.0 { 1.0 - (1.0 - wanted) / (1.0 - achieved) } else { 0.0 };
            let alpha = (needed * 255.0).round().clamp(0.0, 255.0) as u8;
            achieved = 1.0 - (1.0 - achieved) * (1.0 - f32::from(alpha) / 255.0);
            let outset = shadow.spread + offset;
            let width = rect.width.0 + 2.0 * outset;
            let height = rect.height.0 + 2.0 * outset;
            if alpha == 0 || width <= 0.0 || height <= 0.0 {
                continue;
            }
            let layer_rect = Rect {
                x: rect.x + PixelLength(shadow.x - outset),
                y: rect.y + PixelLength(shadow.y - outset),
                width: PixelLength(width),
                height: PixelLength(height),
            };
            // the corners grow with the shadow, but a square corner stays square
            let grow = |(rx, ry): (PixelLength, PixelLength)| {
                if rx.0 <= 0.0 || ry.0 <= 0.0 {
                    (PixelLength(0.0), PixelLength(0.0))
                } else {
                    (PixelLength((rx.0 + outset).max(0.0)), PixelLength((ry.0 + outset).max(0.0)))
                }
            };
            let grown = [grow(radii[0]), grow(radii[1]), grow(radii[2]), grow(radii[3])];
            let fitted = fit_radii(grown, layer_rect.width, layer_rect.height);
            let color = RGBA { alpha, ..color };
            self.append(DisplayItem::Push, entity, z);
            self.append(DisplayItem::Color(color), entity, z);
            self.append(
                DisplayItem::FillPath { path: rounded_rect_path(layer_rect, fitted), even_odd: false },
                entity,
                z,
            );
            self.append(DisplayItem::Pop, entity, z);
        }
        self.append(DisplayItem::Pop, entity, z);
    }

    /// Paints the background of a style over a rectangle, behind everything else.
    fn page_background(&mut self, style: &ComputedValues, rect: Rect, entity: Entity) {
        let z = ZIndex(i64::MIN);
        let color = crate::style::properties::color::color_to_rgba(style.background.background_color);
        let gradients = background_gradients(style, rect, rect, rect);
        if gradients.iter().any(|layer| layer.blend() != BlendMode::Normal) {
            self.blended_background(rect, color, &gradients, &rect_path(rect), entity, z);
            return;
        }
        if color != RGBA::transparent() {
            self.append(DisplayItem::Push, entity, z);
            self.append(DisplayItem::Color(color), entity, z);
            self.append(DisplayItem::FilledRect(rect), entity, z);
            self.append(DisplayItem::Pop, entity, z);
        }
        self.gradients(&gradients, None, entity, z);
    }

    /// Paints gradients, clipped to the (rounded) border box `clip`.
    fn gradients(&mut self, gradients: &[Layer], clip: Option<&[PathSegment]>, entity: Entity, z: ZIndex) {
        for layer in gradients {
            let layer_clip = match layer {
                Layer::Gradient(GradientLayer { clip, .. }) | Layer::Image { clip, .. } => *clip,
            };
            self.append(DisplayItem::Push, entity, z);
            self.append(
                DisplayItem::ClipPath {
                    path: clip.map_or_else(|| rect_path(layer_clip), |clip| clip.to_vec()),
                    even_odd: false,
                },
                entity,
                z,
            );
            match layer {
                Layer::Gradient(GradientLayer { gradient, .. }) => {
                    self.append(DisplayItem::Gradient(Box::new(gradient.clone())), entity, z)
                }
                Layer::Image { image, tiles, .. } => {
                    for &rect in tiles {
                        self.append(DisplayItem::Image { rect, image: image.clone() }, entity, z);
                    }
                }
            }
            self.append(DisplayItem::Pop, entity, z);
        }
    }

    /// Paints the background and borders of a box with rounded corners. `outer` are the radii
    /// of the border edge, the radii of the padding edge are derived from them.
    #[allow(clippy::too_many_arguments)]
    fn rounded_decoration(
        &mut self,
        rect: Rect,
        outer: [(PixelLength, PixelLength); 4],
        border: EdgeValues<PixelLength>,
        border_color: EdgeValues<RGBA>,
        background_color: RGBA,
        gradients: &[Layer],
        entity: Entity,
        z: ZIndex,
    ) {
        let zero = PixelLength(0.0);
        let outer_path = rounded_rect_path(rect, outer);

        let blended = gradients.iter().any(|layer| layer.blend() != BlendMode::Normal);
        if blended {
            self.blended_background(rect, background_color, gradients, &outer_path, entity, z);
        } else if background_color != RGBA::transparent() {
            self.append(DisplayItem::Push, entity, z);
            self.append(DisplayItem::Color(background_color), entity, z);
            self.append(
                DisplayItem::FillPath {
                    path: outer_path.clone(),
                    even_odd: false,
                },
                entity,
                z,
            );
            self.append(DisplayItem::Pop, entity, z);
        }
        if !blended {
            self.gradients(gradients, Some(&outer_path), entity, z);
        }

        if border.top <= zero && border.bottom <= zero && border.left <= zero && border.right <= zero
        {
            return;
        }

        // the padding edge: the radii shrink by the width of the adjacent borders
        let inner_rect = Rect {
            x: rect.x + border.left,
            y: rect.y + border.top,
            width: (rect.width - border.left - border.right).max(zero),
            height: (rect.height - border.top - border.bottom).max(zero),
        };
        let shrink = |(rx, ry): (PixelLength, PixelLength), bx: PixelLength, by: PixelLength| {
            ((rx - bx).max(zero), (ry - by).max(zero))
        };
        let inner = [
            shrink(outer[0], border.left, border.top),
            shrink(outer[1], border.right, border.top),
            shrink(outer[2], border.right, border.bottom),
            shrink(outer[3], border.left, border.bottom),
        ];
        let mut ring = outer_path;
        ring.extend(rounded_rect_path(inner_rect, inner));

        let uniform = border.top == border.bottom
            && border.top == border.left
            && border.top == border.right
            && border_color.top == border_color.bottom
            && border_color.top == border_color.left
            && border_color.top == border_color.right;
        if uniform {
            self.append(DisplayItem::Push, entity, z);
            self.append(DisplayItem::Color(border_color.top), entity, z);
            self.append(
                DisplayItem::FillPath {
                    path: ring,
                    even_odd: true,
                },
                entity,
                z,
            );
            self.append(DisplayItem::Pop, entity, z);
            return;
        }

        // Every side is painted into the ring, clipped to the region between the diagonals
        // through the corners of the border edge and the padding edge.
        let p = Point::new;
        let (ox0, oy0, ox1, oy1) = (rect.x, rect.y, rect.xmax(), rect.ymax());
        let (ix0, iy0, ix1, iy1) = (
            inner_rect.x,
            inner_rect.y,
            inner_rect.xmax(),
            inner_rect.ymax(),
        );
        let wedge = |pts: [Point; 4]| {
            vec![
                PathSegment::MoveTo(pts[0]),
                PathSegment::LineTo(pts[1]),
                PathSegment::LineTo(pts[2]),
                PathSegment::LineTo(pts[3]),
                PathSegment::Close,
            ]
        };
        let sides = [
            (border.top, border_color.top, [p(ox0, oy0), p(ox1, oy0), p(ix1, iy0), p(ix0, iy0)]),
            (border.right, border_color.right, [p(ox1, oy0), p(ox1, oy1), p(ix1, iy1), p(ix1, iy0)]),
            (border.bottom, border_color.bottom, [p(ox1, oy1), p(ox0, oy1), p(ix0, iy1), p(ix1, iy1)]),
            (border.left, border_color.left, [p(ox0, oy1), p(ox0, oy0), p(ix0, iy0), p(ix0, iy1)]),
        ];
        for (width, color, pts) in sides {
            if width <= zero {
                continue;
            }
            self.append(DisplayItem::Push, entity, z);
            self.append(
                DisplayItem::ClipPath {
                    path: ring.clone(),
                    even_odd: true,
                },
                entity,
                z,
            );
            self.append(
                DisplayItem::ClipPath {
                    path: wedge(pts),
                    even_odd: false,
                },
                entity,
                z,
            );
            self.append(DisplayItem::Color(color), entity, z);
            self.append(DisplayItem::FilledRect(rect), entity, z);
            self.append(DisplayItem::Pop, entity, z);
        }
    }
}

#[derive(Debug, Clone)]
#[derive(Default)]
pub struct PrintPage {
    pub display_list: DisplayList,
    pub width: PixelLength,
    pub height: PixelLength,
    pub content_rect: Rect,
}

pub struct PrintPages {
    pub font_map: crate::layout::FontMap,
    pub pages: Vec<PrintPage>,
    /// All hyperlinks in the document, in page order.
    pub hyperlinks: Vec<Hyperlink>,
    /// The headings of the document, in document order (the bookmarks of the PDF).
    pub outline: Vec<crate::layout::components::OutlineEntry>,
    /// Where the elements with an `id` start (the named destinations of the PDF).
    pub named_destinations: std::collections::BTreeMap<String, crate::layout::components::GoToDestination>,
    /// The logical structure of the document (tagged PDF).
    pub structure: crate::tagging::StructureTree,
    /// The content of the `foreignObject` elements of SVG images, laid out like small pages, by
    /// the handle of the element.
    pub islands: Vec<PrintPage>,
}

/// The content of the `foreignObject` elements, a resource of the world of the document.
#[derive(Default)]
pub struct IslandPages(pub Vec<PrintPage>);

impl PrintPages {
    pub fn append_page(&mut self, page: PrintPage) {
        self.pages.push(page)
    }
}

#[derive(Debug, Default)]
pub struct DisplayDecoration {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
    /// The element whose background is the background of the canvas. It is painted behind the
    /// pages and not again in the box of the element.
    pub canvas_source: Option<Entity>,
}

impl<'a> System<'a> for DisplayDecoration {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, Decoration>,
        ReadStorage<'a, crate::layout::components::Fragmented>,
        ReadStorage<'a, ComputedValues>,
        ReadStorage<'a, DecorationFragment>,
    );

    fn run(
        &mut self,
        (entities, positions, decorations, fragmented, styles, fragments): Self::SystemData,
    ) {
        for (
            entity,
            &Position {
                x,
                y,
                width,
                height,
                z,
                ..
            },
            deco,
            _,
            _,
        ) in (
            &entities,
            &positions,
            &decorations,
            &self.bitset,
            !&fragmented,
        )
            .join()
        {
            use PixelLength as Px;

            let &Decoration {
                padding,
                border,
                background_color,
                border_color,
                border_style,
                collapsed,
                ..
            } = deco;

            let border_box = Rect {
                x: x - padding.left - border.left,
                y: y - padding.top - border.top,
                width: width + padding.left + padding.right + border.left + border.right,
                height: height + padding.top + padding.bottom + border.top + border.bottom,
            };
            // the images of a fragment of a box that is split across pages are those of the box
            let style_entity = fragments.get(entity).map_or(entity, |fragment| fragment.0);
            let propagated = self.canvas_source == Some(style_entity);
            let background_color = if propagated {
                RGBA::transparent()
            } else {
                background_color
            };
            let gradients = styles
                .get(style_entity)
                .map(|style| {
                    let padding_box = Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    };
                    let content_box = Rect { x, y, width, height };
                    background_gradients(style, padding_box, border_box, content_box)
                })
                .filter(|_| !propagated)
                .unwrap_or_default();
            if let Some(style) = styles.get(style_entity) {
                // the shadows, the first on top: so the last is painted first
                let radii = resolve_radii(&deco.radius, border_box.width, border_box.height);
                for shadow in style.box_.box_shadow.iter().rev().filter(|shadow| !shadow.inset) {
                    use crate::style::properties::computed::Color;
                    let color = match shadow.color {
                        Color::CurrentColor => crate::style::properties::color::color_to_rgba(style.font.color),
                        color => crate::style::properties::color::color_to_rgba(color),
                    };
                    self.display_list.box_shadow(border_box, radii, shadow, color, entity, z);
                }
            }
            if collapsed.is_none() && deco.radius.is_rounded() {
                let outer = resolve_radii(&deco.radius, border_box.width, border_box.height);
                if outer.iter().any(|&(rx, ry)| rx.0 > 0.0 && ry.0 > 0.0) {
                    // the sides of a rounded border are filled with one colour: the styles that
                    // have a pattern or two tones are drawn solid, `inset` and `outset` keep
                    // their tones
                    let border_color = EdgeValues {
                        top: single_tone(border_style.top, Side::Top, border_color.top),
                        bottom: single_tone(border_style.bottom, Side::Bottom, border_color.bottom),
                        left: single_tone(border_style.left, Side::Left, border_color.left),
                        right: single_tone(border_style.right, Side::Right, border_color.right),
                    };
                    self.display_list.rounded_decoration(
                        border_box, outer, border, border_color, background_color, &gradients, entity, z,
                    );
                    continue;
                }
            }

            let blended = gradients.iter().any(|layer| layer.blend() != BlendMode::Normal);
            if blended {
                self.display_list.blended_background(
                    border_box,
                    background_color,
                    &gradients,
                    &rect_path(border_box),
                    entity,
                    z,
                );
            } else if background_color != RGBA::transparent() {
                self.display_list.append(DisplayItem::Push, entity, z);
                self.display_list
                    .append(DisplayItem::Color(background_color), entity, z);
                // the colour is clipped to the box of the last layer's `background-clip`
                let clip = styles
                    .get(style_entity)
                    .and_then(|style| {
                        // the clip of the bottom-most layer
                        let layers = style.background.background_image.len().max(1);
                        let clips = &style.background.background_clip;
                        clips.get((layers - 1) % clips.len().max(1)).copied()
                    })
                    .unwrap_or_default();
                let clip_rect = match clip {
                    BackgroundBox::BorderBox => border_box,
                    BackgroundBox::PaddingBox => Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    },
                    BackgroundBox::ContentBox => Rect { x, y, width, height },
                };
                self.display_list.append(DisplayItem::FilledRect(clip_rect), entity, z);
                self.display_list.append(DisplayItem::Pop, entity, z);
            }
            if !blended {
                self.display_list.gradients(&gradients, None, entity, z);
            }

            let xmin = x - padding.left - border.left;
            let xmax = x + width + padding.right + border.right;
            let ymin = y - padding.top - border.top;
            let ymax = y + height + padding.bottom + border.bottom;

            if let Some(edges) = collapsed {
                // the borders are centered on the edges of the border box
                let lines = [
                    (Side::Top, edges.top, (xmin, ymin), (xmax, ymin)),
                    (Side::Bottom, edges.bottom, (xmin, ymax), (xmax, ymax)),
                    (Side::Left, edges.left, (xmin, ymin), (xmin, ymax)),
                    (Side::Right, edges.right, (xmax, ymin), (xmax, ymax)),
                ];
                for (side, edge, (x1, y1), (x2, y2)) in lines {
                    if !edge.paint || edge.width <= Px(0.0) {
                        continue;
                    }
                    let (start, end) = if y1 == y2 {
                        (
                            Point::new(x1 - edge.extend_start, y1),
                            Point::new(x2 + edge.extend_end, y2),
                        )
                    } else {
                        (
                            Point::new(x1, y1 - edge.extend_start),
                            Point::new(x2, y2 + edge.extend_end),
                        )
                    };
                    self.display_list
                        .border_line(side, start, end, edge.width, edge.color, edge.style, entity, z);
                }
                continue;
            }

            let hbt = border.top / 2.0;
            let hbb = border.bottom / 2.0;
            let hbl = border.left / 2.0;
            let hbr = border.right / 2.0;

            // the left and right borders of a pattern leave the corners to the top and the
            // bottom, which would have dots and dashes on top of each other
            let patterned = |style: BorderStyle| matches!(style, BorderStyle::Dashed | BorderStyle::Dotted);
            let corner = |style: BorderStyle, width: Px| if patterned(style) { width } else { Px(0.0) };

            let edges = (xmin, ymin, xmax, ymax);
            let widths = (border.top, border.right, border.bottom, border.left);
            let sides = [
                (
                    Side::Top,
                    Point::new(xmin, ymin + hbt),
                    Point::new(xmax, ymin + hbt),
                    border.top,
                    border_color.top,
                    border_style.top,
                ),
                (
                    Side::Bottom,
                    Point::new(xmin, ymax - hbb),
                    Point::new(xmax, ymax - hbb),
                    border.bottom,
                    border_color.bottom,
                    border_style.bottom,
                ),
                (
                    Side::Left,
                    Point::new(xmin + hbl, ymin + corner(border_style.left, border.top)),
                    Point::new(xmin + hbl, ymax - corner(border_style.left, border.bottom)),
                    border.left,
                    border_color.left,
                    border_style.left,
                ),
                (
                    Side::Right,
                    Point::new(xmax - hbr, ymin + corner(border_style.right, border.top)),
                    Point::new(xmax - hbr, ymax - corner(border_style.right, border.bottom)),
                    border.right,
                    border_color.right,
                    border_style.right,
                ),
            ];
            for (side, start, end, width, color, style) in sides {
                if width <= Px(0.0) {
                    continue;
                }
                match DisplayList::toned_bands(side, width, color, style) {
                    Some(bands) => {
                        self.display_list.border_bands(side, edges, widths, &bands, entity, z)
                    }
                    None => self
                        .display_list
                        .border_line(side, start, end, width, color, style, entity, z),
                }
            }
        }
    }
}

pub struct DisplayText {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayText {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        WriteStorage<'a, Paragraph>,
    );

    /// Paints the paragraphs of the page in the logical order of their content (the order of
    /// the structure tree), so that extractors that follow the content stream read the text in
    /// order. Formulas inside a line are painted where they appear in it (see
    /// [`TextOperator::Embed`]).
    ///
    /// The paragraphs move from the world into the display list: the text of a document is
    /// large, and the world does not need it afterwards.
    fn run(&mut self, (entities, positions, mut paragraphs): Self::SystemData) {
        let on_page: Vec<Entity> = (&entities, &positions, &paragraphs, &self.bitset)
            .join()
            .map(|(entity, ..)| entity)
            .collect();

        let embedded: std::collections::HashSet<Entity> = on_page
            .iter()
            .filter_map(|&entity| paragraphs.get(entity))
            .flat_map(|paragraph| paragraph.ops.iter())
            .filter_map(|op| match op {
                TextOperator::Embed { entity, .. } => Some(*entity),
                _ => None,
            })
            .collect();

        let mut ordered: Vec<(Option<(u32, u32)>, Entity, &Position, Paragraph)> = vec![];
        for entity in on_page {
            if embedded.contains(&entity) {
                continue;
            }
            let (Some(pos), Some(paragraph)) = (positions.get(entity), paragraphs.remove(entity)) else {
                continue;
            };
            let paragraph = resolve_embeds(paragraph, &mut paragraphs, &self.bitset);
            ordered.push((logical_position(&paragraph), entity, pos, paragraph));
        }
        // content without a tag (artifacts like running headers) first, in their old order
        ordered.sort_by_key(|(key, ..)| *key);

        for (_, entity, pos, mut paragraph) in ordered {
            // The z of a box is its depth in the box tree. Like the inline content of a stacking
            // context in CSS (CSS 2.1 appendix E), the text goes over the backgrounds and borders
            // of all boxes, in tree order rather than by depth. The page margin boxes are a
            // layer of their own on top.
            let z = if pos.z == ZIndex(i64::MAX) {
                pos.z
            } else {
                ZIndex(i64::MAX - 1)
            };
            // the colour glyphs go over the text of the paragraph
            let color_glyphs = std::mem::take(&mut paragraph.color_glyphs);
            self.display_list
                .append(DisplayItem::Paragraph(paragraph), entity, z);
            for glyph in color_glyphs {
                self.display_list.append(
                    DisplayItem::ColorGlyph(Box::new(ColorGlyphItem {
                        origin: Point { x: glyph.x, y: glyph.baseline },
                        size: glyph.size,
                        font: glyph.font,
                        glyph: glyph.glyph,
                        color: glyph.color,
                    })),
                    entity,
                    z,
                );
            }
        }
    }
}

/// The content slot and offset of the first tagged content of a paragraph.
fn logical_position(paragraph: &Paragraph) -> Option<(u32, u32)> {
    paragraph.ops.iter().find_map(|op| match op {
        TextOperator::BeginTag(tag) => Some((tag.slot, tag.offset)),
        _ => None,
    })
}

/// Replaces the [`TextOperator::Embed`] placeholders of a paragraph by the operators of the
/// embedded paragraphs, which move out of `paragraphs`.
fn resolve_embeds(
    paragraph: Paragraph,
    paragraphs: &mut WriteStorage<Paragraph>,
    page: &specs::BitSet,
) -> Paragraph {
    if !paragraph
        .ops
        .iter()
        .any(|op| matches!(op, TextOperator::Embed { .. }))
    {
        return paragraph;
    }
    let Paragraph { start_pos, mut ops, mut color_glyphs } = paragraph;
    let old_ops = ops.take_ops();
    ops.reserve(old_ops.len());
    for op in old_ops {
        match op {
            TextOperator::Embed { entity, x, y } => {
                if page.contains(entity.id())
                    && let Some(embedded) = paragraphs.remove(entity)
                {
                    ops.append(embedded.ops);
                    color_glyphs.extend(embedded.color_glyphs);
                }
                ops.push(TextOperator::SetTransform(TransformMatrix::translation(x, y)));
            }
            op => ops.push(op),
        }
    }
    Paragraph { start_pos, ops, color_glyphs }
}

/// Paints the images of `<img>` elements, into their content boxes.
pub struct DisplayImages {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayImages {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, ImageBox>,
    );

    fn run(&mut self, (entities, positions, images): Self::SystemData) {
        for (entity, pos, image, _) in (&entities, &positions, &images, &self.bitset).join() {
            if pos.width.0 <= 0.0 || pos.height.0 <= 0.0 {
                continue;
            }
            self.display_list.append(
                DisplayItem::Image {
                    rect: Rect {
                        x: pos.x,
                        y: pos.y,
                        width: pos.width,
                        height: pos.height,
                    },
                    image: image.image.clone(),
                },
                entity,
                pos.z,
            );
        }
    }
}

pub struct DisplayVectorGraphics {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayVectorGraphics {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, VectorGraphics>,
    );

    fn run(&mut self, (entities, positions, vector_graphics): Self::SystemData) {
        for (entity, pos, vector_graphic, _) in
            (&entities, &positions, &vector_graphics, &self.bitset).join()
        {
            if let Some(color) = vector_graphic.color {
                self.display_list.append(DisplayItem::Push, entity, pos.z);
                self.display_list
                    .append(DisplayItem::StrokeColor(color), entity, pos.z);
            }
            for op in &vector_graphic.ops {
                match op {
                    GraphicsOp::Line {
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        thickness,
                    } => self.display_list.append(
                        DisplayItem::LineString {
                            points: vec![
                                Point {
                                    x: *from_x,
                                    y: *from_y,
                                },
                                Point { x: *to_x, y: *to_y },
                            ],
                            line_width: *thickness,
                        },
                        entity,
                        pos.z,
                    ),
                }
            }
            if vector_graphic.color.is_some() {
                self.display_list.append(DisplayItem::Pop, entity, pos.z);
            }
        }
    }
}

pub struct ContentRectCalculator {
    pub bitset: specs::BitSet,
    pub content_rect: Option<Rect>,
}

impl<'a> System<'a> for ContentRectCalculator {
    type SystemData = (ReadStorage<'a, Position>,);

    fn run(&mut self, (positions,): Self::SystemData) {
        for (
            &Position {
                x,
                y,
                width,
                height,
                ..
            },
            ..,
        ) in (&positions, &self.bitset).join()
        {
            if width == PixelLength(0.0) || height == PixelLength(0.0) {
                continue;
            }
            if let Some(r) = self.content_rect {
                self.content_rect = Some(r.union(Rect {
                    x,
                    y,
                    width,
                    height,
                }))
            } else {
                self.content_rect = Some(Rect {
                    x,
                    y,
                    width,
                    height,
                })
            }
        }
    }
}

pub fn generate_display_list(world: &mut specs::World) -> PrintPages {
    <DisplayText as System>::SystemData::setup(world);
    <DisplayVectorGraphics as System>::SystemData::setup(world);
    <DisplayImages as System>::SystemData::setup(world);

    let pages = world.read_resource::<Option<Pages>>();
    let pages = pages.as_ref().unwrap();
    let mut page_vec = vec![];

    // The background of the canvas is the background of the root element, or of the body if the
    // root element has none. It is painted over every page, on top of the background of the
    // page.
    // https://www.w3.org/TR/css-backgrounds-3/#special-backgrounds
    let canvas = *world.read_resource::<crate::layout::components::CanvasEntities>();
    let has_background = |style: &ComputedValues| {
        crate::style::properties::color::color_to_rgba(style.background.background_color)
            != RGBA::transparent()
            || style
                .background
                .background_image
                .iter()
                .any(|image| !matches!(image, Image::None))
    };
    let canvas_source = {
        let styles = world.read_storage::<ComputedValues>();
        [canvas.html, canvas.body]
            .into_iter()
            .flatten()
            .find(|&entity| styles.get(entity).is_some_and(has_background))
    };
    let page_contexts = world.read_resource::<crate::layout::pages::Pages>();
    let page_styles = world.read_resource::<crate::layout::pages::PageStyles>();
    let background_entity = world.entities().create();

    for (page_index, page) in pages.iter().enumerate() {
        let mut display_list = DisplayList::default();
        // (a page that has no size has nothing to paint)
        if page.size.0.0 > 0.0 && page.size.1.0 > 0.0 {
            let rect = Rect {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: page.size.0,
                height: page.size.1,
            };
            if let Some(style) = page_contexts.0.get(page_index).and_then(|c| page_styles.style(c)) {
                display_list.page_background(style, rect, background_entity);
            }
            if let Some(source) = canvas_source {
                let styles = world.read_storage::<ComputedValues>();
                if let Some(style) = styles.get(source) {
                    display_list.page_background(style, rect, background_entity);
                }
            }
        }
        let mut bitset = specs::BitSet::new();
        page.add_elements_to_set(&mut bitset);

        let mut decorations_builder = DisplayDecoration {
            display_list,
            bitset,
            canvas_source,
        };
        decorations_builder.run_now(world);

        let display_list = decorations_builder.display_list;
        let bitset = decorations_builder.bitset;

        let mut graphics_builder = DisplayVectorGraphics {
            display_list,
            bitset,
        };
        graphics_builder.run_now(world);

        let mut image_builder = DisplayImages {
            display_list: graphics_builder.display_list,
            bitset: graphics_builder.bitset,
        };
        image_builder.run_now(world);

        let display_list = image_builder.display_list;
        let bitset = image_builder.bitset;

        let mut text_builder = DisplayText {
            display_list,
            bitset,
        };
        text_builder.run_now(world);
        let mut display_list = text_builder.display_list;
        {
            let scopes = world.read_resource::<crate::layout::components::TransformScopes>();
            let page_rect = Rect { x: PixelLength(0.0), y: PixelLength(0.0), width: page.size.0, height: page.size.1 };
            display_list.apply_transforms(&scopes, &world.read_storage::<Position>(), page_rect);
        }

        let bitset = text_builder.bitset;

        let mut content_rect_calculator = ContentRectCalculator {
            bitset,
            content_rect: None,
        };
        content_rect_calculator.run_now(world);
        let content_rect = content_rect_calculator.content_rect.unwrap_or_default();

        page_vec.push(PrintPage {
            display_list,
            width: page.size.0,
            height: page.size.1,
            content_rect,
        });
    }

    let islands = if world.has_value::<IslandPages>() {
        std::mem::take(&mut world.write_resource::<IslandPages>().0)
    } else {
        vec![]
    };
    PrintPages {
        islands,
        font_map: (*world.read_resource::<crate::layout::FontMap>()).clone(),
        pages: page_vec,
        hyperlinks: world.read_resource::<Hyperlinks>().0.clone(),
        named_destinations: world
            .try_fetch::<crate::layout::components::NamedDestinations>()
            .map(|destinations| destinations.0.clone())
            .unwrap_or_default(),
        outline: world
            .try_fetch::<crate::layout::components::Outline>()
            .map(|outline| outline.0.clone())
            .unwrap_or_default(),
        structure: world
            .read_resource::<crate::tagging::StructureTree>()
            .deref()
            .clone(),
    }
}
