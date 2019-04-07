use super::builtin::{BuiltInFont, scale};

use crate::style::properties::computed::{FontFamily, ComputedValues, PixelLength};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Font {
    BuiltIn(BuiltInFont)
}

impl Font {
    pub fn ascender(&self, font_size: PixelLength) -> PixelLength {
        match self {
            Font::BuiltIn(font) => scale(font.metrics().ascender, font_size)
        }
    }

    pub fn descender(&self, font_size: PixelLength) -> PixelLength {
        match self {
            Font::BuiltIn(font) => scale(font.metrics().descender, font_size)
        }
    }

    pub fn font_height(&self, font_size: PixelLength) -> PixelLength {
        self.ascender(font_size) - self.descender(font_size)
    }
}

impl Default for Font {
    fn default() -> Self {
        Font::BuiltIn(BuiltInFont::Helvetica)
    }
}

pub fn match_font(style: &ComputedValues) -> Option<Font> {
    style.font.font_family.iter().next().and_then(|family| {
        match family {
            FontFamily::Serif => Some(Font::BuiltIn(BuiltInFont::TimesRoman)),
            FontFamily::SansSerif => Some(Font::BuiltIn(BuiltInFont::Helvetica)),
            _ => None
        }
    })
}