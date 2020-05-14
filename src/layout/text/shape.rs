use harfbuzz_rs::{
    shape as hb_shape, Blob, ClusterLevel, Face as HbFace, Font as HbFont,
    GlyphInfo as HbGlyphInfo, Owned, UnicodeBuffer,
};

use font_kit::handle::Handle;

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

#[derive(Debug, Copy, Clone, Default)]
pub struct GlyphPosition {
    pub x_advance: FontUnit,
    pub x_offset: FontUnit,
    pub y_offset: FontUnit,
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
                    GlyphPosition {
                        x_advance: advance,
                        ..Default::default()
                    },
                    GlyphInfo {
                        codepoint: index,
                        cluster: character_index as u32,
                    },
                )
            })
            .collect(),
        super::font::FontData::External(ext_font) => {
            let font_bytes = ext_font.copy_font_data().unwrap();
            let handle = ext_font.handle().unwrap();
            let index = match handle {
                Handle::Memory { font_index, .. } => font_index,
                Handle::Path { font_index, .. } => font_index,
            };
            let blob = Blob::with_bytes_owned(font_bytes, |b| &*b);
            let face: Owned<HbFace<'static>> = HbFace::new(blob, index);
            let hb_font = HbFont::new(face);

            let buffer = UnicodeBuffer::new()
                .add_str(text)
                .set_cluster_level(ClusterLevel::MonotoneCharacters);

            let result = hb_shape(&hb_font, buffer, &[]);

            let scale = |val| FontUnit(val as f64 * (1000f64 / font.upem as f64));

            result
                .get_glyph_positions()
                .iter()
                .map(|gp| GlyphPosition {
                    x_advance: scale(gp.x_advance),
                    x_offset: scale(gp.x_offset),
                    y_offset: scale(gp.y_offset),
                })
                .zip(result.get_glyph_infos().iter().map(|&gi| gi.into()))
                .collect()
        }
    }
}
