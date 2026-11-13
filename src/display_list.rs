use crate::layout::components::{
    Decoration, DecorationFragment, EdgeValues, Glyphs, GraphicsOp, Hyperlink, Hyperlinks, Paragraph, Position, TextOperator,
    TransformMatrix, VectorGraphics, ZIndex,
};
use crate::layout::normal_formatting::Pages;
use crate::style::properties::computed::{ComputedValues, PixelLength, RGBA};
use crate::style::properties::background::resolve_layer;
use crate::style::properties::gradient::{BoxSize, Image, Position as BackgroundPosition, Resolved};
use crate::style::properties::specified::BlendMode;

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
fn background_gradients(style: &ComputedValues, area: Rect, paint: Rect) -> Vec<Layer> {
    use crate::style::properties::color::color_to_rgba;
    let current = color_to_rgba(style.font.color);
    let size = |rect: Rect| BoxSize {
        x: rect.x.0,
        y: rect.y.0,
        width: rect.width.0,
        height: rect.height.0,
    };
    let (area, paint) = (size(area), size(paint));
    let modes = &style.background.background_blend_mode;
    style
        .background
        .background_image
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(index, image)| match image {
            Image::Gradient(gradient) => {
                fn cycle<T: Clone>(values: &[T], index: usize, default: T) -> T {
                    if values.is_empty() { default } else { values[index % values.len()].clone() }
                }
                let background = &style.background;
                let gradient = resolve_layer(
                    &gradient.with_current_color(current),
                    &cycle(&background.background_size, index, Default::default()),
                    &cycle(&background.background_position, index, BackgroundPosition {
                        x: Default::default(),
                        y: Default::default(),
                    }),
                    cycle(&background.background_repeat, index, Default::default()),
                    area,
                    paint,
                )?;
                let blend = if modes.is_empty() { BlendMode::Normal } else { modes[index % modes.len()] };
                Some(Layer { gradient, blend })
            }
            Image::None => None,
        })
        .collect()
}

/// A gradient of a background with the way it is blended with the layers below.
#[derive(Debug, Clone)]
pub struct Layer {
    pub gradient: Resolved,
    pub blend: BlendMode,
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
    pub layers: Vec<Layer>,
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
    Color(RGBA),
    StrokeColor(RGBA),
    Text(Point, PixelLength, String, Glyphs),
    Paragraph(Paragraph),
    Translate(PixelLength, PixelLength),
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

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    items: Vec<DisplayElement>,
}

impl DisplayList {
    pub fn append(&mut self, item: DisplayItem, entity: Entity, z_index: ZIndex) {
        self.items.push(DisplayElement {
            item,
            entity,
            z_index,
        })
    }

    pub fn items(&mut self) -> impl ExactSizeIterator<Item = &DisplayItem> {
        self.items.sort_by_key(|elem| ZIndex(elem.z_index.0));
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
        self.append(
            DisplayItem::BlendGroup(Box::new(BlendGroup {
                clip: clip.to_vec(),
                bbox,
                color,
                layers: layers.to_vec(),
            })),
            entity,
            z,
        );
    }

