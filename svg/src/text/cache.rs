// Copyright 2026 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The fonts that a conversion uses: their data, and the outlines of their glyphs.
//!
//! Upstream reads fonts with `skrifa` and also supports color glyphs (COLR, SVG, bitmaps) and
//! variable fonts. This fork reads the outlines and the metrics with `read-fonts`, and shaping
//! is up to the caller ([`TextShaper`](super::TextShaper)). Glyphs have only their outline.

use std::collections::HashMap;
use std::sync::Arc;

use fontdb::{Database, ID};

use super::layout::ResolvedFont;
use super::TextShaper;
use crate::{Font, FontOpticalSizing, FontVariation, GlyphId};

pub(crate) struct FontCache {
    /// This fontdb is initialized from [`Options::fontdb`](crate::Options::fontdb) and then
    /// populated over the course of conversion.
    pub(crate) fontdb: Arc<Database>,
    pub(crate) resolved: HashMap<Font, Arc<ResolvedFont>>,
    pub(crate) shaper: Option<Arc<dyn TextShaper>>,
    /// The metrics of the faces that glyphs come from, by ID (see [`Self::resolved_by_id`]).
    by_id: HashMap<ID, Option<Arc<ResolvedFont>>>,
    // `None` marks faces that failed to load, so they are not read again.
    loaded: HashMap<ID, Option<LoadedFont>>,
    outlines: HashMap<(ID, GlyphId), Option<tiny_skia_path::Path>>,
}

pub(crate) struct LoadedFont {
    pub(crate) data: Arc<Vec<u8>>,
    pub(crate) index: u32,
    pub(crate) face: read_fonts::model::Font,
}

impl LoadedFont {
    fn new(id: ID, fontdb: &Database) -> Option<Self> {
        let (data, index) = fontdb.with_face_data(id, |data, index| (data.to_vec(), index))?;
        let data = Arc::new(data);
        // check that the face can be read
        let face = read_fonts::model::Font::new(
            data.clone() as Arc<dyn AsRef<[u8]> + Send + Sync>,
            index,
        )?;
        Some(LoadedFont { data, index, face })
    }
}

impl FontCache {
    /// The metrics of the face `id`, read once.
    pub(crate) fn resolved_by_id(&mut self, id: ID) -> Option<Arc<ResolvedFont>> {
        if let Some(font) = self.by_id.get(&id) {
            return font.clone();
        }
        let font = super::layout::resolve_font(id, &[], self).map(Arc::new);
        self.by_id.insert(id, font.clone());
        font
    }

    pub(crate) fn new(fontdb: Arc<Database>, shaper: Option<Arc<dyn TextShaper>>) -> Self {
        Self {
            fontdb,
            resolved: HashMap::new(),
            shaper,
            by_id: HashMap::new(),
            loaded: HashMap::new(),
            outlines: HashMap::new(),
        }
    }

    pub(crate) fn load(&mut self, id: ID) -> Option<&mut LoadedFont> {
        let fontdb = &self.fontdb;
        self.loaded
            .entry(id)
            .or_insert_with(|| LoadedFont::new(id, fontdb))
            .as_mut()
    }

    /// The outline of a glyph in font units, y up.
    pub(crate) fn outline(
        &mut self,
        id: ID,
        glyph: GlyphId,
        _variations: impl IntoIterator<Item = FontVariation>,
    ) -> Option<tiny_skia_path::Path> {
        if let Some(outline) = self.outlines.get(&(id, glyph)) {
            return outline.clone();
        }
        let outline = self
            .load(id)
            .and_then(|font| outline(&font.face, glyph));
        self.outlines.insert((id, glyph), outline.clone());
        outline
    }
}

/// Adds the `opsz` value that `font-optical-sizing: auto` implies, like browsers do.
///
/// It comes first, so that an `opsz` in `variations` overrides it. This fork does not read
/// variable fonts: the values are not used.
pub(crate) fn with_optical_sizing(
    variations: &[FontVariation],
    optical_sizing: FontOpticalSizing,
    font_size: f32,
) -> impl Iterator<Item = FontVariation> + Clone + '_ {
    let auto_opsz = (optical_sizing == FontOpticalSizing::Auto)
        .then(|| FontVariation::new(*b"opsz", font_size));
    auto_opsz.into_iter().chain(variations.iter().copied())
}

struct Builder(tiny_skia_path::PathBuilder);

impl read_fonts::model::glyph::outline::PathSink for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.0.close();
    }
}

fn outline(face: &read_fonts::model::Font, glyph: GlyphId) -> Option<tiny_skia_path::Path> {
    use read_fonts::model::glyph::outline::{DrawMemory, DrawOptions};
    let mut builder = Builder(tiny_skia_path::PathBuilder::new());
    face.outline_glyphs()
        .get(read_fonts::types::GlyphId::new(glyph.0))?
        .draw(&DrawOptions::new(), DrawMemory::Scratch, &mut builder)
        .ok()?;
    builder.0.finish()
}
