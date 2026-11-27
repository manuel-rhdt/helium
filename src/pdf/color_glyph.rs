//! Paints colour glyphs (emoji) from the `COLR` table of a font as vector graphics.
//!
//! The paint graph of a glyph (OpenType COLR v0 and v1) is read through the `PaintSink` of
//! `read-fonts` into a tree, which is written as a form XObject, once for every glyph; the form
//! is drawn at every place where the glyph is used. A form is an isolated transparency group, so
//! that the blend modes of its layers act on the glyph and not on the page behind it.
//!
//! * `PaintGlyph` is a clip with the outline of the glyph, filled with the paint: a colour or an
//!   axial or radial shading (COLR's two-circle radial gradients are PDF's radial shadings).
//!   Colour stops with an opacity use a soft mask of gray levels like CSS gradients do.
//! * `PaintTransform` and its variants are `cm`.
//! * `PaintComposite`: `SourceOver` needs nothing, the blend modes are `/BM` on the source group,
//!   and `SourceIn` (the source where the backdrop is) is the source group drawn through a soft
//!   mask that has the opacity of the backdrop. The other Porter-Duff modes are painted as
//!   `SourceOver`.
//! * Sweep gradients (not used by Noto Color Emoji) are a mesh of Coons patches.
//!
//! Glyphs are drawn in font units with the y axis up, which is what PDF has.

use std::collections::HashMap;
use std::sync::Arc;
use std::io::{Error, ErrorKind};

use lemon_pdf::Pt;
use lemon_pdf::content::PageContext;
use lemon_pdf::form::{FormReference, TransparencyGroup};
use lemon_pdf::graphics_state::{BlendMode, ExtGState, FillRule, SoftMask};
use lemon_pdf::image::ColorSpace;
use lemon_pdf::mesh::{CoonsMesh, Patch};
use lemon_pdf::shading::{
    AxialShading, Circle, Color, ColorStop, Gradient, Gray, Point, RadialShading, Rgb, Shading,
};
use lemon_pdf::stream::StreamFilter;
use read_fonts::model::glyph::color::{Brush, ColorStop as StopRecord, CompositeMode, Extend, PaintSink};
use read_fonts::model::glyph::outline::{DrawMemory, DrawOptions, PathSink};
use read_fonts::types::{BoundingBox, GlyphId, Matrix};

use crate::layout::text::font::{Font, GlyphIndex};
use crate::style::properties::computed::{PaintColor, SrgbColor};

/// The colour glyphs of a document: the fonts that have them and the forms that were written for
/// their glyphs (by font, glyph and text colour; `None` for a glyph that has nothing to paint).
///
/// This is the one place that paints colour glyphs. The text of the page and the text of SVG
/// images both go through [`ColorGlyphs::paint`]; they only differ in the matrix that places a
/// glyph.
#[derive(Default)]
pub struct ColorGlyphs {
    fonts: HashMap<String, Arc<Font>>,
    forms: HashMap<(String, u32, [u8; 4]), Option<FormReference>>,
}

impl ColorGlyphs {
    /// The colour fonts among the fonts of a document.
    pub fn new(font_map: &crate::layout::FontMap) -> Self {
        ColorGlyphs {
            fonts: font_map
                .iter()
                .filter(|(_, font)| font.has_color_glyphs())
                .map(|(name, font)| (name.clone(), font.clone()))
                .collect(),
            forms: HashMap::new(),
        }
    }

    /// Paints `glyph` of the font `name` (if it is a colour glyph) with its origin at `(x, y)`
    /// in the units of the current user space, scaled to a font size of `size`.
    pub fn paint_at(
        &mut self,
        context: &mut PageContext<'_, '_, '_>,
        name: &str,
        glyph: GlyphIndex,
        color: PaintColor,
        (x, y): (Pt, Pt),
        size: Pt,
        filter: Option<StreamFilter>,
    ) -> Result<(), Error> {
        let Some(font) = self.fonts.get(name) else { return Ok(()) };
        let scale = size.0 / f64::from(font.upem);
        // the layers of a colour glyph that take the colour of the text are painted in sRGB
        self.paint(context, name, glyph, color.clip_to_srgb(), [scale, 0.0, 0.0, scale, x.0, y.0], filter)
    }

