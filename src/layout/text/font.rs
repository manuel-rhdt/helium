pub use super::{
    HarfbuzzFont,
    builtin::{BuiltInFont, FontUnit},
};

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, PixelLength,
};

use font_kit;
use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::{handle::Handle, source::SystemSource};

use harfbuzz_rs::{Blob, Face as HbFace, Font as HbFont};
use parking_lot::{Mutex, MutexGuard, RwLock};
use std::collections::BTreeMap;
use std::sync::Arc;

fn scale_ext(upem: u32, metric: i32) -> FontUnit {
    let upem = upem as f64;
    FontUnit(metric as f64 * 1000f64 / upem)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct GlyphIndex(pub u32);

#[derive(Debug)]
pub struct Font {
    pub(crate) upem: u32,
    pub(crate) glyph_advances: Mutex<BTreeMap<GlyphIndex, FontUnit>>,
    pub(crate) to_unicode: Mutex<BTreeMap<GlyphIndex, String>>,
    pub(crate) hb_font: HarfbuzzFont,
    pub(crate) postscript_name: String,
    pub(crate) font_data: FontData,
    /// Fonts that provide the characters this font has no glyph for, in order of preference.
    ///
    /// They are filled in during box construction (see `fallback::FontResolver`), when the text
    /// that is rendered with this font is known, and used by `shape`. Every font in here is also
    /// an entry of the `FontMap`, so that it is embedded into the PDF.
    pub(crate) fallbacks: RwLock<Vec<Arc<Font>>>,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Font {
            upem: self.upem,
            glyph_advances: Mutex::new(self.glyph_advances.lock().clone()),
            to_unicode: Mutex::new(self.to_unicode.lock().clone()),
            font_data: self.font_data.clone(),
            hb_font: self.hb_font.clone(),
            postscript_name: self.postscript_name.clone(),
            fallbacks: RwLock::new(self.fallbacks.read().clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FontData {
    BuiltIn(BuiltInFont),
    External(font_kit::handle::Handle),
}

impl Font {
    pub(crate) fn built_in(val: BuiltInFont) -> Self {
        Font {
            upem: 1000,
            font_data: FontData::BuiltIn(val),
            postscript_name: val.metrics().font_name.to_owned(),
            ..Default::default()
        }
    }

    /// Loads a system font, or returns `None` if the font file cannot be read.
    pub(crate) fn external(handle: font_kit::handle::Handle) -> Option<Self> {
        let loaded = handle.load().ok()?;

        let upem = loaded.metrics().units_per_em;
        let font_bytes = loaded.copy_font_data()?;
        let index = match handle {
            Handle::Memory { font_index, .. } => font_index,
            Handle::Path { font_index, .. } => font_index,
        };
        let name = loaded
            .postscript_name()
            .unwrap_or_else(|| loaded.full_name());
        let blob = Blob::with_bytes_owned(font_bytes, |b| &*b);
        let face = HbFace::new(blob, index);
        let hb_font = HbFont::new(face).into();
        Some(Font {
            upem,
            font_data: FontData::External(handle),
            hb_font,
            postscript_name: name,
            ..Default::default()
        })
    }

    /// Whether this font has a glyph for `ch`.
    ///
    /// Built-in fonts are written with `WinAnsiEncoding`, so they only cover the characters of
    /// that encoding.
    pub fn covers(&self, ch: char) -> bool {
        match &self.font_data {
            FontData::BuiltIn(font) => builtin_glyph(*font, ch).is_some(),
            FontData::External(_) => self.hb_font.get_nominal_glyph(ch).is_some(),
        }
    }

    /// A snapshot of the fallback fonts of this font.
    pub fn fallbacks(&self) -> Vec<Arc<Font>> {
        self.fallbacks.read().clone()
    }

    /// Appends `fallback` to the fallback fonts unless it is this font or already one of them.
    pub(crate) fn add_fallback(&self, fallback: Arc<Font>) {
        if fallback.postscript_name == self.postscript_name {
            return;
        }
        let mut fallbacks = self.fallbacks.write();
        if !fallbacks
            .iter()
            .any(|font| font.postscript_name == fallback.postscript_name)
        {
            fallbacks.push(fallback);
        }
    }

    pub fn postscript_name(&self) -> String {
        self.postscript_name.clone()
    }

    pub fn ascent(&self) -> FontUnit {
        match &self.font_data {
            FontData::BuiltIn(font) => font.metrics().ascender,
            FontData::External(_font) => scale_ext(
                self.upem,
                self.hb_font
                    .get_font_h_extents()
                    .map(|f| f.ascender)
                    .unwrap_or_default(),
            ),
        }
    }

    pub fn descent(&self) -> FontUnit {
        match &self.font_data {
            FontData::BuiltIn(font) => font.metrics().descender,
            FontData::External(_font) => scale_ext(
                self.upem,
                self.hb_font
                    .get_font_h_extents()
                    .map(|f| f.descender)
                    .unwrap_or_default(),
            ),
        }
    }

    pub fn advance(&self, glyph: GlyphIndex) -> FontUnit {
        *self
            .glyph_advances
            .lock()
            .entry(glyph)
            .or_insert_with(|| match &self.font_data {
                FontData::BuiltIn(font) => {
                    font.metrics()
                        .char_metrics
                        .get(glyph.0 as usize)
                        .unwrap()
                        .advance_width
                }
                FontData::External(_) => {
                    scale_ext(self.upem, self.hb_font.get_glyph_h_advance(glyph.0))
                }
            })
    }

    pub fn scale_to_px(&self, val: FontUnit, font_size: PixelLength) -> PixelLength {
        val.0 as f32 * font_size / 1000.0f32
    }

    pub fn font_height(&self) -> FontUnit {
        self.ascent() - self.descent()
    }
}

impl Default for Font {
    fn default() -> Self {
        Font {
            upem: 1000,
            glyph_advances: Mutex::new(BTreeMap::new()),
            to_unicode: Default::default(),
            font_data: FontData::BuiltIn(BuiltInFont::Helvetica),
            hb_font: HarfbuzzFont::default(),
            postscript_name: format!("DEFAULT_FONT"),
            fallbacks: Default::default(),
        }
    }
}

impl<'a> From<&'a ComputedValues> for Properties {
    fn from(computed_values: &'a ComputedValues) -> Properties {
        let mut props = Properties::new();
        props
            .style(computed_values.font.font_style.clone().into())
            .weight(computed_values.font.font_weight.clone().into());
        props
    }
}

pub fn match_font(style: &ComputedValues) -> Font {
    let families = style
        .font
        .font_family
        .iter()
        .map(|fam| FamilyName::from(fam.clone()))
        .collect::<Vec<_>>();
    SystemSource::new()
        .select_best_match(&families, &style.into())
        .ok()
        .and_then(Font::external)
        .unwrap_or_else(|| Font::built_in(BuiltInFont::Helvetica))
}

/// The glyph of `ch` in a built-in font, if the character can be written with
/// `WinAnsiEncoding` (the encoding the built-in fonts are written with).
pub(crate) fn builtin_glyph(font: BuiltInFont, ch: char) -> Option<GlyphIndex> {
    use lemon_pdf::font::encoding::PredefinedEncoding;
    let metrics = font.metrics();
    let index = (metrics.unicode_mapping)(ch)?;
    let name = metrics.char_metrics.get(index as usize)?.name;
    PredefinedEncoding::WinAnsiEncoding.encode(name)?;
    Some(GlyphIndex(index as u32))
}

pub fn match_font_builtin(style: &ComputedValues) -> Option<Font> {
    style
        .font
        .font_family
        .iter()
        .next()
        .and_then(|family| match family {
            FontFamily::Serif => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::TimesRoman))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::TimesBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::TimesItalic))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::TimesBoldItalic))
                }
            },
            FontFamily::SansSerif => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::Helvetica))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::HelveticaBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::HelveticaOblique))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::HelveticaBoldOblique))
                }
            },
            FontFamily::Monospace => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::Courier))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::CourierBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::built_in(BuiltInFont::CourierOblique))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::built_in(BuiltInFont::CourierBoldOblique))
                }
            },
            FontFamily::Named(_) => None,
        })
}

