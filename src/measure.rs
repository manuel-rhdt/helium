//! `Engine::measure`: the size of a line of text (or of a formula) as the backend will draw it,
//! with what a plotting library needs to align it: the baseline, the line box, the advance
//! width and the bounds of the ink.
//!
//! A probe is laid out as one line with `white-space: pre`, `line-height: normal`, no margin and
//! the width of its content (a float). A batch is laid out as one document, the probes stacked
//! one below the other, so that the fixed cost of a layout (the style sheet, the world) is paid
//! once per batch.
//!
//! All lengths are CSS pixels measured from the left edge of the line box and from the first
//! baseline; `ink.top` and `ink.bottom` are distances from the baseline downwards, so `top` is
//! negative for what is above it.

use std::collections::HashMap;

use specs::{Entity, Join, WorldExt};

use crate::engine::{Document, DocumentKind, Engine, Error, LayoutOptions, Result};
use crate::layout::FontMap;
use crate::layout::components::{
    GraphicsOp, LayoutEntity, Paragraph, Position, TextOperator, VectorGraphics, NO_CLUSTER,
};
use crate::layout::inline::InlineFormattingContext;
use crate::layout::construct::AssignLayoutEntity;
use crate::parser::ParsedData;
use crate::tree::{DomRef, TreeLinked};

/// What to measure: `markup` (an XHTML or MathML fragment, well-formed) or `text` (plain), set
/// with the declarations of `css` (`font-family`, `font-size`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Probe {
    pub markup: Option<String>,
    pub text: Option<String>,
    pub css: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ink {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    /// The advance width of the line, which is not the width of the ink.
    pub width: f32,
    /// The line box above and below the baseline, from the layout of the line.
    pub ascent: f32,
    pub descent: f32,
    pub height: f32,
    /// What is painted, or `None` if nothing is.
    pub ink: Option<Ink>,
    /// The code points that no font had, as `U+XXXX`.
    pub missing_glyphs: Vec<String>,
}

const CACHE_SIZE: usize = 4096;
/// Probes per document: the stacked probes have to be on one page, whose coordinates are `f32`.
const BATCH: usize = 200;
const PAGE: (f32, f32) = (100_000.0, 100_000.0);

/// The results of the probes that were measured, by probe, evicting the least recently used.
#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<Probe, (Metrics, u64)>,
    clock: u64,
}