    /// Paints `glyph` of the font `name` (if it is a colour glyph) with the matrix that maps the
    /// font units (y up) to the current user space.
    pub fn paint(
        &mut self,
        context: &mut PageContext<'_, '_, '_>,
        name: &str,
        glyph: GlyphIndex,
        color: SrgbColor,
        matrix: [f64; 6],
        filter: Option<StreamFilter>,
    ) -> Result<(), Error> {
        let Some(font) = self.fonts.get(name).cloned() else { return Ok(()) };
        let key = (name.to_owned(), glyph.0, [color.red, color.green, color.blue, color.alpha]);
        let form = match self.forms.get(&key) {
            Some(form) => *form,
            None => {
                let form = write_glyph(context, &font, glyph, color, filter)?;
                self.forms.insert(key, form);
                form
            }
        };
        let Some(form) = form else { return Ok(()) };
        let [a, b, c, d, e, f] = matrix;
        context.save_graphics_state()?;
        context.concatenate_matrix(a, b, c, d, Pt(e), Pt(f))?;
        let key = context.add_xobject(form);
        context.draw_xobject(&key)?;
        context.restore_graphics_state()
    }
}

/// Reads the glyph and writes its form.
fn write_glyph(
    context: &mut PageContext<'_, '_, '_>,
    font: &Font,
    glyph: GlyphIndex,
    color: SrgbColor,
    filter: Option<StreamFilter>,
) -> Result<Option<FormReference>, Error> {
    let face = &font.face;
    let Some(color_glyph) = face.color_glyphs().get(GlyphId::new(glyph.0)) else {
        return Ok(None);
    };
    let palette = face.palettes().get(0).map_or(&[][..], |palette| palette.colors());
    let foreground = [
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0,
        f32::from(color.alpha) / 255.0,
    ];
    let mut collector = Collector::new(face, palette, foreground);
    if color_glyph.paint(&mut collector).is_err() {
        return Ok(None);
    }
    let Some(root) = collector.finish() else { return Ok(None) };
    super::pdfa1_forbid("colour glyphs (emoji), which are transparency groups")?;
    let upem = f64::from(face.units_per_em());
    // the page of the glyphs is much smaller than this; the clip box of the glyph does the rest
    let extent = Pt(8.0 * upem);
    let bbox = [Pt(-extent.0), Pt(-extent.0), extent, extent];
    let reach = Reach { extent: extent.0, filter };
    let form = context.pdf_context.write_form(
        bbox,
        Some(TransparencyGroup::new(ColorSpace::DeviceRGB)),
        filter,
        |form| {
            super::set_device_defaults(form)?;
            emit_all(form, &root, &reach)
        },
    )?;
    Ok(Some(form))
}

/// A path of the outline of a glyph, in font units.
#[derive(Debug, Clone, Copy)]
enum Segment {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Curve(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Default)]
struct Outline(Vec<Segment>);

