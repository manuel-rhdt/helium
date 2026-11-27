//! Drawing SVG images (`usvg` trees) into the PDF.
//!
//! An SVG image becomes one form XObject in the coordinate system of the SVG (the origin is at
//! the top left, y points down), which is painted wherever the image is used. Everything that
//! PDF has an equivalent for is drawn as vector graphics: paths with fills and strokes, solid
//! colors, linear and radial gradients (also with the repeat and reflect spread methods),
//! patterns, clip paths, masks, group opacity, blend modes and embedded JPEG and PNG images.
//! Text is shown with the embedded fonts of the document where possible. Filters are not
//! supported and are ignored; neither is the opacity of gradient stops.
//!
//! The output is kept small: the writer tracks the graphics state, so colors, alpha values and
//! line styles are only set when they change and paths need no `q`/`Q` of their own; groups with
//! opacity, masks or blend modes only become a form (a transparency group) where the result
//! depends on it; gradient fills share one shading per gradient; masks, tiling patterns and
//! images that are used again are written once.

use std::collections::{HashMap, HashSet};
use std::io::Result;
use std::sync::Arc;

use lemon_pdf::form::{FormReference, TransparencyGroup};
use lemon_pdf::graphics_state::{BlendMode, ExtGState, FillRule, LineCap, LineJoin, SoftMask};
use lemon_pdf::image::{ColorSpace, ImageXObject};
use lemon_pdf::object::IndirectReference;
use lemon_pdf::shading::{
    AxialShading, Circle, ColorStop, Gradient, Point, RadialShading, Rgb, Shading, ShadingPattern,
};
use lemon_pdf::mesh::{FreeFormMesh, Vertex};
use lemon_pdf::stream::StreamFilter;
use lemon_pdf::content::PageContext;
use lemon_pdf::{DocumentContext, Matrix, Pt};
use usvg::tiny_skia_path::{self, PathSegment};
use usvg::{Node, Transform};

use crate::image::RasterImage;

/// The most repetitions of a gradient that are written out (spread methods `repeat` and
/// `reflect`).
const MAX_PERIODS: i64 = 256;

/// The most children of a group with opacity that are checked for overlapping (see
/// `SvgWriter::is_flat`).
const MAX_FLAT_CHILDREN: usize = 128;

/// The most triangles that one shading of a Gouraud mesh holds (an `sh` stream of this many
/// triangles is a few hundred kilobytes before compression).
const MESH_TRIANGLES: usize = 32768;

/// A color of the graphics state, as far as the writer knows it.
#[derive(Clone, PartialEq)]
enum PaintKey {
    Rgb([u8; 3]),
    /// A color that sRGB cannot hold (see [`crate::pdf::set_fill_color`]).
    Wide(crate::style::properties::computed::PaintColor),
    Pattern(lemon_pdf::object::Name),
}

/// What the writer knows of the graphics state of the content stream it is writing (`None` is
/// unknown). `q` and `Q` push and pop it, like in the stream.
#[derive(Clone, Default)]
struct Tracked {
    fill: Option<PaintKey>,
    stroke: Option<PaintKey>,
    fill_alpha: Option<f64>,
    stroke_alpha: Option<f64>,
    width: Option<f32>,
    cap: Option<usvg::LineCap>,
    join: Option<usvg::LineJoin>,
    miter: Option<f32>,
    /// The dash array and the phase; an empty array is a solid line.
    dash: Option<(Vec<f32>, f32)>,
}

impl Tracked {
    /// The state at the start of a form or pattern: opaque and solid. (The colors and line
    /// widths are inherited from where the form is painted, so they are not known.)
    fn start() -> Self {
        Tracked {
            fill_alpha: Some(1.0),
            stroke_alpha: Some(1.0),
            dash: Some((Vec::new(), 0.0)),
            ..Tracked::default()
        }
    }
}

/// Writes the SVG images of a document. Raster images that are used by several SVG images are
/// embedded once.
pub struct SvgWriter {
    filter: Option<StreamFilter>,
    /// Embedded raster files by a hash of their contents (`usvg` reads a file again for every
    /// `<image>` that refers to it).
    images: HashMap<u64, Vec<(Arc<Vec<u8>>, Option<IndirectReference<ImageXObject>>)>>,
    warned: HashSet<&'static str>,
    /// The content of the `foreignObject` elements, by their handle: a form that is as big as the
    /// element, in points, with its origin at the bottom left corner.
    islands: HashMap<u64, FormReference>,
    /// The fonts of the document that the glyphs of SVG text are shown with: the font of the
    /// font database of the SVG images, and the embedded font (with its units per em).
    font_names: crate::svg_text::SvgFontNames,
    fonts: HashMap<String, (IndirectReference<lemon_pdf::font::Font>, u32, crate::pdf::GlyphEncoding)>,
    /// The colour glyphs (emoji) of the document, which `pdf::color_glyph` paints for the text of
    /// the page too.
    color_glyphs: std::rc::Rc<std::cell::RefCell<crate::pdf::color_glyph::ColorGlyphs>>,
    /// The graphics states of the content streams that are being written (the innermost last);
    /// each has one entry per `q` that is open.
    states: Vec<Vec<Tracked>>,
    /// The opacity of the groups that are drawn without a transparency group of their own: it
    /// multiplies the opacity of what they contain.
    alpha: f64,
    /// The tiling patterns and masks of the tree that is being written, by the address of the
    /// `usvg` object (and the transformation for the patterns), for the ones that are used again.
    tilings: HashMap<(usize, [u32; 6]), IndirectReference<lemon_pdf::pattern::TilingPattern>>,
    masks: HashMap<usize, FormReference>,
    /// The ICC space of the gradient that is being built, if it has a color that sRGB cannot
    /// hold: the colors of its stops are components of that space (see `prepare_gradient`).
    gradient_profile: Option<lemon_pdf::color_space::IccBasedColorSpace>,
}

fn matrix(t: Transform) -> Matrix {
    Matrix::new(
        f64::from(t.sx),
        f64::from(t.ky),
        f64::from(t.kx),
        f64::from(t.sy),
        f64::from(t.tx),
        f64::from(t.ty),
    )
}

fn map(t: Transform, x: f32, y: f32) -> (Pt, Pt) {
    (
        Pt(f64::from(t.sx * x + t.kx * y + t.tx)),
        Pt(f64::from(t.ky * x + t.sy * y + t.ty)),
    )
}

fn concat(c: &mut PageContext<'_, '_, '_>, t: Transform) -> Result<()> {
    if t.is_identity() {
        return Ok(());
    }
    let m = matrix(t).0;
    c.concatenate_matrix(m[0], m[1], m[2], m[3], Pt(m[4]), Pt(m[5]))
}