impl Cache {
    fn get(&mut self, probe: &Probe) -> Option<Metrics> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(probe).map(|(metrics, used)| {
            *used = clock;
            metrics.clone()
        })
    }

    fn insert(&mut self, probe: Probe, metrics: Metrics) {
        self.clock += 1;
        if self.entries.len() >= CACHE_SIZE {
            // the oldest quarter goes at once, so that this is not done for every insert
            let mut used: Vec<u64> = self.entries.values().map(|(_, used)| *used).collect();
            used.sort_unstable();
            let cutoff = used[used.len() / 4];
            self.entries.retain(|_, (_, used)| *used > cutoff);
        }
        self.entries.insert(probe, (metrics, self.clock));
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The element for a probe, or what is wrong with it.
fn probe_element(index: usize, probe: &Probe) -> Result<String> {
    let fragment = match (&probe.markup, &probe.text) {
        (Some(markup), None) => markup.clone(),
        (None, Some(text)) => escape(text),
        _ => return Err(Error::BadRequest("a probe has exactly one of `markup` and `text`".into())),
    };
    // (the rules of a style sheet are not measured: one in a probe applies to some of the probes only,
    // and not to the ones that are cached, or in other batches)
    if fragment.to_ascii_lowercase().contains("<style") {
        return Err(Error::BadRequest(
            "a probe has no `<style>`: send style sheets (`@font-face` rules) once with `set_stylesheet`".into(),
        ));
    }
    if fragment.contains(['\n', '\r', '\u{2028}', '\u{2029}']) || fragment.to_ascii_lowercase().contains("<br") {
        return Err(Error::BadRequest("a probe is one line: it has a line break".into()));
    }
    // (the declarations cannot leave the attribute, and the ones that make a probe a probe come last)
    Ok(format!(
        "<div id=\"probe{index}\" style=\"{}; display: block; width: max-content; height: auto; \
         margin: 0 0 4px 0; padding: 0; border: 0; white-space: pre; line-height: normal\">{fragment}</div>",
        escape(&probe.css)
    ))
}

fn element_ids(dom: &DomRef, wanted: &HashMap<String, usize>, found: &mut HashMap<usize, Entity>) {
    if let ParsedData::Element(element) = &dom.html_data {
        let id = element
            .attributes
            .borrow()
            .iter()
            .find(|(name, _)| &*name.local == "id")
            .map(|(_, attribute)| attribute.value.to_string());
        if let (Some(index), Some(LayoutEntity(entity))) = (id.and_then(|id| wanted.get(&id).copied()), dom.layout_entity())
        {
            found.insert(index, entity);
        }
    }
    for child in TreeLinked::children(dom) {
        element_ids(&child, wanted, found);
    }
}

impl Engine {
    /// Measures the probes. One probe that cannot be measured is an error of its entry only.
    pub fn measure(&self, probes: &[Probe]) -> Vec<Result<Metrics>> {
        let mut results: Vec<Option<Result<Metrics>>> = vec![None; probes.len()];
        let mut todo: Vec<usize> = Vec::new();
        {
            let mut cache = self.measure_cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            for (index, probe) in probes.iter().enumerate() {
                match cache.get(probe) {
                    Some(metrics) => results[index] = Some(Ok(metrics)),
                    None => todo.push(index),
                }
            }
        }
        // the probes that are not well-formed are errors of their own
        let mut elements: Vec<(usize, String)> = Vec::new();
        for index in todo {
            let checked = probe_element(0, &probes[index]).and_then(|element| {
                let wrapped = format!("<div xmlns=\"http://www.w3.org/1999/xhtml\">{}</div>", &element[element.find('>').unwrap() + 1..element.len() - 6]);
                crate::parser::generate_render_tree_xml(&wrapped).map(|_| ()).map_err(Error::Parse)
            });
            match checked {
                Ok(()) => elements.push((index, String::new())),
                Err(error) => results[index] = Some(Err(error)),
            }
        }
        for chunk in elements.chunks(BATCH) {
            let indices: Vec<usize> = chunk.iter().map(|(index, _)| *index).collect();
            let measured = self.measure_batch(probes, &indices).unwrap_or_else(|_| {
                // (a batch that fails is measured probe by probe to find out which one it was)
                indices
                    .iter()
                    .map(|&index| {
                        self.measure_batch(probes, &[index]).and_then(|mut one| one.pop().expect("one result"))
                    })
                    .collect()
            });
            let mut cache = self.measure_cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            for (&index, metrics) in indices.iter().zip(measured) {
                if let Ok(metrics) = &metrics {
                    cache.insert(probes[index].clone(), metrics.clone());
                }
                results[index] = Some(metrics);
            }
        }
        results.into_iter().map(|result| result.expect("every probe has a result")).collect()
    }

    /// Lays the probes out as one document. The error is for the document; a probe that does not
    /// have a box is an error of its own.
    fn measure_batch(&self, probes: &[Probe], indices: &[usize]) -> Result<Vec<Result<Metrics>>> {
        let mut body = String::new();
        let mut wanted = HashMap::new();
        for (slot, &index) in indices.iter().enumerate() {
            body.push_str(&probe_element(slot, &probes[index])?);
            wanted.insert(format!("probe{slot}"), slot);
        }
        let source = format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>measure</title>\
             <style>@page {{ size: 100000px 100000px; margin: 0 }} html, body {{ margin: 0; padding: 0 }}</style></head><body>{body}</body></html>"
        );
        let document = Document::new(source, DocumentKind::Xml);
        let layout = self.layout(&document, &LayoutOptions { crop: false, page: Some(PAGE) })?;
        let mut found = HashMap::new();
        element_ids(layout.dom(), &wanted, &mut found);
        let world = layout.context();

        let positions = world.read_storage::<Position>();
        let paragraphs = world.read_storage::<Paragraph>();
        let ifcs = world.read_storage::<InlineFormattingContext>();
        let graphics = world.read_storage::<VectorGraphics>();
        let fonts = world.read_resource::<FontMap>();
        let mut faces = Faces::default();

        // the lines of the probes, with the baseline of each
        let lines: Vec<(f32, f32, f32, f32, f32)> = ifcs
            .join()
            .flat_map(|ifc| ifc.lines.iter())
            .filter_map(|line| positions.get(line.entity).map(|pos| (pos.x.0, pos.y.0, pos.width.0, line.height.0, line.baseline.0)))
            .collect();
        // What is painted, in pieces that each belong to the probe whose box the baseline is in:
        // a paragraph can run on from the line of one probe to the line of the next.
        let mut pieces: Vec<Piece> = Vec::new();
        for paragraph in (&paragraphs).join() {
            paragraph_pieces(paragraph, &fonts, &mut faces, &mut pieces);
        }
        for graphic in (&graphics).join() {
            for op in &graphic.ops {
                let GraphicsOp::Line { from_x, from_y, to_x, to_y, thickness } = op;
                let mut bounds = Bounds::default();
                bounds.line((from_x.0, from_y.0), (to_x.0, to_y.0), thickness.0);
                pieces.push(Piece { at: (from_x.0, from_y.0), bounds, missing: vec![] });
            }
        }

        let mut out = Vec::new();
        for slot in 0..indices.len() {
            let Some(&entity) = found.get(&slot) else {
                out.push(Err(Error::Layout("the probe has no box".into())));
                continue;
            };
            let Some(pos) = positions.get(entity) else {
                out.push(Err(Error::Layout("the probe has no box".into())));
                continue;
            };
            if pos.page != 0 {
                return Err(Error::Layout("the probes do not fit on one page".into()));
            }
            let (left, top, bottom) = (pos.x.0, pos.y.0, pos.y.0 + pos.height.0);
            let inside = |y: f32| y >= top - 1.0 && y <= bottom + 1.0;
            // the line of the probe: the widest of the lines in its box
            let line = lines
                .iter()
                .filter(|line| line.0 >= left - 0.5 && inside(line.1) && line.1 + line.3 <= bottom + 1.0)
                .max_by(|a, b| a.2.total_cmp(&b.2));
            let Some(&(line_x, line_y, _, line_height, baseline)) = line else {
                // nothing was set (an empty text)
                out.push(Ok(Metrics { width: pos.width.0, ascent: 0.0, descent: 0.0, height: 0.0, ink: None, missing_glyphs: vec![] }));
                continue;
            };
            let origin = (line_x, line_y + baseline);
            let mut bounds = Bounds::default();
            let mut missing = std::collections::BTreeSet::new();
            for piece in pieces.iter().filter(|piece| inside(piece.at.1) && piece.at.0 >= left - 0.5) {
                bounds.merge(&piece.bounds);
                missing.extend(piece.missing.iter().copied());
            }
            out.push(Ok(Metrics {
                width: pos.width.0,
                ascent: baseline,
                descent: line_height - baseline,
                height: line_height,
                ink: bounds.get().map(|(l, t, r, b)| Ink {
                    left: l - origin.0,
                    top: t - origin.1,
                    right: r - origin.0,
                    bottom: b - origin.1,
                }),
                missing_glyphs: missing.into_iter().map(|ch| format!("U+{:04X}", ch as u32)).collect(),
            }));
        }
        Ok(out)
    }
}

