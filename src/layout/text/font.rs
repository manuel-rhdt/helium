use super::builtin::BuiltInFont;

use crate::style::properties::computed::{FontFamily, ComputedValues};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Font {
    BuiltIn(BuiltInFont)
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