impl PathSink for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Segment::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Segment::Line(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push(Segment::Quad(x1, y1, x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.push(Segment::Curve(x1, y1, x2, y2, x, y));
    }
    fn close(&mut self) {
        self.0.push(Segment::Close);
    }
}

/// A colour with straight components in `0..=1`.
type Rgba = [f32; 4];

#[derive(Debug, Clone)]
enum Fill {
    Solid(Rgba),
    Linear { from: (f32, f32), to: (f32, f32), stops: Vec<(f32, Rgba)> },
    Radial { start: (f32, f32, f32), end: (f32, f32, f32), stops: Vec<(f32, Rgba)> },
    Sweep { center: (f32, f32), start: f32, end: f32, stops: Vec<(f32, Rgba)> },
}

#[derive(Debug, Clone)]
enum Node {
    /// Paints the clip.
    Fill(Fill),
    Clip(Vec<Segment>, Vec<Node>),
    ClipBox([f32; 4], Vec<Node>),
    Transform([f32; 6], Vec<Node>),
    Layer(CompositeMode, Vec<Node>),
}

enum Frame {
    Root,
    Clip(Vec<Segment>),
    ClipBox([f32; 4]),
    Transform([f32; 6]),
    Layer(CompositeMode),
}

/// Reads the paint graph into a tree.
struct Collector<'f> {
    face: &'f read_fonts::model::Font,
    /// The colours of the first palette of the font.
    palette: &'f [read_fonts::model::glyph::color::Color],
    /// The text colour, for the palette index 0xFFFF.
    foreground: Rgba,
    stack: Vec<(Frame, Vec<Node>)>,
}

impl<'f> Collector<'f> {
    fn new(
        face: &'f read_fonts::model::Font,
        palette: &'f [read_fonts::model::glyph::color::Color],
        foreground: Rgba,
    ) -> Self {
        Collector { face, palette, foreground, stack: vec![(Frame::Root, Vec::new())] }
    }

    fn finish(mut self) -> Option<Vec<Node>> {
        while self.stack.len() > 1 {
            self.pop();
        }
        let (_, nodes) = self.stack.pop()?;
        (!nodes.is_empty()).then_some(nodes)
    }

    fn push(&mut self, frame: Frame) {
        self.stack.push((frame, Vec::new()));
    }

    fn pop(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let (frame, children) = self.stack.pop().unwrap();
        let node = match frame {
            Frame::Root => return,
            Frame::Clip(path) => Node::Clip(path, children),
            Frame::ClipBox(rect) => Node::ClipBox(rect, children),
            Frame::Transform(matrix) => Node::Transform(matrix, children),
            Frame::Layer(mode) => Node::Layer(mode, children),
        };
        self.stack.last_mut().unwrap().1.push(node);
    }

    /// The colour of a palette entry (0xFFFF is the text colour) with `alpha` applied.
    fn color(&self, palette_index: u16, alpha: f32) -> Rgba {
        let [r, g, b, a] = if palette_index == 0xFFFF {
            self.foreground
        } else {
            match self.palette.get(usize::from(palette_index)) {
                Some(color) => [
                    f32::from(color.red) / 255.0,
                    f32::from(color.green) / 255.0,
                    f32::from(color.blue) / 255.0,
                    f32::from(color.alpha) / 255.0,
                ],
                // a missing entry is transparent, which paints nothing
                None => [0.0; 4],
            }
        };
        [r, g, b, a * alpha]
    }

    fn stops(&self, stops: &[StopRecord]) -> Vec<(f32, Rgba)> {
        stops.iter().map(|stop| (stop.offset, self.color(stop.palette_index, stop.alpha))).collect()
    }
}

fn warn_if_repeating(extend: Extend) {
    if extend != Extend::Pad {
        log::warn!("A repeating COLR gradient is painted with its edge colours.");
    }
}