    /// Paints gradients, clipped to the (rounded) border box `clip`.
    fn gradients(&mut self, gradients: &[Layer], clip: &[PathSegment], entity: Entity, z: ZIndex) {
        for Layer { gradient, .. } in gradients {
            self.append(DisplayItem::Push, entity, z);
            self.append(
                DisplayItem::ClipPath {
                    path: clip.to_vec(),
                    even_odd: false,
                },
                entity,
                z,
            );
            self.append(DisplayItem::Gradient(Box::new(gradient.clone())), entity, z);
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

        let blended = gradients.iter().any(|layer| layer.blend != BlendMode::Normal);
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
            self.gradients(gradients, &outer_path, entity, z);
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

    // fn append_text(&mut self, pos: Point, text: &components::Text, z_index: ZIndex) {
    //     self.append(
    //         DisplayItem::Text(
    //             Point {
    //                 y: pos.y + text.baseline,
    //                 ..pos
    //             },
    //             text.font_size,
    //             text.font.clone(),
    //             text.glyphs.clone(),
    //         ),
    //         z_index,
    //     );
    // }
}

#[derive(Debug, Clone)]
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
    /// The logical structure of the document (tagged PDF).
    pub structure: crate::tagging::StructureTree,
}

impl PrintPages {
    pub fn append_page(&mut self, page: PrintPage) {
        self.pages.push(page)
    }
}

#[derive(Debug, Default)]
pub struct DisplayDecoration {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
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
            let gradients = styles
                .get(style_entity)
                .map(|style| {
                    let padding_box = Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    };
                    background_gradients(style, padding_box, border_box)
                })
                .unwrap_or_default();
            if collapsed.is_none() && deco.radius.is_rounded() {
                let outer = resolve_radii(&deco.radius, border_box.width, border_box.height);
                if outer.iter().any(|&(rx, ry)| rx.0 > 0.0 && ry.0 > 0.0) {
                    self.display_list.rounded_decoration(
                        border_box, outer, border, border_color, background_color, &gradients, entity, z,
                    );
                    continue;
                }
            }

