pub use super::builtin::{BuiltInFont, FontUnit};

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, PixelLength,
};

use font_kit;
use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

fn scale_ext(font: &font_kit::font::Font, metric: f32) -> FontUnit {
    let upem = font.metrics().units_per_em as f64;
    FontUnit(metric as f64 * 1000f64 / upem)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct GlyphIndex(pub u32);

#[derive(Debug)]
pub struct Font {
    pub(crate) glyph_advances: Mutex<BTreeMap<GlyphIndex, FontUnit>>,
    pub(crate) to_unicode: Mutex<BTreeMap<GlyphIndex, String>>,
    pub(crate) font_data: FontData,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Font {
            glyph_advances: Mutex::new(self.glyph_advances.lock().unwrap().clone()),
            to_unicode: Mutex::new(self.to_unicode.lock().unwrap().clone()),
            font_data: self.font_data.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FontData {
    BuiltIn(BuiltInFont),
    External(font_kit::font::Font),
}

impl Font {
    fn built_in(val: BuiltInFont) -> Self {
        Font {
            font_data: FontData::BuiltIn(val),
            ..Default::default()
        }
    }

    fn external(val: font_kit::font::Font) -> Self {
        Font {
            font_data: FontData::External(val),
            ..Default::default()
        }
    }

    pub fn postscript_name(&self) -> String {
        match &self.font_data {
            FontData::BuiltIn(builtin) => builtin.metrics().font_name.to_owned(),
            FontData::External(ext_font) => ext_font.postscript_name().unwrap(),
        }
    }

    pub fn ascent(&self) -> FontUnit {
        match &self.font_data {
            FontData::BuiltIn(font) => font.metrics().ascender,
            FontData::External(font) => scale_ext(font, font.metrics().ascent),
        }
    }

    pub fn descent(&self) -> FontUnit {
        match &self.font_data {
            FontData::BuiltIn(font) => font.metrics().descender,
            FontData::External(font) => scale_ext(font, font.metrics().descent),
        }
    }

    pub fn advance(&self, glyph: GlyphIndex) -> FontUnit {
        let result = match &self.font_data {
            FontData::BuiltIn(font) => {
                font.metrics()
                    .char_metrics
                    .get(glyph.0 as usize)
                    .unwrap()
                    .advance_width
            }
            FontData::External(font) => scale_ext(font, font.advance(glyph.0).unwrap().x()),
        };
        self.glyph_advances.lock().unwrap().insert(glyph, result);
        result
    }

    pub fn scale_to_px(&self, val: FontUnit, font_size: PixelLength) -> PixelLength {
        val.0 as f32 * font_size / 1000.0f32
    }

    pub fn font_height(&self) -> FontUnit {
        self.ascent() - self.descent()
    }

    pub fn glyph_metrics_provider<'font>(
        &'font self,
        font_size: PixelLength,
    ) -> Box<dyn 'font + GlyphMetricsProvide> {
        match &self.font_data {
            FontData::BuiltIn(builtin) => Box::new(FontInstance::new(
                *builtin,
                self.glyph_advances.lock().unwrap(),
                font_size,
            )),
            FontData::External(font) => Box::new(FontInstance::new(
                font,
                self.glyph_advances.lock().unwrap(),
                font_size,
            )),
        }
    }
}

impl Default for Font {
    fn default() -> Self {
        Font {
            glyph_advances: Mutex::new(BTreeMap::new()),
            to_unicode: Default::default(),
            font_data: FontData::BuiltIn(BuiltInFont::Helvetica),
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

pub fn match_font(style: &ComputedValues) -> Option<Font> {
    if let Some(_) = option_env!("PREFER_BUILTIN") {
        match_font_builtin(style)
    } else {
        let families = style
            .font
            .font_family
            .iter()
            .map(|fam| FamilyName::from(fam.clone()))
            .collect::<Vec<_>>();
        let font = SystemSource::new()
            .select_best_match(&families, &style.into())
            .ok()?
            .load()
            .ok()?;

        Some(Font::external(font))
    }
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

#[derive(Debug)]
pub(super) struct FontInstance<'font, Font: 'font> {
    pub font: Font,
    pub glyph_map: MutexGuard<'font, BTreeMap<GlyphIndex, FontUnit>>,
    pub size: PixelLength,
}

impl<'font, Font: 'font> FontInstance<'font, Font> {
    pub(super) fn new(
        font: Font,
        glyph_map: MutexGuard<'font, BTreeMap<GlyphIndex, FontUnit>>,
        size: PixelLength,
    ) -> Self {
        Self {
            font,
            glyph_map,
            size,
        }
    }
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
