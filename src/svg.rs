//! Drawing SVG images (`usvg` trees) into the PDF.
//!
//! An SVG image becomes one form XObject in the coordinate system of the SVG (the origin is at
//! the top left, y points down), which is painted wherever the image is used. Everything that
//! PDF has an equivalent for is drawn as vector graphics: paths with fills and strokes, solid
//! colors, linear and radial gradients (also with the repeat and reflect spread methods),
//! patterns, clip paths, masks, group opacity, blend modes and embedded JPEG and PNG images.
//! Text is drawn as outlines (`usvg` converts it). Filters are not supported and are ignored;
//! neither is the opacity of gradient stops.

use std::collections::{HashMap, HashSet};
use std::io::Result;
use std::sync::Arc;

use lemon_pdf::form::{FormReference, TransparencyGroup};
use lemon_pdf::graphics_state::{BlendMode, ExtGState, FillRule, LineCap, LineJoin, SoftMask};
use lemon_pdf::image::{ColorSpace, ImageXObject};
use lemon_pdf::object::IndirectReference;
use lemon_pdf::shading::{
    AxialShading, Circle, ColorStop, Gradient, Point, RadialShading, Rgb, ShadingPattern,
};
use lemon_pdf::stream::StreamFilter;
use lemon_pdf::content::PageContext;
use lemon_pdf::{DocumentContext, Matrix, Pt};
use usvg::tiny_skia_path::{self, PathSegment};
use usvg::{Node, Transform};

use crate::image::RasterImage;

/// The most repetitions of a gradient that are written out (spread methods `repeat` and
/// `reflect`).
const MAX_PERIODS: i64 = 256;

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
    fonts: HashMap<String, (IndirectReference<lemon_pdf::font::Font>, u32)>,
    /// The colour glyphs (emoji) of the document, which `pdf::color_glyph` paints for the text of
    /// the page too.
    color_glyphs: std::rc::Rc<std::cell::RefCell<crate::pdf::color_glyph::ColorGlyphs>>,
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