/// A rectangle that grows to hold points: left, top, right, bottom.
#[derive(Default)]
struct Bounds(Option<(f32, f32, f32, f32)>);

impl Bounds {
    fn add(&mut self, x: f32, y: f32) {
        self.0 = Some(match self.0 {
            None => (x, y, x, y),
            Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
        });
    }

    /// A stroked segment with butt caps.
    fn line(&mut self, from: (f32, f32), to: (f32, f32), thickness: f32) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = if length > 0.0 { (-dy / length * thickness / 2.0, dx / length * thickness / 2.0) } else { (0.0, thickness / 2.0) };
        for (x, y) in [from, to] {
            self.add(x + nx, y + ny);
            self.add(x - nx, y - ny);
        }
    }

    fn merge(&mut self, other: &Bounds) {
        if let Some((l, t, r, b)) = other.0 {
            self.add(l, t);
            self.add(r, b);
        }
    }

    fn get(&self) -> Option<(f32, f32, f32, f32)> {
        self.0
    }
}

/// The parsed fonts, by the name that the paragraphs use.
#[derive(Default)]
struct Faces<'a>(HashMap<String, FaceBounds<'a>>);

/// The outlines of a font and the box of its glyphs, in font units.
struct FaceBounds<'a> {
    outlines: read_fonts::model::glyph::outline::OutlineGlyphs<'a>,
    /// x_min, y_min, x_max, y_max of the `head` table.
    global: [f32; 4],
    upem: f32,
}

impl FaceBounds<'_> {
    /// The box of the outline of a glyph as x_min, y_min, x_max, y_max, if it has an outline.
    fn glyph_bounding_box(&self, glyph: u32) -> Option<[f32; 4]> {
        let extents = self.outlines.get(read_fonts::types::GlyphId::new(glyph))?.extents().ok()?;
        // an empty glyph (a space) has no box
        if extents.width == 0.0 && extents.height == 0.0 {
            return None;
        }
        let top = extents.y_bearing;
        Some([extents.x_bearing, top - extents.height, extents.x_bearing + extents.width, top])
    }
}