/// Adds the segments of `path` (transformed by `t`) to the current path.
fn emit_path(c: &mut PageContext<'_, '_, '_>, path: &tiny_skia_path::Path, t: Transform) -> Result<()> {
    let mut current = tiny_skia_path::Point::zero();
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                let (x, y) = map(t, p.x, p.y);
                c.move_to(x, y)?;
                current = p;
            }
            PathSegment::LineTo(p) => {
                let (x, y) = map(t, p.x, p.y);
                c.line_to(x, y)?;
                current = p;
            }
            PathSegment::QuadTo(q, p) => {
                // a quadratic curve is a cubic one with the control points two thirds of the
                // way to the single control point
                let third = |from: f32, to: f32| from + (to - from) * 2.0 / 3.0;
                let (x1, y1) = map(t, third(current.x, q.x), third(current.y, q.y));
                let (x2, y2) = map(t, third(p.x, q.x), third(p.y, q.y));
                let (x3, y3) = map(t, p.x, p.y);
                c.curve_to(x1, y1, x2, y2, x3, y3)?;
                current = p;
            }
            PathSegment::CubicTo(a, b, p) => {
                let (x1, y1) = map(t, a.x, a.y);
                let (x2, y2) = map(t, b.x, b.y);
                let (x3, y3) = map(t, p.x, p.y);
                c.curve_to(x1, y1, x2, y2, x3, y3)?;
                current = p;
            }
            PathSegment::Close => c.close_path()?,
        }
    }
    Ok(())
}

/// Whether the path is made of straight segments that all lie on one line, so that it encloses
/// nothing. (Some viewers paint a hairline for the fill of such a path.)
fn is_line(path: &tiny_skia_path::Path) -> bool {
    let mut points = path.points().iter().copied();
    let Some(first) = points.next() else {
        return true;
    };
    if path.segments().any(|segment| matches!(segment, PathSegment::QuadTo(..) | PathSegment::CubicTo(..))) {
        return false;
    }
    let Some(second) = points.clone().find(|p| *p != first) else {
        return true;
    };
    let (dx, dy) = (second.x - first.x, second.y - first.y);
    let length_squared = dx * dx + dy * dy;
    points.all(|p| {
        let cross = dx * (p.y - first.y) - dy * (p.x - first.x);
        cross * cross <= 1e-10 * length_squared * length_squared
    })
}