impl PaintSink for Collector<'_> {
    fn push_transform(&mut self, t: Matrix<f32>) {
        self.push(Frame::Transform([t.xx, t.yx, t.xy, t.yy, t.dx, t.dy]));
    }

    fn pop_transform(&mut self) {
        self.pop();
    }

    fn push_clip_glyph(&mut self, glyph: GlyphId) {
        let mut outline = Outline::default();
        if let Some(glyph) = self.face.outline_glyphs().get(glyph) {
            // a glyph that cannot be drawn clips everything away, like one without an outline
            let _ = glyph.draw(&DrawOptions::new(), DrawMemory::Scratch, &mut outline);
        }
        self.push(Frame::Clip(outline.0));
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.push(Frame::ClipBox([clip_box.x_min, clip_box.y_min, clip_box.x_max, clip_box.y_max]));
    }

    fn pop_clip(&mut self) {
        self.pop();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let fill = match brush {
            Brush::Solid { palette_index, alpha } => Fill::Solid(self.color(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                warn_if_repeating(extend);
                Fill::Linear { from: (p0.x, p0.y), to: (p1.x, p1.y), stops: self.stops(color_stops) }
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                warn_if_repeating(extend);
                Fill::Radial { start: (c0.x, c0.y, r0), end: (c1.x, c1.y, r1), stops: self.stops(color_stops) }
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                warn_if_repeating(extend);
                // read-fonts counts the angles clockwise, the glyph is painted with y up
                Fill::Sweep {
                    center: (c0.x, c0.y),
                    start: -start_angle,
                    end: -end_angle,
                    stops: self.stops(color_stops),
                }
            }
        };
        self.stack.last_mut().unwrap().1.push(Node::Fill(fill));
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.push(Frame::Layer(mode));
    }

    fn pop_layer(&mut self) {
        self.pop();
    }
}

// Writing

/// What the content of a form can reach, in the units of the glyph.
struct Reach {
    extent: f64,
    filter: Option<StreamFilter>,
}

impl Reach {
    fn bbox(&self) -> [Pt; 4] {
        [Pt(-self.extent), Pt(-self.extent), Pt(self.extent), Pt(self.extent)]
    }
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidData, error.to_string())
}

fn pt(value: f32) -> Pt {
    Pt(f64::from(value))
}

fn emit_all(ctx: &mut PageContext<'_, '_, '_>, nodes: &[Node], reach: &Reach) -> Result<(), Error> {
    for node in nodes {
        emit(ctx, node, reach)?;
    }
    Ok(())
}

fn emit_path(ctx: &mut PageContext<'_, '_, '_>, path: &[Segment]) -> Result<(), Error> {
    let mut current = (0.0f32, 0.0f32);
    for &segment in path {
        match segment {
            Segment::Move(x, y) => {
                ctx.move_to(pt(x), pt(y))?;
                current = (x, y);
            }
            Segment::Line(x, y) => {
                ctx.line_to(pt(x), pt(y))?;
                current = (x, y);
            }
            Segment::Quad(x1, y1, x, y) => {
                // a quadratic curve as the cubic with the same shape
                let (x0, y0) = current;
                ctx.curve_to(
                    pt(x0 + 2.0 / 3.0 * (x1 - x0)),
                    pt(y0 + 2.0 / 3.0 * (y1 - y0)),
                    pt(x + 2.0 / 3.0 * (x1 - x)),
                    pt(y + 2.0 / 3.0 * (y1 - y)),
                    pt(x),
                    pt(y),
                )?;
                current = (x, y);
            }
            Segment::Curve(x1, y1, x2, y2, x, y) => {
                ctx.curve_to(pt(x1), pt(y1), pt(x2), pt(y2), pt(x), pt(y))?;
                current = (x, y);
            }
            Segment::Close => ctx.close_path()?,
        }
    }
    Ok(())
}

fn emit(ctx: &mut PageContext<'_, '_, '_>, node: &Node, reach: &Reach) -> Result<(), Error> {
    match node {
        Node::Fill(fill) => paint_fill(ctx, fill, reach),
        Node::Clip(path, children) => {
            ctx.save_graphics_state()?;
            emit_path(ctx, path)?;
            ctx.clip(FillRule::NonZero)?;
            ctx.end_path()?;
            emit_all(ctx, children, reach)?;
            ctx.restore_graphics_state()
        }
        Node::ClipBox([x0, y0, x1, y1], children) => {
            ctx.save_graphics_state()?;
            ctx.rect(pt(*x0), pt(*y0), pt(x1 - x0), pt(y1 - y0))?;
            ctx.clip(FillRule::NonZero)?;
            ctx.end_path()?;
            emit_all(ctx, children, reach)?;
            ctx.restore_graphics_state()
        }
        Node::Transform([a, b, c, d, e, f], children) => {
            ctx.save_graphics_state()?;
            ctx.concatenate_matrix(
                f64::from(*a),
                f64::from(*b),
                f64::from(*c),
                f64::from(*d),
                pt(*e),
                pt(*f),
            )?;
            emit_all(ctx, children, reach)?;
            ctx.restore_graphics_state()
        }
        Node::Layer(_, children) => composite(ctx, children, reach),
    }
}