#[cfg(test)]
mod tests {
    use super::{BuiltInFont, ComputedValues, FontData, match_font};
    use crate::style::properties::specified::FontFamily;

    #[test]
    fn missing_system_font_falls_back_to_builtin_helvetica() {
        let mut style = ComputedValues::default();
        std::sync::Arc::make_mut(&mut style.font).font_family =
            vec![FontFamily::Named("Helium Missing Font".to_owned())];

        let font = match_font(&style);

        assert_eq!(font.postscript_name(), "Helvetica");
        assert!(matches!(
            &font.font_data,
            FontData::BuiltIn(BuiltInFont::Helvetica)
        ));
    }
}

// Glyph-metrics backend for the text layout iterator; no constructor is wired up yet.
#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct FontInstance<'font, Font: 'font> {
    pub font: Font,
    pub glyph_map: MutexGuard<'font, BTreeMap<GlyphIndex, FontUnit>>,
    pub size: PixelLength,
}

pub trait GlyphMetricsProvide {
    fn char_to_glyph_index(&self, ch: char) -> GlyphIndex;
    fn glyph_advance(&mut self, glyph: GlyphIndex) -> PixelLength;
}

impl<'font> GlyphMetricsProvide for FontInstance<'font, &'font font_kit::font::Font> {
    fn char_to_glyph_index(&self, ch: char) -> GlyphIndex {
        GlyphIndex(self.font.glyph_for_char(ch).unwrap_or_default())
    }
    fn glyph_advance(&mut self, glyph: GlyphIndex) -> PixelLength {
        let font = self.font;
        let unscaled_advance = *self.glyph_map.entry(glyph).or_insert_with(|| {
            FontUnit(
                font.advance(glyph.0).unwrap().x() as f64 * 1000f64
                    / font.metrics().units_per_em as f64,
            )
        });
        self.size * (unscaled_advance.0 / 1000f64) as f32
    }
}

impl super::FontLike for Font {}

impl From<FontFamily> for FamilyName {
    fn from(family: FontFamily) -> FamilyName {
        match family {
            FontFamily::Monospace => FamilyName::Monospace,
            FontFamily::SansSerif => FamilyName::SansSerif,
            FontFamily::Serif => FamilyName::Serif,
            FontFamily::Named(name) => FamilyName::Title(name),
        }
    }
}

impl From<FontStyle> for font_kit::properties::Style {
    fn from(style: FontStyle) -> font_kit::properties::Style {
        match style {
            FontStyle::Normal => font_kit::properties::Style::Normal,
            FontStyle::Italic => font_kit::properties::Style::Italic,
            FontStyle::Oblique => font_kit::properties::Style::Oblique,
        }
    }
}

impl From<FontWeight> for font_kit::properties::Weight {
    fn from(weight: FontWeight) -> font_kit::properties::Weight {
        match weight {
            FontWeight::Normal => font_kit::properties::Weight::NORMAL,
            FontWeight::Bold => font_kit::properties::Weight::BOLD,
        }
    }
}
