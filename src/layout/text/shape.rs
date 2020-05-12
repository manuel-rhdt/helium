use harfbuzz_rs::{
    shape as hb_shape, Blob, Face as HbFace, Font as HbFont, GlyphInfo as HbGlyphInfo, Owned,
    UnicodeBuffer,
};

use super::{
    font::{FontUnit, GlyphIndex},
    Font,
};

#[derive(Debug, Copy, Clone)]
pub struct GlyphInfo {
    pub codepoint: u32,
    pub cluster: u32,
}

impl From<HbGlyphInfo> for GlyphInfo {
    fn from(other: HbGlyphInfo) -> GlyphInfo {
        GlyphInfo {
            codepoint: other.codepoint,
            cluster: other.cluster,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct GlyphPosition {
    pub x_advance: FontUnit,
}

pub fn shape(text: &str, font: &Font) -> Vec<(GlyphPosition, GlyphInfo)> {
    match &font.font_data {
        super::font::FontData::BuiltIn(builtin) => text
            .chars()
            .enumerate()
            .map(|(character_index, character)| {
                let index = (builtin.metrics().unicode_mapping)(character)
                    .unwrap_or_else(|| panic!("Could not find character {:?} in font.", character))
                    as u32;
                let advance = font.advance(GlyphIndex(index));
                (
                    GlyphPosition { x_advance: advance },
                    GlyphInfo {
                        codepoint: index,
                        cluster: character_index as u32,
                    },
                )
            })
            .collect(),
        super::font::FontData::External(font) => {
            let font_bytes = font.copy_font_data().unwrap();
            let blob = Blob::with_bytes_owned(font_bytes, |b| &*b);
            let face: Owned<HbFace<'static>> = HbFace::new(blob, 0);
            let hb_font = HbFont::new(face);

            let buffer = UnicodeBuffer::new().add_str(text);

            let result = hb_shape(&hb_font, buffer, &[]);

            result
                .get_glyph_positions()
                .iter()
                .map(|gp| GlyphPosition {
                    x_advance: FontUnit(
                        gp.x_advance as f64 * 1000f64 / font.metrics().units_per_em as f64,
                    ),
                })
                .zip(result.get_glyph_infos().iter().map(|&gi| gi.into()))
                .collect()
        }
    }
}