impl<'a> Faces<'a> {
    fn get(&mut self, name: &str, fonts: &'a FontMap) -> Option<&FaceBounds<'a>> {
        if !self.0.contains_key(name) {
            let font = fonts.get(name)?;
            let bounds = font.face.metrics().bounds;
            self.0.insert(
                name.to_owned(),
                FaceBounds {
                    outlines: font.face.outline_glyphs(),
                    global: [bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max].map(|v| v.to_f32()),
                    upem: font.upem as f32,
                },
            );
        }
        self.0.get(name)
    }
}

/// What a run of text or a rule painted: where it starts (the origin of its first glyph, on the
/// baseline), the bounds of its ink, and the characters that it drew with the `.notdef` glyph.
struct Piece {
    at: (f32, f32),
    bounds: Bounds,
    missing: Vec<char>,
}

/// The bounds of the glyphs of a paragraph, run by run, as the text operators place them (the
/// same as the PDF operators do).
fn paragraph_pieces<'a>(paragraph: &Paragraph, fonts: &'a FontMap, faces: &mut Faces<'a>, out: &mut Vec<Piece>) {
    // characters that had no glyph: drawn with glyph 0
    let mut notdef: HashMap<usize, char> = HashMap::new();
    for piece in &paragraph.ops.pieces {
        let text = paragraph.ops.piece_text(piece);
        for index in piece.glyphs.start as usize..piece.glyphs.end as usize {
            let glyph = &paragraph.ops.glyphs[index];
            if glyph.glyph.0 == 0 && glyph.cluster != NO_CLUSTER {
                if let Some(ch) = text.get(glyph.cluster as usize..).and_then(|rest| rest.chars().next()) {
                    notdef.insert(index, ch);
                }
            }
        }
    }
    // the text matrix: x' = a x + c y + e, and y' (downwards) = f - (b x + d y), with the text
    // space in y-up pixels; `pen` is the position along the line in text space
    let mut matrix = [1.0f32, 0.0, 0.0, 1.0, paragraph.start_pos.0.0, paragraph.start_pos.1.0];
    let mut pen = 0.0f32;
    let mut rise = 0.0f32;
    let mut font: Option<(&str, f32)> = None;
    let glyphs = &paragraph.ops.glyphs;
    for op in paragraph.ops.iter() {
        match op {
            TextOperator::SetFont(name, size) => font = Some((name.as_str(), size.0)),
            TextOperator::SetRise(value) => rise = value.0,
            TextOperator::SetTransform(m) => {
                matrix = [m.a, m.b, m.c, m.d, m.e.0, m.f.0];
                pen = 0.0;
            }
            TextOperator::Embed { x, y, .. } => {
                matrix = [1.0, 0.0, 0.0, 1.0, x.0, y.0];
                pen = 0.0;
            }
            TextOperator::NewLine { x_offset, leading } => {
                let (tx, ty) = (x_offset.0, -leading.0);
                matrix[4] += matrix[0] * tx + matrix[2] * ty;
                matrix[5] -= matrix[1] * tx + matrix[3] * ty;
                pen = 0.0;
            }
            TextOperator::KernedGlyphs(range) => {
                let Some((name, size)) = font else { continue };
                let Some(face) = faces.get(name, fonts) else { continue };
                let scale = size / face.upem;
                let advance_scale = size / 1000.0;
                let font_data = &fonts[name];
                let [a, b, c, d, e, f] = matrix;
                let place = |tx: f32, ty: f32| (e + a * tx + c * ty, f - (b * tx + d * ty));
                let mut piece = Piece { at: place(pen, rise), bounds: Bounds::default(), missing: vec![] };
                for index in range.start as usize..range.end as usize {
                    let glyph = &glyphs[index];
                    if glyph.kern != 0 {
                        pen -= glyph.kern as f32 / 10.0 * advance_scale;
                    }
                    if let Some([x_min, y_min, x_max, y_max]) = face.glyph_bounding_box(glyph.glyph.0) {
                        for (gx, gy) in [(x_min, y_min), (x_max, y_min), (x_min, y_max), (x_max, y_max)] {
                            let (x, y) = place(pen + gx * scale, rise + gy * scale);
                            piece.bounds.add(x, y);
                        }
                    }
                    if let Some(&ch) = notdef.get(&index) {
                        piece.missing.push(ch);
                    }
                    pen += font_data.advance(glyph.glyph).0 as f32 * advance_scale;
                }
                if range.trailing_kern != 0 {
                    pen -= range.trailing_kern as f32 / 10.0 * advance_scale;
                }
                out.push(piece);
            }
            _ => {}
        }
    }
    // the colour glyphs are empty in the text; their box is the one of the font
    for placed in &paragraph.color_glyphs {
        if let Some(face) = faces.get(&placed.font, fonts) {
            let [x_min, y_min, x_max, y_max] = face.global;
            let scale = placed.size.0 / face.upem;
            let mut bounds = Bounds::default();
            for (gx, gy) in [(x_min, y_min), (x_max, y_max)] {
                bounds.add(placed.x.0 + gx * scale, placed.baseline.0 - gy * scale);
            }
            out.push(Piece { at: (placed.x.0, placed.baseline.0), bounds, missing: vec![] });
        }
    }
}