fn fill_rule(rule: usvg::FillRule) -> FillRule {
    match rule {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn blend_mode(mode: usvg::BlendMode) -> Option<BlendMode> {
    use usvg::BlendMode as B;
    Some(match mode {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
        B::PlusLighter => return None,
    })
}

fn channel(value: u8) -> f64 {
    f64::from(value) / 255.0
}

fn rgb(color: usvg::Color) -> Rgb {
    Rgb(channel(color.red), channel(color.green), channel(color.blue))
}


impl SvgWriter {
    pub fn new(filter: Option<StreamFilter>) -> Self {
        SvgWriter {
            filter,
            images: HashMap::new(),
            warned: HashSet::new(),
            islands: HashMap::new(),
            font_names: HashMap::new(),
            fonts: HashMap::new(),
            color_glyphs: Default::default(),
            states: Vec::new(),
            alpha: 1.0,
            tilings: HashMap::new(),
            masks: HashMap::new(),
            gradient_profile: None,
        }
    }

    /// Tells the writer the colour glyphs of the document, so that the images share the forms of
    /// the glyphs with the page.
    pub fn set_color_glyphs(
        &mut self,
        color_glyphs: std::rc::Rc<std::cell::RefCell<crate::pdf::color_glyph::ColorGlyphs>>,
    ) {
        self.color_glyphs = color_glyphs;
    }

    /// Tells the writer which fonts of the document show the text of SVG images: `names` maps the
    /// fonts of the font database of the images to the PostScript names, `fonts` the PostScript
    /// names to the embedded fonts and their units per em. Text with a font that is not in them
    /// is drawn as outlines.
    pub fn set_fonts(
        &mut self,
        names: crate::svg_text::SvgFontNames,
        fonts: HashMap<String, (IndirectReference<lemon_pdf::font::Font>, u32, crate::pdf::GlyphEncoding)>,
    ) {
        self.font_names = names;
        self.fonts = fonts;
    }

    /// Tells the writer the form of the content of a `foreignObject`.
    pub fn add_island(&mut self, handle: u64, form: FormReference) {
        self.islands.insert(handle, form);
    }

    /// The state of the content stream that is being written.
    fn state(&mut self) -> &mut Tracked {
        self.states
            .last_mut()
            .and_then(|stack| stack.last_mut())
            .expect("a content stream is being written")
    }

    /// Runs `write`, which writes a form or pattern (a content stream with a graphics state of
    /// its own), with the state of a new content stream.
    fn in_stream<T>(&mut self, write: impl FnOnce(&mut Self) -> T) -> T {
        self.states.push(vec![Tracked::start()]);
        let outer = std::mem::replace(&mut self.alpha, 1.0);
        let result = write(self);
        self.alpha = outer;
        self.states.pop();
        result
    }

    /// `q`
    fn save(&mut self, c: &mut PageContext<'_, '_, '_>) -> Result<()> {
        c.save_graphics_state()?;
        let state = self.state().clone();
        self.states.last_mut().expect("a content stream is being written").push(state);
        Ok(())
    }

    /// `Q`
    fn restore(&mut self, c: &mut PageContext<'_, '_, '_>) -> Result<()> {
        self.states.last_mut().expect("a content stream is being written").pop();
        c.restore_graphics_state()
    }

    fn set_paint_key(&mut self, c: &mut PageContext<'_, '_, '_>, key: PaintKey, stroke: bool) -> Result<()> {
        let state = self.state();
        let known = if stroke { &mut state.stroke } else { &mut state.fill };
        if known.as_ref() == Some(&key) {
            return Ok(());
        }
        *known = Some(key.clone());
        match (key, stroke) {
            (PaintKey::Rgb([r, g, b]), false) => {
                c.device_rgb_fill_color(channel(r), channel(g), channel(b))
            }
            (PaintKey::Rgb([r, g, b]), true) => {
                c.device_rgb_stroke_color(channel(r), channel(g), channel(b))
            }
            (PaintKey::Wide(color), false) => crate::pdf::set_fill_color(c, color).map_err(std::io::Error::other),
            (PaintKey::Wide(color), true) => crate::pdf::set_stroke_color(c, color).map_err(std::io::Error::other),
            (PaintKey::Pattern(name), false) => c.set_fill_pattern(&name),
            (PaintKey::Pattern(name), true) => c.set_stroke_pattern(&name),
        }
    }

    fn set_color(&mut self, c: &mut PageContext<'_, '_, '_>, color: usvg::Color, stroke: bool) -> Result<()> {
        let key = if color.wide.is_some() {
            PaintKey::Wide(paint_color(color))
        } else {
            PaintKey::Rgb([color.red, color.green, color.blue])
        };
        self.set_paint_key(c, key, stroke)
    }

    /// Sets the opacity of fills (and text, images and forms) or strokes.
    fn set_alpha(&mut self, c: &mut PageContext<'_, '_, '_>, alpha: f64, stroke: bool) -> Result<()> {
        let state = self.state();
        let known = if stroke { &mut state.stroke_alpha } else { &mut state.fill_alpha };
        if *known == Some(alpha) {
            return Ok(());
        }
        if alpha != 1.0 {
            crate::pdf::pdfa1_forbid("an SVG with opacity")?;
        }
        *known = Some(alpha);
        if stroke {
            c.set_stroke_alpha(alpha)
        } else {
            c.set_fill_alpha(alpha)
        }
    }

    /// Sets the line width, caps, joins and dashes of a stroke.
    fn set_stroke_style(&mut self, c: &mut PageContext<'_, '_, '_>, stroke: &usvg::Stroke) -> Result<()> {
        let state = self.state();
        let width = stroke.width().get();
        if state.width != Some(width) {
            state.width = Some(width);
            c.line_width(Pt(f64::from(width)))?;
        }
        let state = self.state();
        if state.cap != Some(stroke.linecap()) {
            state.cap = Some(stroke.linecap());
            c.line_cap(match stroke.linecap() {
                usvg::LineCap::Butt => LineCap::Butt,
                usvg::LineCap::Round => LineCap::Round,
                usvg::LineCap::Square => LineCap::ProjectingSquare,
            })?;
        }
        let state = self.state();
        if state.join != Some(stroke.linejoin()) {
            state.join = Some(stroke.linejoin());
            c.line_join(match stroke.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                usvg::LineJoin::Round => LineJoin::Round,
                usvg::LineJoin::Bevel => LineJoin::Bevel,
            })?;
        }
        let state = self.state();
        let miter = stroke.miterlimit().get();
        if state.miter != Some(miter) {
            state.miter = Some(miter);
            c.miter_limit(f64::from(miter))?;
        }
        let state = self.state();
        let dash = (stroke.dasharray().map(<[f32]>::to_vec).unwrap_or_default(), stroke.dashoffset());
        if state.dash.as_ref() != Some(&dash) {
            let dashes: Vec<Pt> = dash.0.iter().map(|&d| Pt(f64::from(d))).collect();
            c.dash_pattern(&dashes, Pt(f64::from(dash.1)))?;
            state.dash = Some(dash);
        }
        Ok(())
    }

    fn warn(&mut self, message: &'static str) {
        if self.warned.insert(message) {
            log::warn!("SVG: {}", message);
        }
    }

    /// Writes `tree` as a form XObject that is as large as the image. The origin is at its top
    /// left corner, so the form has to be painted with a flipped y axis.
    pub fn write(
        &mut self,
        context: &mut DocumentContext<'_>,
        tree: &usvg::Tree,
        unscaled: bool,
    ) -> Result<FormReference> {
        let size = tree.size();
        // (the viewport of an element without a `viewBox` is the box that it is in, which the
        // page clips to: the form does not cut at the size of its attributes)
        let (width, height) = if unscaled { (1.0e5, 1.0e5) } else { (size.width(), size.height()) };
        let bbox = [Pt(0.0), Pt(0.0), Pt(f64::from(width)), Pt(f64::from(height))];
        let filter = self.filter;
        // the addresses of the objects of an earlier tree may be used again
        self.tilings.clear();
        self.masks.clear();
        let form = context.write_form(bbox, None, filter, |form| {
            crate::pdf::set_device_defaults(form)?;
            self.in_stream(|this| this.draw_group(form, tree.root(), Transform::identity()))
        });
        self.tilings.clear();
        self.masks.clear();
        form
    }

    /// Whether the group can be painted without a transparency group although it has an opacity,
    /// a mask or a blend mode. That is the case if everything in it is painted once and in a
    /// single operation, and nothing overlaps: opacity, mask and blend mode of the group then
    /// give the same result when they are applied to every object on its own. It holds for a
    /// group with a single path (with a fill or a stroke, not both), and for paths and images
    /// whose bounding boxes are apart.
    fn is_flat(group: &usvg::Group) -> bool {
        fn collect(group: &usvg::Group, boxes: &mut Vec<usvg::tiny_skia_path::Rect>) -> bool {
            for child in group.children() {
                let bounds = match child {
                    Node::Path(path) => {
                        if !path.is_visible() {
                            continue;
                        }
                        if path.fill().is_some() && path.stroke().is_some() {
                            return false;
                        }
                        path.abs_stroke_bounding_box()
                    }
                    Node::Image(image) => {
                        if !image.is_visible() {
                            continue;
                        }
                        match image.kind() {
                            usvg::ImageKind::JPEG(_) | usvg::ImageKind::PNG(_) => {
                                Some(image.abs_bounding_box())
                            }
                            _ => return false,
                        }
                    }
                    Node::Group(inner) => {
                        let plain = inner.filters().is_empty()
                            && inner.opacity().get() >= 1.0
                            && inner.mask().is_none()
                            && inner.blend_mode() == usvg::BlendMode::Normal;
                        if !plain || !collect(inner, boxes) {
                            return false;
                        }
                        continue;
                    }
                    Node::Text(_) | Node::Foreign(_) => return false,
                };
                match bounds {
                    Some(bounds) => boxes.push(bounds),
                    None => return false,
                }
                if boxes.len() > MAX_FLAT_CHILDREN {
                    return false;
                }
            }
            true
        }
        let mut boxes = Vec::new();
        if !collect(group, &mut boxes) {
            return false;
        }
        // a margin for the antialiased edges
        const MARGIN: f32 = 0.5;
        boxes.iter().enumerate().all(|(i, a)| {
            boxes[i + 1..].iter().all(|b| {
                a.right() + MARGIN <= b.left()
                    || b.right() + MARGIN <= a.left()
                    || a.bottom() + MARGIN <= b.top()
                    || b.bottom() + MARGIN <= a.top()
            })
        })
    }

    /// Draws a group; `ctm` maps the coordinate system of the group's parent to the one of the
    /// form that is being written.
    fn draw_group(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        group: &usvg::Group,
        ctm: Transform,
    ) -> Result<()> {
        if !group.filters().is_empty() {
            self.warn("filters are not supported and are ignored");
        }
        let ctm = ctm.pre_concat(group.transform());
        let opacity = f64::from(group.opacity().get());
        let blend = match blend_mode(group.blend_mode()) {
            Some(mode) => mode,
            None => {
                self.warn("the blend mode `plus-lighter` is not supported");
                BlendMode::Normal
            }
        };
        let effects = opacity < 1.0 || group.mask().is_some() || blend != BlendMode::Normal;
        if effects {
            crate::pdf::pdfa1_forbid("an SVG with opacity, a mask or a blend mode")?;
        }
        let flat = effects && Self::is_flat(group);

        // a group that changes nothing in the graphics state needs no `q` and `Q`; the opacity of
        // a flat group is the writer's own business
        let scoped = !group.transform().is_identity()
            || group.clip_path().is_some()
            || (effects && (!flat || group.mask().is_some() || blend != BlendMode::Normal));
        if scoped {
            self.save(c)?;
        }
        concat(c, group.transform())?;
        if let Some(clip) = group.clip_path() {
            self.apply_clip(c, clip, Transform::identity())?;
        }
        if flat {
            // the mask and blend mode stay in the graphics state; the opacity is the one of
            // everything that is drawn
            if group.mask().is_some() || blend != BlendMode::Normal {
                let mut state = ExtGState::new();
                if blend != BlendMode::Normal {
                    state = state.with_blend_mode(blend);
                }
                if let Some(mask) = group.mask() {
                    let mask_form = self.write_mask(c, mask)?;
                    state = state.with_soft_mask(match mask.kind() {
                        usvg::MaskType::Luminance => SoftMask::luminosity(mask_form),
                        usvg::MaskType::Alpha => SoftMask::alpha(mask_form),
                    });
                }
                c.apply_ext_gstate(state)?;
            }
            let outer = self.alpha;
            self.alpha *= opacity;
            self.draw_children(c, group, ctm)?;
            self.alpha = outer;
        } else if effects {
            // the group is composited as a whole: opacity, mask and blend mode apply to the
            // result and not to the children
            if let Some(bbox) = group.layer_bounding_box() {
                let filter = self.filter;
                let form = c.pdf_context.write_form(
                    [
                        Pt(f64::from(bbox.left())),
                        Pt(f64::from(bbox.top())),
                        Pt(f64::from(bbox.right())),
                        Pt(f64::from(bbox.bottom())),
                    ],
                    Some(TransparencyGroup::new(ColorSpace::DeviceRGB)),
                    filter,
                    |form| {
                        crate::pdf::set_device_defaults(form)?;
                        self.in_stream(|this| this.draw_children(form, group, Transform::identity()))
                    },
                )?;
                let mut state = ExtGState::new();
                if opacity < 1.0 {
                    state = state.with_fill_alpha(opacity).with_stroke_alpha(opacity);
                }
                if blend != BlendMode::Normal {
                    state = state.with_blend_mode(blend);
                }
                if let Some(mask) = group.mask() {
                    let mask_form = self.write_mask(c, mask)?;
                    state = state.with_soft_mask(match mask.kind() {
                        usvg::MaskType::Luminance => SoftMask::luminosity(mask_form),
                        usvg::MaskType::Alpha => SoftMask::alpha(mask_form),
                    });
                }
                c.apply_ext_gstate(state)?;
                let key = c.add_xobject(form);
                c.draw_xobject(&key)?;
            }
        } else {
            self.draw_children(c, group, ctm)?;
        }
        if scoped {
            self.restore(c)?;
        }
        Ok(())
    }

    /// Draws the content of a group: the children, or, for a group with a Gouraud mesh that can
    /// be painted as one shading, the mesh.
    fn draw_children(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        group: &usvg::Group,
        ctm: Transform,
    ) -> Result<()> {
        if let Some(mesh) = group.gouraud() {
            if self.draw_mesh(c, mesh)? {
                return Ok(());
            }
        }
        for child in group.children() {
            self.draw_node(c, child, ctm)?;
        }
        Ok(())
    }

    /// Paints the triangles of a Gouraud mesh (`<g data-umbruch-mesh="gouraud">`) as type 4
    /// shadings, `MESH_TRIANGLES` triangles to a shading. Returns `false`, having painted
    /// nothing, if the mesh cannot be painted this way and the flat triangles of the children
    /// have to be drawn: a shading has no alpha per vertex, so the vertices must have one alpha.
    ///
    /// The coordinates of the triangles are the ones inside of the group, which is the current
    /// user space. The colors are device RGB like the ones of the fills (the form has the sRGB
    /// space as the default for it), and are interpolated as such, like Matplotlib does.
    fn draw_mesh(&mut self, c: &mut PageContext<'_, '_, '_>, mesh: &usvg::GouraudMesh) -> Result<bool> {
        let alpha = mesh.triangles[0].colors[0][3];
        let same_alpha = mesh
            .triangles
            .iter()
            .all(|triangle| triangle.colors.iter().all(|color| (color[3] - alpha).abs() <= 1.0 / 512.0));
        if !same_alpha {
            self.warn("a Gouraud mesh with an alpha that changes between the vertices is drawn with flat triangles");
            return Ok(false);
        }
        let vertex = |triangle: &usvg::GouraudTriangle, i: usize| {
            let [r, g, b, _] = triangle.colors[i];
            Vertex::new(
                f64::from(triangle.points[i].x),
                f64::from(triangle.points[i].y),
                Rgb(f64::from(r), f64::from(g), f64::from(b)),
            )
        };
        let opacity = self.alpha * f64::from(alpha);
        if opacity <= 0.0 {
            return Ok(true);
        }
        self.set_alpha(c, opacity, false)?;
        for chunk in mesh.triangles.chunks(MESH_TRIANGLES) {
            let vertices = |triangle: &usvg::GouraudTriangle| [vertex(triangle, 0), vertex(triangle, 1), vertex(triangle, 2)];
            let mut stream = FreeFormMesh::new(vertices(&chunk[0]));
            for triangle in &chunk[1..] {
                stream = stream.triangle(vertices(triangle));
            }
            let shading = c.pdf_context.write_mesh_shading(&stream).map_err(std::io::Error::other)?;
            let key = c.add_shading(shading);
            c.paint_shading(&key)?;
        }
        Ok(true)
    }

    /// The group of a soft mask: the content of the mask in the coordinate system that is
    /// current while the mask is applied. A mask that several groups use is written once.
    fn write_mask(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        mask: &usvg::Mask,
    ) -> Result<FormReference> {
        let address = mask as *const usvg::Mask as usize;
        if let Some(&form) = self.masks.get(&address) {
            return Ok(form);
        }
        let rect = mask.rect();
        let bbox = [
            Pt(f64::from(rect.left())),
            Pt(f64::from(rect.top())),
            Pt(f64::from(rect.right())),
            Pt(f64::from(rect.bottom())),
        ];
        let group = TransparencyGroup::new(match mask.kind() {
            usvg::MaskType::Luminance => ColorSpace::DeviceGray,
            usvg::MaskType::Alpha => ColorSpace::DeviceRGB,
        });
        let filter = self.filter;
        let form = c.pdf_context.write_form(bbox, Some(group), filter, |form| {
            crate::pdf::set_device_defaults(form)?;
            self.in_stream(|this| {
                if let Some(inner) = mask.mask() {
                    // a mask of the mask: it limits the content that makes the mask
                    let inner_form = this.write_mask(form, inner)?;
                    form.apply_ext_gstate(ExtGState::new().with_soft_mask(match inner.kind() {
                        usvg::MaskType::Luminance => SoftMask::luminosity(inner_form),
                        usvg::MaskType::Alpha => SoftMask::alpha(inner_form),
                    }))?;
                }
                this.draw_group(form, mask.root(), Transform::identity())
            })
        })?;
        self.masks.insert(address, form);
        Ok(form)
    }

    /// Intersects the clipping path with `clip`, whose coordinates are transformed by `t`
    /// first.
    fn apply_clip(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        clip: &usvg::ClipPath,
        t: Transform,
    ) -> Result<()> {
        if let Some(outer) = clip.clip_path() {
            self.apply_clip(c, outer, t)?;
        }
        let t = t.pre_concat(clip.transform());
        let mut rule = None;
        if !self.collect_clip(c, clip.root(), t, &mut rule)? {
            // nothing is inside of the clip path: clip everything
            c.rect(Pt(0.0), Pt(0.0), Pt(0.0), Pt(0.0))?;
            rule = Some(FillRule::NonZero);
        }
        c.clip(rule.unwrap_or(FillRule::NonZero))?;
        c.end_path()
    }

    /// Adds the shapes of the children of a clip path to the current path (as the union of
    /// them, which is exact as long as they do not overlap with opposite directions). Returns
    /// whether there was any.
    fn collect_clip(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        group: &usvg::Group,
        t: Transform,
        rule: &mut Option<FillRule>,
    ) -> Result<bool> {
        let t = t.pre_concat(group.transform());
        let mut any = false;
        for child in group.children() {
            match child {
                Node::Path(path) => {
                    if !path.is_visible() {
                        continue;
                    }
                    emit_path(c, path.data(), t)?;
                    if rule.is_none() {
                        *rule = Some(fill_rule(path.fill().map_or(usvg::FillRule::NonZero, |f| f.rule())));
                    }
                    any = true;
                }
                Node::Group(inner) => {
                    if inner.clip_path().is_some() {
                        self.warn("a clip path inside of a clip path is not supported");
                    }
                    any |= self.collect_clip(c, inner, t, rule)?;
                }
                Node::Text(text) => any |= self.collect_clip(c, text.flattened(), t, rule)?,
                Node::Image(_) | Node::Foreign(_) => {}
            }
        }
        Ok(any)
    }

    fn draw_node(&mut self, c: &mut PageContext<'_, '_, '_>, node: &Node, ctm: Transform) -> Result<()> {
        match node {
            Node::Group(group) => self.draw_group(c, group, ctm),
            Node::Path(path) => self.draw_path(c, path, ctm),
            Node::Image(image) => self.draw_image(c, image, ctm),
            Node::Text(text) => self.draw_text(c, text, ctm),
            Node::Foreign(foreign) => self.draw_foreign(c, foreign),
        }
    }

    /// Whether the text can be shown as text with the embedded fonts of the document: every glyph
    /// has such a font, and the fills and strokes are plain colors (gradients and patterns are
    /// painted through the outlines of the glyphs).
    fn is_plain_text(&self, text: &usvg::Text) -> bool {
        let plain = |paint: &usvg::Paint| matches!(paint, usvg::Paint::Color(_));
        text.layouted().iter().all(|span| {
            span.fill.as_ref().is_none_or(|fill| plain(fill.paint()))
                && span.stroke.as_ref().is_none_or(|stroke| plain(stroke.paint()))
                && span.positioned_glyphs.iter().all(|glyph| {
                    self.font_names.get(&glyph.font).is_some_and(|name| self.fonts.contains_key(name))
                })
        })
    }

    /// Draws text: as text with the embedded fonts if possible (so that it can be selected and
    /// searched, and the glyphs are shared with the text of the document), else as outlines.
    fn draw_text(&mut self, c: &mut PageContext<'_, '_, '_>, text: &usvg::Text, ctm: Transform) -> Result<()> {
        if !self.is_plain_text(text) {
            self.draw_group(c, text.flattened(), ctm)?;
            // the outlines have nothing for the colour glyphs
            self.set_alpha(c, self.alpha, false)?;
            for span in text.layouted().iter().filter(|span| span.visible) {
                let color = match span.fill.as_ref().map(|fill| fill.paint()) {
                    Some(usvg::Paint::Color(color)) => *color,
                    _ => usvg::Color::black(),
                };
                self.paint_color_glyphs(c, span, color)?;
            }
            return Ok(());
        }
        for span in text.layouted() {
            for decoration in [&span.overline, &span.underline].into_iter().flatten() {
                self.draw_path(c, decoration, ctm)?;
            }
            if span.visible {
                self.draw_span(c, span)?;
            }
            if let Some(line) = &span.line_through {
                self.draw_path(c, line, ctm)?;
            }
        }
        Ok(())
    }

    /// Paints the colour glyphs (emoji) of a span, which have no outline to fill: their layers
    /// are painted by the same code as the emoji of the page. `color` is the text color, for the
    /// layers that use it.
    fn paint_color_glyphs(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        span: &usvg::layout::Span,
        color: usvg::Color,
    ) -> Result<()> {
        let color = crate::style::properties::computed::SrgbColor { red: color.red, green: color.green, blue: color.blue, alpha: 255 };
        for glyph in &span.positioned_glyphs {
            let Some(name) = self.font_names.get(&glyph.font) else { continue };
            // the matrix of an outline: font units with y up to the coordinates of the text
            let t = glyph.outline_transform();
            let matrix = [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f64::from);
            let filter = self.filter;
            self.color_glyphs.borrow_mut().paint(
                c,
                name,
                crate::layout::text::font::GlyphIndex(glyph.id.0),
                color,
                matrix,
                filter,
            )?;
        }
        Ok(())
    }

    /// Shows the glyphs of a span. The text matrix of a glyph puts the em square of its font on
    /// the glyph, in the coordinate system of the text (with the y axis down, so it flips).
    fn draw_span(&mut self, c: &mut PageContext<'_, '_, '_>, span: &usvg::layout::Span) -> Result<()> {
        let (fill, stroke) = (span.fill.as_ref(), span.stroke.as_ref());
        if fill.is_none() && stroke.is_none() || span.positioned_glyphs.is_empty() {
            return Ok(());
        }
        // text rendering modes: 0 fills, 1 strokes, 2 fills and strokes (in that order)
        let stroke_first = span.paint_order == usvg::PaintOrder::StrokeAndFill;
        let passes: &[i32] = match (fill.is_some(), stroke.is_some()) {
            (true, true) if stroke_first => &[1, 0],
            (true, true) => &[2],
            (true, false) => &[0],
            _ => &[1],
        };
        let font_size = f64::from(span.font_size.get());
        for &mode in passes {
            self.save(c)?;
            if let (Some(fill), true) = (fill, mode != 1) {
                let usvg::Paint::Color(color) = fill.paint() else {
                    unreachable!("plain text has plain colors")
                };
                self.set_color(c, *color, false)?;
                self.set_alpha(c, self.alpha * f64::from(fill.opacity().get()), false)?;
            } else {
                self.set_alpha(c, self.alpha, false)?;
            }
            if let (Some(stroke), true) = (stroke, mode != 0) {
                let usvg::Paint::Color(color) = stroke.paint() else {
                    unreachable!("plain text has plain colors")
                };
                self.set_color(c, *color, true)?;
                self.set_alpha(c, self.alpha * f64::from(stroke.opacity().get()), true)?;
                self.set_stroke_style(c, stroke)?;
            }
            c.begin_text()?;
            c.push_operand(mode)?;
            c.apply_operator("Tr")?;
            let mut current: Option<&str> = None;
            for glyph in &span.positioned_glyphs {
                let name = &self.font_names[&glyph.font];
                let (font, upem, encoding) = self.fonts[name];
                if current != Some(name.as_str()) {
                    let key = c.add_font(font);
                    c.set_font(&key, Pt(font_size))?;
                    current = Some(name.as_str());
                }
                let scale = f64::from(upem) / font_size;
                let t = glyph.outline_transform().pre_scale(scale as f32, scale as f32);
                for value in [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty] {
                    c.push_operand(f64::from(value))?;
                }
                c.apply_operator("Tm")?;
                c.draw_cid_glyphs([encoding.code(crate::layout::text::font::GlyphIndex(u32::from(glyph.id.0)))])?;
            }
            c.end_text()?;
            // Colour glyphs (emoji) have no outline to fill: the text above only advances over
            // them and gives their text; their layers are painted over it.
            if let (Some(fill), true) = (fill, mode != 1) {
                if let usvg::Paint::Color(color) = fill.paint() {
                    self.paint_color_glyphs(c, span, *color)?;
                }
            }
            self.restore(c)?;
        }
        Ok(())
    }

    /// Draws the content of a `foreignObject`, clipped to its rectangle.
    fn draw_foreign(&mut self, c: &mut PageContext<'_, '_, '_>, foreign: &usvg::ForeignObject) -> Result<()> {
        let Some(&form) = self.islands.get(&foreign.handle()) else {
            self.warn("the content of a foreignObject could not be laid out");
            return Ok(());
        };
        let rect = foreign.rect();
        self.save(c)?;
        // the content is painted with the state it finds: opaque and with solid lines
        self.set_alpha(c, self.alpha, false)?;
        self.set_alpha(c, self.alpha, true)?;
        if self.state().dash != Some((Vec::new(), 0.0)) {
            c.dash_pattern(&[], Pt(0.0))?;
            self.state().dash = Some((Vec::new(), 0.0));
        }
        c.rect(
            Pt(f64::from(rect.x())),
            Pt(f64::from(rect.y())),
            Pt(f64::from(rect.width())),
            Pt(f64::from(rect.height())),
        )?;
        c.clip(FillRule::NonZero)?;
        c.end_path()?;
        // the form is in points with y up, from the bottom left corner; the SVG is in pixels
        // with y down
        let k = 1.0 / 0.75;
        concat(c, Transform::from_row(k, 0.0, 0.0, -k, rect.x(), rect.y() + rect.height()))?;
        let key = c.add_xobject(form);
        c.draw_xobject(&key)?;
        self.restore(c)
    }

    fn draw_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        let stroke_first = path.paint_order() == usvg::PaintOrder::StrokeAndFill;
        for stroke in if stroke_first { [true, false] } else { [false, true] } {
            if stroke {
                self.stroke_path(c, path, ctm)?;
            } else {
                self.fill_path(c, path, ctm)?;
            }
        }
        Ok(())
    }

    fn fill_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<()> {
        let Some(fill) = path.fill() else {
            return Ok(());
        };
        if is_line(path.data()) {
            // a line has no inside
            return Ok(());
        }
        let opacity = self.alpha * f64::from(fill.opacity().get());
        if opacity <= 0.0 {
            return Ok(());
        }
        match fill.paint() {
            usvg::Paint::Color(color) => {
                self.set_alpha(c, opacity, false)?;
                self.set_color(c, *color, false)?;
                emit_path(c, path.data(), Transform::identity())?;
                c.fill_path_with_rule(fill_rule(fill.rule()))
            }
            usvg::Paint::LinearGradient(_) | usvg::Paint::RadialGradient(_) => {
                self.prepare_gradient(c, fill.paint())?;
                let Some((shading, transform)) = self.gradient(fill.paint(), path) else {
                    return Ok(());
                };
                // The path is the clipping path of the shading. The shading is the same for
                // all paths with the same gradient (the transformation puts it in place), so
                // it is written once.
                self.set_alpha(c, opacity, false)?;
                self.save(c)?;
                emit_path(c, path.data(), Transform::identity())?;
                c.clip(fill_rule(fill.rule()))?;
                c.end_path()?;
                concat(c, transform)?;
                let key = c.add_shading(shading);
                c.paint_shading(&key)?;
                self.restore(c)
            }
            usvg::Paint::Pattern(pattern) => {
                self.set_alpha(c, opacity, false)?;
                let tiling = self.tiling_pattern(c, pattern, ctm)?;
                let key = c.add_tiling_pattern(tiling);
                self.set_paint_key(c, PaintKey::Pattern(key), false)?;
                emit_path(c, path.data(), Transform::identity())?;
                c.fill_path_with_rule(fill_rule(fill.rule()))
            }
        }
    }

    fn stroke_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<()> {
        let Some(stroke) = path.stroke() else {
            return Ok(());
        };
        let opacity = self.alpha * f64::from(stroke.opacity().get());
        if opacity <= 0.0 {
            return Ok(());
        }
        match stroke.paint() {
            usvg::Paint::Color(color) => self.set_color(c, *color, true)?,
            usvg::Paint::LinearGradient(_) | usvg::Paint::RadialGradient(_) => {
                // strokes cannot be clipping paths: the shading is the color of a pattern
                self.prepare_gradient(c, stroke.paint())?;
                let Some((shading, transform)) = self.gradient(stroke.paint(), path) else {
                    return Ok(());
                };
                let pattern = ShadingPattern::new(shading).with_matrix(matrix(ctm.pre_concat(transform)));
                let key = c.add_shading_pattern(pattern);
                self.set_paint_key(c, PaintKey::Pattern(key), true)?;
            }
            usvg::Paint::Pattern(pattern) => {
                let tiling = self.tiling_pattern(c, pattern, ctm)?;
                let key = c.add_tiling_pattern(tiling);
                self.set_paint_key(c, PaintKey::Pattern(key), true)?;
            }
        }
        self.set_alpha(c, opacity, true)?;
        self.set_stroke_style(c, stroke)?;
        emit_path(c, path.data(), Transform::identity())?;
        c.stroke_path()
    }

    /// Chooses the color space of the gradient `paint` (see `gradient_profile`): the ICC space of the
    /// wide colors of CSS (Rec. 2020 primaries, gamma 2.4) if a stop has a color that sRGB cannot
    /// hold, else the device space.
    fn prepare_gradient(&mut self, c: &mut PageContext<'_, '_, '_>, paint: &usvg::Paint) -> Result<()> {
        let base: Option<&usvg::BaseGradient> = match paint {
            usvg::Paint::LinearGradient(gradient) => Some(gradient),
            usvg::Paint::RadialGradient(gradient) => Some(gradient),
            _ => None,
        };
        self.gradient_profile = None;
        if !base.is_some_and(|base| base.stops().iter().any(|stop| stop.color().wide.is_some())) {
            return Ok(());
        }
        if crate::pdf::pdfa1_active() {
            // the ICC space of the wide colors is a profile of version 4, which PDF/A-1 does not allow
            self.warn("the wide-gamut colors of a gradient are clipped to sRGB: PDF/A-1 does not allow ICC profiles of version 4");
            return Ok(());
        }
        self.gradient_profile = Some(crate::pdf::wide_color_space(c).map_err(std::io::Error::other)?);
        Ok(())
    }

    /// The color of a gradient stop in the space of the gradient.
    fn stop_color(&self, color: usvg::Color) -> Rgb {
        if self.gradient_profile.is_none() {
            return rgb(color);
        }
        let xyz = paint_color(color).to_xyz().map(f64::from);
        let [r, g, b] = crate::pdf::rec2020_gamma24_components(xyz);
        Rgb(r, g, b)
    }

    /// The shading of a gradient and the transformation from its coordinates to the current
    /// ones, or `None` if there is nothing to paint.
    fn gradient(&mut self, paint: &usvg::Paint, path: &usvg::Path) -> Option<(Shading, Transform)> {
        match paint {
            usvg::Paint::LinearGradient(gradient) => {
                self.linear_gradient(gradient, path).map(|shading| (shading, gradient.transform()))
            }
            usvg::Paint::RadialGradient(gradient) => {
                self.radial_gradient(gradient, path).map(|shading| (shading, gradient.transform()))
            }
            _ => None,
        }
    }

    /// The color stops of a gradient over the parameter range `from..to` (the gradient itself
    /// is `0..1`), with the offsets scaled to `0..1`. Outside of `0..1`, the spread method
    /// decides the colors.
    fn spread_stops(
        &mut self,
        base: &usvg::BaseGradient,
        from: i64,
        to: i64,
    ) -> Vec<ColorStop<Rgb>> {
        if base.stops().iter().any(|stop| stop.opacity().get() < 1.0) {
            self.warn("the opacity of gradient stops is not supported");
        }
        let stops: Vec<(f64, Rgb)> = base
            .stops()
            .iter()
            .map(|stop| (f64::from(stop.offset().get()), self.stop_color(stop.color())))
            .collect();
        let width = (to - from) as f64;
        let mut out = Vec::new();
        for period in from..to {
            let reversed = base.spread_method() == usvg::SpreadMethod::Reflect && period.rem_euclid(2) == 1;
            let mut push = |offset: f64, color: Rgb| {
                out.push(ColorStop::new((period as f64 + offset - from as f64) / width, color));
            };
            if reversed {
                for &(offset, color) in stops.iter().rev() {
                    push(1.0 - offset, color);
                }
            } else {
                for &(offset, color) in &stops {
                    push(offset, color);
                }
            }
        }
        out
    }

    /// The range of whole periods of the gradient that covers `path`: the parameter values
    /// of the corners of its bounding box, rounded outwards.
    fn period_range(
        &self,
        base: &usvg::BaseGradient,
        path: &usvg::Path,
        parameter: impl Fn(f32, f32) -> f64,
    ) -> (i64, i64) {
        if base.spread_method() == usvg::SpreadMethod::Pad {
            return (0, 1);
        }
        let Some(inverse) = base.transform().invert() else {
            return (0, 1);
        };
        let b = path.data().bounds();
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for (x, y) in [(b.left(), b.top()), (b.right(), b.top()), (b.left(), b.bottom()), (b.right(), b.bottom())] {
            let (x, y) = map(inverse, x, y);
            let t = parameter(x.0 as f32, y.0 as f32);
            low = low.min(t);
            high = high.max(t);
        }
        if !low.is_finite() || !high.is_finite() {
            return (0, 1);
        }
        let from = (low.floor() as i64).clamp(-MAX_PERIODS, MAX_PERIODS);
        let to = (high.ceil() as i64).clamp(from + 1, from + MAX_PERIODS);
        (from.min(0), to.max(1))
    }

    fn linear_gradient(
        &mut self,
        gradient: &usvg::LinearGradient,
        path: &usvg::Path,
    ) -> Option<Shading> {
        let base: &usvg::BaseGradient = gradient;
        let (x1, y1, x2, y2) = (
            f64::from(gradient.x1()),
            f64::from(gradient.y1()),
            f64::from(gradient.x2()),
            f64::from(gradient.y2()),
        );
        let (dx, dy) = (x2 - x1, y2 - y1);
        let length_squared = dx * dx + dy * dy;
        if length_squared == 0.0 {
            // a gradient without length paints the color of its last stop
            return self.last_stop_pattern(base);
        }
        let (from, to) = self.period_range(base, path, |x, y| {
            ((f64::from(x) - x1) * dx + (f64::from(y) - y1) * dy) / length_squared
        });
        let stops = self.spread_stops(base, from, to);
        let function = Gradient::new(&stops).ok()?.extended();
        let (from_f, to_f) = (from as f64, to as f64);
        let shading = AxialShading::new(
            Point::new(x1 + from_f * dx, y1 + from_f * dy),
            Point::new(x1 + to_f * dx, y1 + to_f * dy),
            &function,
        );
        Some(match self.gradient_profile {
            Some(profile) => shading.with_icc_profile(profile).into(),
            None => shading.into(),
        })
    }

    fn radial_gradient(
        &mut self,
        gradient: &usvg::RadialGradient,
        path: &usvg::Path,
    ) -> Option<Shading> {
        let base: &usvg::BaseGradient = gradient;
        let (cx, cy, r) = (
            f64::from(gradient.cx()),
            f64::from(gradient.cy()),
            f64::from(gradient.r().get()),
        );
        let (fx, fy, fr) = (
            f64::from(gradient.fx()),
            f64::from(gradient.fy()),
            f64::from(gradient.fr().get()),
        );
        if r <= 0.0 {
            return self.last_stop_pattern(base);
        }
        let concentric = fx == cx && fy == cy;
        let (from, to) = if concentric && base.spread_method() != usvg::SpreadMethod::Pad {
            let (from, to) = self.period_range(base, path, |x, y| {
                ((f64::from(x) - cx).hypot(f64::from(y) - cy) - fr) / (r - fr).max(f64::EPSILON)
            });
            // the distance from the center is never negative
            (0, to.max(from + 1).max(1))
        } else {
            if base.spread_method() != usvg::SpreadMethod::Pad {
                self.warn("the spread method of a radial gradient with a focal point is not supported");
            }
            (0, 1)
        };
        let stops = if concentric || base.spread_method() == usvg::SpreadMethod::Pad {
            self.spread_stops(base, from, to)
        } else {
            self.spread_stops(base, 0, 1)
        };
        let function = Gradient::new(&stops).ok()?.extended();
        let shading = RadialShading::new(
            Circle { x: Pt(fx), y: Pt(fy), radius: Pt(fr) },
            Circle { x: Pt(cx), y: Pt(cy), radius: Pt(fr + to as f64 * (r - fr)) },
            &function,
        );
        Some(match self.gradient_profile {
            Some(profile) => shading.with_icc_profile(profile).into(),
            None => shading.into(),
        })
    }

    /// A shading that has the color of the last stop everywhere.
    fn last_stop_pattern(&mut self, base: &usvg::BaseGradient) -> Option<Shading> {
        let color = self.stop_color(base.stops().last()?.color());
        let function = Gradient::new(&[ColorStop::new(0.0, color), ColorStop::new(1.0, color)])
            .ok()?
            .extended();
        let shading = AxialShading::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0), &function);
        Some(match self.gradient_profile {
            Some(profile) => shading.with_icc_profile(profile).into(),
            None => shading.into(),
        })
    }

    fn tiling_pattern(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        pattern: &usvg::Pattern,
        ctm: Transform,
    ) -> Result<IndirectReference<lemon_pdf::pattern::TilingPattern>> {
        let rect = pattern.rect();
        let (width, height) = (f64::from(rect.width()), f64::from(rect.height()));
        // the pattern space starts at the corner of a tile
        let t = ctm
            .pre_concat(pattern.transform())
            .pre_translate(rect.x(), rect.y());
        // many paths often share a pattern
        let key = (
            pattern as *const usvg::Pattern as usize,
            [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f32::to_bits),
        );
        if let Some(&known) = self.tilings.get(&key) {
            return Ok(known);
        }
        let filter = self.filter;
        let tiling = c.pdf_context.write_tiling_pattern(
            [Pt(0.0), Pt(0.0), Pt(width), Pt(height)],
            Pt(width),
            Pt(height),
            matrix(t),
            filter,
            |cell| {
                crate::pdf::set_device_defaults(cell)?;
                self.in_stream(|this| this.draw_group(cell, pattern.root(), Transform::identity()))
            },
        )?;
        self.tilings.insert(key, tiling);
        Ok(tiling)
    }

    fn draw_image(&mut self, c: &mut PageContext<'_, '_, '_>, image: &usvg::Image, ctm: Transform) -> Result<()> {
        if !image.is_visible() {
            return Ok(());
        }
        let size = image.size();
        let (width, height) = (f64::from(size.width()), f64::from(size.height()));
        let data = match image.kind() {
            usvg::ImageKind::JPEG(data) | usvg::ImageKind::PNG(data) => data,
            usvg::ImageKind::SVG(tree) => {
                // a nested SVG image: its contents, clipped to its size
                self.save(c)?;
                c.rect(Pt(0.0), Pt(0.0), Pt(width), Pt(height))?;
                c.clip(FillRule::NonZero)?;
                c.end_path()?;
                self.draw_group(c, tree.root(), ctm)?;
                return self.restore(c);
            }
            usvg::ImageKind::GIF(_) | usvg::ImageKind::WEBP(_) => {
                self.warn("GIF and WebP images are not supported");
                return Ok(());
            }
        };
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut hasher);
            hasher.finish()
        };
        let known = self
            .images
            .get(&key)
            .and_then(|files| files.iter().find(|(file, _)| file == data))
            .map(|(_, reference)| *reference);
        let reference = match known {
            Some(reference) => reference,
            None => {
                let embedded = RasterImage::from_bytes(data.as_ref().clone())
                    .and_then(|raster| raster.to_pdf_image())
                    .map_err(|error| error.to_string())
                    .and_then(|pdf_image| {
                        c.pdf_context.write_image(pdf_image).map_err(|error| error.to_string())
                    });
                let reference = match embedded {
                    Ok(reference) => Some(reference),
                    Err(error) => {
                        log::warn!("SVG: cannot embed an image: {}", error);
                        None
                    }
                };
                self.images.entry(key).or_default().push((data.clone(), reference));
                reference
            }
        };
        let Some(reference) = reference else {
            return Ok(());
        };
        let name = c.add_xobject(reference);
        self.save(c)?;
        self.set_alpha(c, self.alpha, false)?;
        // the unit square of an image has its top at y = 1, SVG has the top at y = 0
        concat(c, Transform::from_row(width as f32, 0.0, 0.0, -(height as f32), 0.0, height as f32))?;
        c.draw_xobject(&name)?;
        self.restore(c)
    }
}

