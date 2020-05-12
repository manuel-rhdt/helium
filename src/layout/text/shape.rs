use harfbuzz_rs::{
    shape as hb_shape, Blob, Face as HbFace, Font as HbFont, GlyphInfo, GlyphPosition, Owned,
    UnicodeBuffer,
};

use super::Font;

pub fn shape(text: &str, font: &Font) -> Vec<(GlyphPosition, GlyphInfo)> {
    match &font.font_data {
        super::font::FontData::BuiltIn(_) => todo!(),
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
                .cloned()
                .zip(result.get_glyph_infos().iter().cloned())
                .collect()
        }
    }
}