/// Paints the contents of a `PaintComposite`: the backdrop and the layer of the source (the last
/// child, a `Layer` with its own mode) that is composited onto it.
fn composite(ctx: &mut PageContext<'_, '_, '_>, children: &[Node], reach: &Reach) -> Result<(), Error> {
    let Some((Node::Layer(mode, source), backdrop)) = children.split_last() else {
        return emit_all(ctx, children, reach);
    };
    let group = |ctx: &mut PageContext<'_, '_, '_>, nodes: &[Node]| {
        ctx.pdf_context.write_form(
            reach.bbox(),
            Some(TransparencyGroup::new(ColorSpace::DeviceRGB)),
            reach.filter,
            |form| {
                super::set_device_defaults(form)?;
                emit_all(form, nodes, reach)
            },
        )
    };
    let draw = |ctx: &mut PageContext<'_, '_, '_>, form, state: Option<ExtGState>| -> Result<(), Error> {
        ctx.save_graphics_state()?;
        if let Some(state) = state {
            ctx.apply_ext_gstate(state)?;
        }
        let key = ctx.add_xobject(form);
        ctx.draw_xobject(&key)?;
        ctx.restore_graphics_state()
    };
    match mode {
        CompositeMode::SrcOver => {
            emit_all(ctx, backdrop, reach)?;
            emit_all(ctx, source, reach)
        }
        CompositeMode::Src => emit_all(ctx, source, reach),
        CompositeMode::Dest => emit_all(ctx, backdrop, reach),
        CompositeMode::Clear => Ok(()),
        CompositeMode::DestOver => {
            emit_all(ctx, source, reach)?;
            emit_all(ctx, backdrop, reach)
        }
        CompositeMode::SrcIn => {
            // the source where the backdrop is opaque; the backdrop itself is not painted
            let mask = group(ctx, backdrop)?;
            let source = group(ctx, source)?;
            draw(ctx, source, Some(ExtGState::new().with_soft_mask(SoftMask::alpha(mask))))
        }
        blend => {
            let Some(blend_mode) = blend_mode(*blend) else {
                log::warn!("The COLR composite mode {blend:?} is painted as SourceOver.");
                emit_all(ctx, backdrop, reach)?;
                return emit_all(ctx, source, reach);
            };
            emit_all(ctx, backdrop, reach)?;
            let source = group(ctx, source)?;
            let mut state = ExtGState::new();
            state = state.with_blend_mode(blend_mode);
            draw(ctx, source, Some(state))
        }
    }
}

fn blend_mode(mode: CompositeMode) -> Option<BlendMode> {
    Some(match mode {
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        _ => return None,
    })
}

/// The colour stops of a gradient line, with the offsets mapped to `0..=1`. Returns the
/// `(start, end)` of the line in the old offsets: COLR stops can lie outside of `0..=1`, which
/// moves the ends of the gradient line.
fn normalized(stops: &[(f32, Rgba)]) -> Option<(Vec<(f32, Rgba)>, f32, f32)> {
    let mut stops = stops.to_vec();
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (first, last) = (stops.first()?.0, stops.last()?.0);
    if last - first < 1e-6 {
        return None;
    }
    let stops = stops.iter().map(|&(offset, color)| ((offset - first) / (last - first), color)).collect();
    Some((stops, first, last))
}