/// Reads the colours of SVG that `svgtypes` rejects (`color()`, `lab()`, `oklch()`, `color-mix()`, …)
/// with the colour code of the style sheets, so that the colours of an SVG are the ones that CSS means.
#[derive(Debug)]
pub struct SvgColorParser;

impl usvg::ColorParser for SvgColorParser {
    fn parse(&self, text: &str) -> Option<(usvg::Color, u8)> {
        use crate::style::properties::computed::{IccSpace, PaintColor};
        use usvg::{WideColor, WideSpace};
        let color = crate::style::properties::specified::parse_color_text(text)?;
        if matches!(color, cssparser_color::Color::CurrentColor) {
            return None;
        }
        let paint = crate::style::properties::color::color_to_paint(color);
        let srgb = paint.clip_to_srgb();
        let mut out = usvg::Color::new_rgb(srgb.red, srgb.green, srgb.blue);
        out.wide = match paint {
            PaintColor::Srgb(_) => None,
            PaintColor::Icc(icc) => Some(WideColor {
                space: match icc.space {
                    IccSpace::SrgbLinear => WideSpace::SrgbLinear,
                    IccSpace::DisplayP3 => WideSpace::DisplayP3,
                    IccSpace::DisplayP3Linear => WideSpace::DisplayP3Linear,
                    IccSpace::A98Rgb => WideSpace::A98Rgb,
                    IccSpace::ProphotoRgb => WideSpace::ProphotoRgb,
                    IccSpace::Rec2020 => WideSpace::Rec2020,
                },
                components: icc.components(),
            }),
            PaintColor::Xyz(xyz) => Some(WideColor { space: WideSpace::Xyz, components: xyz.components() }),
        };
        Some((out, paint.alpha()))
    }
}

/// The colour that the PDF paints for `color`: the wide one where there is one, else sRGB.
fn paint_color(color: usvg::Color) -> crate::style::properties::computed::PaintColor {
    use crate::style::properties::computed::{IccColor, IccSpace, PaintColor, XyzColor};
    use usvg::WideSpace;
    let Some(wide) = color.wide else {
        return PaintColor::srgb(color.red, color.green, color.blue, 255);
    };
    let [a, b, c] = wide.components;
    let space = match wide.space {
        WideSpace::Xyz => return PaintColor::Xyz(XyzColor::new(a, b, c, 255)),
        WideSpace::SrgbLinear => IccSpace::SrgbLinear,
        WideSpace::DisplayP3 => IccSpace::DisplayP3,
        WideSpace::DisplayP3Linear => IccSpace::DisplayP3Linear,
        WideSpace::A98Rgb => IccSpace::A98Rgb,
        WideSpace::ProphotoRgb => IccSpace::ProphotoRgb,
        WideSpace::Rec2020 => IccSpace::Rec2020,
    };
    PaintColor::Icc(IccColor::new(space, a, b, c, 255))
}