/// Sets the line width, caps, joins and dashes of a stroke.
fn stroke_style(c: &mut PageContext<'_, '_, '_>, stroke: &usvg::Stroke) -> Result<()> {
    c.line_width(Pt(f64::from(stroke.width().get())))?;
    c.line_cap(match stroke.linecap() {
        usvg::LineCap::Butt => LineCap::Butt,
        usvg::LineCap::Round => LineCap::Round,
        usvg::LineCap::Square => LineCap::ProjectingSquare,
    })?;
    c.line_join(match stroke.linejoin() {
        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
        usvg::LineJoin::Round => LineJoin::Round,
        usvg::LineJoin::Bevel => LineJoin::Bevel,
    })?;
    c.miter_limit(f64::from(stroke.miterlimit().get()))?;
    if let Some(dashes) = stroke.dasharray() {
        let dashes: Vec<Pt> = dashes.iter().map(|&d| Pt(f64::from(d))).collect();
        c.dash_pattern(&dashes, Pt(f64::from(stroke.dashoffset())))?;
    }
    Ok(())
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
        fonts: HashMap<String, (IndirectReference<lemon_pdf::font::Font>, u32)>,
    ) {
        self.font_names = names;
        self.fonts = fonts;
    }

    /// Tells the writer the form of the content of a `foreignObject`.
    pub fn add_island(&mut self, handle: u64, form: FormReference) {
        self.islands.insert(handle, form);
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
    ) -> Result<FormReference> {
        let size = tree.size();
        let bbox = [Pt(0.0), Pt(0.0), Pt(f64::from(size.width())), Pt(f64::from(size.height()))];
        let filter = self.filter;
        context.write_form(bbox, None, filter, |form| {
            self.draw_group(form, tree.root(), Transform::identity())
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
        let layer = opacity < 1.0 || group.mask().is_some() || blend != BlendMode::Normal;

        c.save_graphics_state()?;
        concat(c, group.transform())?;
        if let Some(clip) = group.clip_path() {
            self.apply_clip(c, clip, Transform::identity())?;
        }
        if layer {
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
                        for child in group.children() {
                            self.draw_node(form, child, Transform::identity())?;
                        }
                        Ok(())
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
            for child in group.children() {
                self.draw_node(c, child, ctm)?;
            }
        }
        c.restore_graphics_state()
    }

    /// The group of a soft mask: the content of the mask in the coordinate system that is
    /// current while the mask is applied.
    fn write_mask(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        mask: &usvg::Mask,
    ) -> Result<FormReference> {
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
        c.pdf_context.write_form(bbox, Some(group), filter, |form| {
            if let Some(inner) = mask.mask() {
                // a mask of the mask: it limits the content that makes the mask
                let inner_form = self.write_mask(form, inner)?;
                form.apply_ext_gstate(ExtGState::new().with_soft_mask(match inner.kind() {
                    usvg::MaskType::Luminance => SoftMask::luminosity(inner_form),
                    usvg::MaskType::Alpha => SoftMask::alpha(inner_form),
                }))?;
            }
            self.draw_group(form, mask.root(), Transform::identity())
        })
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
        let color = crate::style::properties::computed::RGBA {
            red: color.red,
            green: color.green,
            blue: color.blue,
            alpha: 255,
        };
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
            c.save_graphics_state()?;
            if let (Some(fill), true) = (fill, mode != 1) {
                let Rgb(r, g, b) = match fill.paint() {
                    usvg::Paint::Color(color) => rgb(*color),
                    _ => unreachable!("plain text has plain colors"),
                };
                c.device_rgb_fill_color(r, g, b)?;
                let opacity = f64::from(fill.opacity().get());
                if opacity < 1.0 {
                    c.set_fill_alpha(opacity)?;
                }
            }
            if let (Some(stroke), true) = (stroke, mode != 0) {
                let Rgb(r, g, b) = match stroke.paint() {
                    usvg::Paint::Color(color) => rgb(*color),
                    _ => unreachable!("plain text has plain colors"),
                };
                c.device_rgb_stroke_color(r, g, b)?;
                let opacity = f64::from(stroke.opacity().get());
                if opacity < 1.0 {
                    c.set_stroke_alpha(opacity)?;
                }
                stroke_style(c, stroke)?;
            }
            c.begin_text()?;
            c.push_operand(mode)?;
            c.apply_operator("Tr")?;
            let mut current: Option<&str> = None;
            for glyph in &span.positioned_glyphs {
                let name = &self.font_names[&glyph.font];
                let (font, upem) = self.fonts[name];
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
                c.draw_cid_glyphs([glyph.id.0 as u16])?;
            }
            c.end_text()?;
            // Colour glyphs (emoji) have no outline to fill: the text above only advances over
            // them and gives their text; their layers are painted over it.
            if let (Some(fill), true) = (fill, mode != 1) {
                if let usvg::Paint::Color(color) = fill.paint() {
                    self.paint_color_glyphs(c, span, *color)?;
                }
            }
            c.restore_graphics_state()?;
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
        c.save_graphics_state()?;
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
        c.restore_graphics_state()
    }

    fn draw_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        let stroke_first = path.paint_order() == usvg::PaintOrder::StrokeAndFill;
        for stroke in if stroke_first { [true, false] } else { [false, true] } {
            c.save_graphics_state()?;
            if stroke {
                self.stroke_path(c, path, ctm)?;
            } else {
                self.fill_path(c, path, ctm)?;
            }
            c.restore_graphics_state()?;
        }
        Ok(())
    }

    fn fill_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<bool> {
        let Some(fill) = path.fill() else {
            return Ok(false);
        };
        if !self.set_paint(c, fill.paint(), f64::from(fill.opacity().get()), path, ctm, false)? {
            return Ok(false);
        }
        emit_path(c, path.data(), Transform::identity())?;
        c.fill_path_with_rule(fill_rule(fill.rule()))?;
        Ok(true)
    }

    fn stroke_path(&mut self, c: &mut PageContext<'_, '_, '_>, path: &usvg::Path, ctm: Transform) -> Result<bool> {
        let Some(stroke) = path.stroke() else {
            return Ok(false);
        };
        if !self.set_paint(c, stroke.paint(), f64::from(stroke.opacity().get()), path, ctm, true)? {
            return Ok(false);
        }
        stroke_style(c, stroke)?;
        emit_path(c, path.data(), Transform::identity())?;
        c.stroke_path()?;
        Ok(true)
    }

    /// Makes the fill (or stroke) color of the graphics state the paint. Returns false if
    /// there is nothing to paint.
    fn set_paint(
        &mut self,
        c: &mut PageContext<'_, '_, '_>,
        paint: &usvg::Paint,
        opacity: f64,
        path: &usvg::Path,
        ctm: Transform,
        stroke: bool,
    ) -> Result<bool> {
        if opacity <= 0.0 {
            return Ok(false);
        }
        if opacity < 1.0 {
            if stroke {
                c.set_stroke_alpha(opacity)?;
            } else {
                c.set_fill_alpha(opacity)?;
            }
        }
        match paint {
            usvg::Paint::Color(color) => {
                let Rgb(r, g, b) = rgb(*color);
                if stroke {
                    c.device_rgb_stroke_color(r, g, b)?;
                } else {
                    c.device_rgb_fill_color(r, g, b)?;
                }
            }
            usvg::Paint::LinearGradient(gradient) => {
                let Some(pattern) = self.linear_gradient(gradient, path, ctm) else {
                    return Ok(false);
                };
                let key = c.add_shading_pattern(pattern);
                if stroke {
                    c.set_stroke_pattern(&key)?;
                } else {
                    c.set_fill_pattern(&key)?;
                }
            }
            usvg::Paint::RadialGradient(gradient) => {
                let Some(pattern) = self.radial_gradient(gradient, path, ctm) else {
                    return Ok(false);
                };
                let key = c.add_shading_pattern(pattern);
                if stroke {
                    c.set_stroke_pattern(&key)?;
                } else {
                    c.set_fill_pattern(&key)?;
                }
            }
            usvg::Paint::Pattern(pattern) => {
                let tiling = self.tiling_pattern(c, pattern, ctm)?;
                let key = c.add_tiling_pattern(tiling);
                if stroke {
                    c.set_stroke_pattern(&key)?;
                } else {
                    c.set_fill_pattern(&key)?;
                }
            }
        }
        Ok(true)
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
            .map(|stop| (f64::from(stop.offset().get()), rgb(stop.color())))
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
        ctm: Transform,
    ) -> Option<ShadingPattern> {
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
        Some(ShadingPattern::new(shading).with_matrix(matrix(ctm.pre_concat(base.transform()))))
    }

    fn radial_gradient(
        &mut self,
        gradient: &usvg::RadialGradient,
        path: &usvg::Path,
        ctm: Transform,
    ) -> Option<ShadingPattern> {
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
        Some(ShadingPattern::new(shading).with_matrix(matrix(ctm.pre_concat(base.transform()))))
    }

    /// A shading that has the color of the last stop everywhere.
    fn last_stop_pattern(&mut self, base: &usvg::BaseGradient) -> Option<ShadingPattern> {
        let color = rgb(base.stops().last()?.color());
        let function = Gradient::new(&[ColorStop::new(0.0, color), ColorStop::new(1.0, color)])
            .ok()?
            .extended();
        let shading = AxialShading::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0), &function);
        Some(ShadingPattern::new(shading))
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
        let filter = self.filter;
        c.pdf_context.write_tiling_pattern(
            [Pt(0.0), Pt(0.0), Pt(width), Pt(height)],
            Pt(width),
            Pt(height),
            matrix(t),
            filter,
            |cell| self.draw_group(cell, pattern.root(), Transform::identity()),
        )
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
                c.save_graphics_state()?;
                c.rect(Pt(0.0), Pt(0.0), Pt(width), Pt(height))?;
                c.clip(FillRule::NonZero)?;
                c.end_path()?;
                self.draw_group(c, tree.root(), ctm)?;
                return c.restore_graphics_state();
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
        c.save_graphics_state()?;
        // the unit square of an image has its top at y = 1, SVG has the top at y = 0
        concat(c, Transform::from_row(width as f32, 0.0, 0.0, -(height as f32), 0.0, height as f32))?;
        c.draw_xobject(&name)?;
        c.restore_graphics_state()
    }
}