fn function<C: Color>(stops: &[(f32, Rgba)], to_color: impl Fn(Rgba) -> C) -> Result<Gradient<C>, Error> {
    let stops: Vec<ColorStop<C>> =
        stops.iter().map(|&(offset, color)| ColorStop::new(f64::from(offset), to_color(color))).collect();
    Ok(Gradient::new(&stops).map_err(invalid)?.extended())
}

/// The shading of a gradient with the colours `to_color` makes of its stops, in glyph units.
fn shading<C: Color>(fill: &Fill, to_color: impl Fn(Rgba) -> C) -> Result<Option<Shading>, Error> {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    Ok(match fill {
        Fill::Linear { from, to, stops } => {
            let Some((stops, first, last)) = normalized(stops) else { return Ok(None) };
            let point = |t: f32| Point::new(f64::from(lerp(from.0, to.0, t)), f64::from(lerp(from.1, to.1, t)));
            let function = function(&stops, to_color)?;
            Some(AxialShading::new(point(first), point(last), &function).into())
        }
        Fill::Radial { start, end, stops } => {
            let Some((stops, first, last)) = normalized(stops) else { return Ok(None) };
            let circle = |t: f32| Circle {
                x: pt(lerp(start.0, end.0, t)),
                y: pt(lerp(start.1, end.1, t)),
                radius: pt(lerp(start.2, end.2, t).max(0.0)),
            };
            let function = function(&stops, to_color)?;
            Some(RadialShading::new(circle(first), circle(last), &function).into())
        }
        Fill::Solid(_) | Fill::Sweep { .. } => None,
    })
}

/// The colour at `offset` of the stops, which are sorted.
fn color_at(stops: &[(f32, Rgba)], offset: f32) -> Rgba {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if offset <= first.0 {
        return first.1;
    }
    if offset >= last.0 {
        return last.1;
    }
    let index = stops.windows(2).position(|pair| pair[0].0 <= offset && offset < pair[1].0).unwrap_or(0);
    let (from, to) = (stops[index], stops[index + 1]);
    let t = ((offset - from.0) / (to.0 - from.0).max(1e-9)).clamp(0.0, 1.0);
    let mut color = [0.0; 4];
    for i in 0..4 {
        color[i] = from.1[i] + (to.1[i] - from.1[i]) * t;
    }
    color
}

/// A sweep gradient as a mesh of sectors, each of which is linear in the angle. Angles are
/// counter-clockwise from the x axis, in degrees, as in COLR.
fn sweep<C: Color>(
    ctx: &mut PageContext<'_, '_, '_>,
    center: (f32, f32),
    start: f32,
    end: f32,
    stops: &[(f32, Rgba)],
    reach: &Reach,
    to_color: impl Fn(Rgba) -> C,
) -> Result<Option<Shading>, Error> {
    const SECTORS: usize = 90;
    let mut stops = stops.to_vec();
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if stops.is_empty() || (end - start).abs() < 1e-6 {
        return Ok(None);
    }
    let mut offsets: Vec<f32> = (0..=SECTORS).map(|i| i as f32 / SECTORS as f32).collect();
    offsets.extend(stops.iter().map(|s| s.0).filter(|o| (0.0..=1.0).contains(o)));
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let radius = reach.extent as f32;
    let rim = |offset: f32| {
        let angle = (start + (end - start) * offset).to_radians();
        (center.0 + radius * angle.cos(), center.1 + radius * angle.sin())
    };
    let point = |(x, y): (f32, f32)| Point::new(f64::from(x), f64::from(y));
    let between = |t: f32, a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let mut mesh: Option<CoonsMesh<C>> = None;
    for pair in offsets.windows(2) {
        let (rim_a, rim_b) = (rim(pair[0]), rim(pair[1]));
        let points = [
            point(center),
            point(between(1.0 / 3.0, center, rim_a)),
            point(between(2.0 / 3.0, center, rim_a)),
            point(rim_a),
            point(between(1.0 / 3.0, rim_a, rim_b)),
            point(between(2.0 / 3.0, rim_a, rim_b)),
            point(rim_b),
            point(between(2.0 / 3.0, center, rim_b)),
            point(between(1.0 / 3.0, center, rim_b)),
            point(center),
            point(center),
            point(center),
        ];
        let (a, b) = (color_at(&stops, pair[0]), color_at(&stops, pair[1]));
        let patch = Patch { points, colors: [to_color(a), to_color(a), to_color(b), to_color(b)] };
        mesh = Some(match mesh {
            None => CoonsMesh::new(patch),
            Some(mesh) => mesh.patch(patch),
        });
    }
    let Some(mesh) = mesh else { return Ok(None) };
    Ok(Some(ctx.pdf_context.write_mesh_shading(&mesh).map_err(invalid)?))
}