            let blended = gradients.iter().any(|layer| layer.blend != BlendMode::Normal);
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
                self.display_list.append(
                    DisplayItem::FilledRect(Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    }),
                    entity,
                    z,
                );
                self.display_list.append(DisplayItem::Pop, entity, z);
            }
            if !blended {
                self.display_list
                    .gradients(&gradients, &rect_path(border_box), entity, z);
            }

            let xmin = x - padding.left - border.left;
            let xmax = x + width + padding.right + border.right;
            let ymin = y - padding.top - border.top;
            let ymax = y + height + padding.bottom + border.bottom;

            if let Some(edges) = collapsed {
                // the borders are centered on the edges of the border box
                let lines = [
                    (edges.top, (xmin, ymin), (xmax, ymin)),
                    (edges.bottom, (xmin, ymax), (xmax, ymax)),
                    (edges.left, (xmin, ymin), (xmin, ymax)),
                    (edges.right, (xmax, ymin), (xmax, ymax)),
                ];
                for (edge, (x1, y1), (x2, y2)) in lines {
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
                        .stroked_line(start, end, edge.width, edge.color, entity, z);
                }
                continue;
            }

            let hbt = border.top / 2.0;
            let hbb = border.bottom / 2.0;
            let hbl = border.left / 2.0;
            let hbr = border.right / 2.0;

            if border.top > Px(0.0) {
                let pt1 = Point::new(xmin, ymin + hbt);
                let pt2 = Point::new(xmax, ymin + hbt);

                self.display_list
                    .stroked_line(pt1, pt2, border.top, border_color.top, entity, z);
            }
            if border.bottom > Px(0.0) {
                let pt1 = Point::new(xmin, ymax - hbb);
                let pt2 = Point::new(xmax, ymax - hbb);
                self.display_list.stroked_line(
                    pt1,
                    pt2,
                    border.bottom,
                    border_color.bottom,
                    entity,
                    z,
                );
            }
            if border.left > Px(0.0) {
                let pt1 = Point::new(xmin + hbl, ymin);
                let pt2 = Point::new(xmin + hbl, ymax);
                self.display_list
                    .stroked_line(pt1, pt2, border.left, border_color.left, entity, z);
            }
            if border.right > Px(0.0) {
                let pt1 = Point::new(xmax - hbr, ymin);
                let pt2 = Point::new(xmax - hbr, ymax);
                self.display_list.stroked_line(
                    pt1,
                    pt2,
                    border.right,
                    border_color.right,
                    entity,
                    z,
                );
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
        ReadStorage<'a, Paragraph>,
    );

    /// Paints the paragraphs of the page in the logical order of their content (the order of
    /// the structure tree), so that extractors that follow the content stream read the text in
    /// order. Formulas inside a line are painted where they appear in it (see
    /// [`TextOperator::Embed`]).
    fn run(&mut self, (entities, positions, paragraphs): Self::SystemData) {
        let on_page: Vec<(Entity, &Position, &Paragraph)> =
            (&entities, &positions, &paragraphs, &self.bitset)
                .join()
                .map(|(entity, pos, paragraph, _)| (entity, pos, paragraph))
                .collect();

        let embedded: std::collections::HashSet<Entity> = on_page
            .iter()
            .flat_map(|(_, _, paragraph)| paragraph.ops.iter())
            .filter_map(|op| match op {
                TextOperator::Embed { entity, .. } => Some(*entity),
                _ => None,
            })
            .collect();

        let mut ordered: Vec<(Option<(u32, u32)>, Entity, &Position, Paragraph)> = on_page
            .iter()
            .filter(|(entity, _, _)| !embedded.contains(entity))
            .map(|&(entity, pos, paragraph)| {
                let paragraph = resolve_embeds(paragraph, &paragraphs, &self.bitset);
                (logical_position(&paragraph), entity, pos, paragraph)
            })
            .collect();
        // content without a tag (artifacts like running headers) first, in their old order
        ordered.sort_by_key(|(key, ..)| *key);

        for (_, entity, pos, paragraph) in ordered {
            // The z of a box is its depth in the box tree. Like the inline content of a stacking
            // context in CSS (CSS 2.1 appendix E), the text goes over the backgrounds and borders
            // of all boxes, in tree order rather than by depth. The page margin boxes are a
            // layer of their own on top.
            let z = if pos.z == ZIndex(i64::MAX) {
                pos.z
            } else {
                ZIndex(i64::MAX - 1)
            };
            self.display_list
                .append(DisplayItem::Paragraph(paragraph), entity, z)
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
/// embedded paragraphs.
fn resolve_embeds(
    paragraph: &Paragraph,
    paragraphs: &ReadStorage<Paragraph>,
    page: &specs::BitSet,
) -> Paragraph {
    if !paragraph
        .ops
        .iter()
        .any(|op| matches!(op, TextOperator::Embed { .. }))
    {
        return paragraph.clone();
    }
    let mut ops = Vec::with_capacity(paragraph.ops.len());
    for op in &paragraph.ops {
        match op {
            &TextOperator::Embed { entity, x, y } => {
                if page.contains(entity.id()) {
                    if let Some(embedded) = paragraphs.get(entity) {
                        ops.extend(embedded.ops.iter().cloned());
                    }
                }
                ops.push(TextOperator::SetTransform(TransformMatrix::translation(x, y)));
            }
            op => ops.push(op.clone()),
        }
    }
    Paragraph {
        start_pos: paragraph.start_pos,
        ops,
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

    let pages = world.read_resource::<Option<Pages>>();
    let pages = pages.as_ref().unwrap();
    let mut page_vec = vec![];
    for page in pages.iter() {
        let display_list = DisplayList::default();
        let mut bitset = specs::BitSet::new();
        page.add_elements_to_set(&mut bitset);

        let mut decorations_builder = DisplayDecoration {
            display_list,
            bitset,
        };
        decorations_builder.run_now(world);

        let display_list = decorations_builder.display_list;
        let bitset = decorations_builder.bitset;

        let mut graphics_builder = DisplayVectorGraphics {
            display_list,
            bitset,
        };
        graphics_builder.run_now(world);

        let display_list = graphics_builder.display_list;
        let bitset = graphics_builder.bitset;

        let mut text_builder = DisplayText {
            display_list,
            bitset,
        };
        text_builder.run_now(world);
        let display_list = text_builder.display_list;

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

    PrintPages {
        font_map: (*world.read_resource::<crate::layout::FontMap>()).clone(),
        pages: page_vec,
        hyperlinks: world.read_resource::<Hyperlinks>().0.clone(),
        structure: world
            .read_resource::<crate::tagging::StructureTree>()
            .deref()
            .clone(),
    }
}
