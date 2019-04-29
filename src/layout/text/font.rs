use super::builtin::scale;

pub use super::builtin::BuiltInFont;

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, PixelLength,
};

use std::rc::Rc;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GlyphIndex(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Font {
    BuiltIn(BuiltInFont),
}

impl Font {
    pub fn ascender(&self, font_size: PixelLength) -> PixelLength {
        match self {
            Font::BuiltIn(font) => scale(font.metrics().ascender, font_size),
        }
    }

    pub fn descender(&self, font_size: PixelLength) -> PixelLength {
        match self {
            Font::BuiltIn(font) => scale(font.metrics().descender, font_size),
        }
    }

    pub fn font_height(&self, font_size: PixelLength) -> PixelLength {
        self.ascender(font_size) - self.descender(font_size)
    }

    pub fn glyph_metrics_provider(&self, font_size: PixelLength) -> Rc<dyn GlyphMetricsProvide> {
        match self {
            Font::BuiltIn(builtin) => {
                Rc::new(super::builtin::FontInstance::new(*builtin, font_size))
            }
        }
    }
}

impl Default for Font {
    fn default() -> Self {
        Font::BuiltIn(BuiltInFont::Helvetica)
    }
}

pub fn match_font(style: &ComputedValues) -> Option<Font> {
    style
        .font
        .font_family
        .iter()
        .next()
        .and_then(|family| match family {
            FontFamily::Serif => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::TimesRoman))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::TimesBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::TimesItalic))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::TimesBoldItalic))
                }
            },
            FontFamily::SansSerif => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::Helvetica))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::HelveticaBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::HelveticaOblique))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::HelveticaBoldOblique))
                }
            },
            FontFamily::Monospace => match (&style.font.font_style, &style.font.font_weight) {
                (FontStyle::Normal, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::Courier))
                }
                (FontStyle::Normal, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::CourierBold))
                }
                (FontStyle::Italic, FontWeight::Normal)
                | (FontStyle::Oblique, FontWeight::Normal) => {
                    Some(Font::BuiltIn(BuiltInFont::CourierOblique))
                }
                (FontStyle::Italic, FontWeight::Bold) | (FontStyle::Oblique, FontWeight::Bold) => {
                    Some(Font::BuiltIn(BuiltInFont::CourierBoldOblique))
                }
            },
            FontFamily::Named(_) => None,
        })
}

pub trait GlyphMetricsProvide {
    fn char_to_glyph_index(&self, ch: char) -> GlyphIndex;
    fn glyph_advance(&self, glyph: GlyphIndex) -> PixelLength;
}