fn paint_shading(ctx: &mut PageContext<'_, '_, '_>, shading: Shading) -> Result<(), Error> {
    let key = ctx.add_shading(shading);
    ctx.paint_shading(&key)
}

fn paint_fill(ctx: &mut PageContext<'_, '_, '_>, fill: &Fill, reach: &Reach) -> Result<(), Error> {
    let rgb = |c: Rgba| Rgb(f64::from(c[0]), f64::from(c[1]), f64::from(c[2]));
    let gray = |c: Rgba| Gray(f64::from(c[3]));
    ctx.save_graphics_state()?;
    let result = (|| -> Result<(), Error> {
        let stops = match fill {
            Fill::Solid([r, g, b, a]) => {
                ctx.device_rgb_fill_color(f64::from(*r), f64::from(*g), f64::from(*b))?;
                if *a < 1.0 {
                    ctx.set_fill_alpha(f64::from(*a))?;
                }
                let e = Pt(reach.extent);
                ctx.rect(Pt(-e.0), Pt(-e.0), Pt(2.0 * e.0), Pt(2.0 * e.0))?;
                return ctx.fill_path();
            }
            Fill::Linear { stops, .. } | Fill::Radial { stops, .. } | Fill::Sweep { stops, .. } => stops,
        };
        let build = |ctx: &mut PageContext<'_, '_, '_>, alpha: bool| -> Result<Option<Shading>, Error> {
            match (fill, alpha) {
                (Fill::Sweep { center, start, end, stops }, false) => {
                    sweep(ctx, *center, *start, *end, stops, reach, rgb)
                }
                (Fill::Sweep { center, start, end, stops }, true) => {
                    sweep(ctx, *center, *start, *end, stops, reach, gray)
                }
                (_, false) => shading(fill, rgb),
                (_, true) => shading(fill, gray),
            }
        };
        let Some(color) = build(ctx, false)? else { return Ok(()) };
        let alphas = stops.iter().map(|s| s.1[3]);
        let min = alphas.clone().fold(f32::INFINITY, f32::min);
        let max = alphas.fold(f32::NEG_INFINITY, f32::max);
        if max - min < 1e-4 {
            if max < 1.0 {
                ctx.set_fill_alpha(f64::from(max))?;
            }
        } else {
            // The opacities are a second shading, of gray levels: the soft mask.
            let mask = ctx.pdf_context.write_form(
                reach.bbox(),
                Some(TransparencyGroup::new(ColorSpace::DeviceGray)),
                reach.filter,
                |form| match build(form, true)? {
                    Some(alpha) => paint_shading(form, alpha),
                    None => Ok(()),
                },
            )?;
            ctx.apply_ext_gstate(ExtGState::new().with_soft_mask(SoftMask::luminosity(mask)))?;
        }
        paint_shading(ctx, color)
    })();
    ctx.restore_graphics_state()?;
    result
}
