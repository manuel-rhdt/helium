//! Contains the builtin PDF fonts

pub use lemon_pdf::font::builtin::BuiltInFont;
pub use lemon_pdf::font::FontUnit;

use super::{whitespace, Font};
use crate::style::properties::computed::PixelLength;

impl From<FontUnit> for PixelLength {
    fn from(fu: FontUnit) -> PixelLength {
        PixelLength((fu.0 / 1000.0) as _)
    }
}

pub fn scale(fu: FontUnit, font_size: PixelLength) -> PixelLength {
    font_size * PixelLength::from(fu)
}

/// Returns lines
pub fn layout_text<'a>(
    text: &'a mut String,
    font: &Font,
    font_size: PixelLength,
    container_width: PixelLength,
) -> impl 'a + Iterator<Item = Vec<u32>> {
    let font = match font {
        Font::BuiltIn(builtin_font) => builtin_font.metrics(),
    };
    let mut cursor = PixelLength(0.0);

    whitespace::transform_text(text);

    // tracks the string index of the last space and the cursor position for the text after the last space
    let mut last_space = (0, PixelLength(0.0));

    // make an immutable borrow of text.
    let text: &'a str = &*text;

    let mut glyph_indices = vec![];
    text.char_indices().filter_map(move |(index, character)| {
        let char_index = (font.unicode_mapping)(character);
        let char_metric = char_index.and_then(|index| font.char_metrics.get(index));

        if let Some(char_metric) = char_metric {
            glyph_indices.push(char_index.unwrap() as u32);
            cursor += scale(char_metric.advance_width, font_size);
            if character == ' ' {
                last_space = (glyph_indices.len() - 1, cursor);
            }
            if cursor > container_width {
                let full_line = &glyph_indices[..last_space.0];
                let result = Some(full_line.to_owned());

                let mut i = 0;
                glyph_indices.retain(|_| {
                    i += 1;
                    (i - 1) > last_space.0
                });

                cursor = cursor - last_space.1;
                return result;
            }
        } else {
            return None;
        }
        if index + 1 == text.len() {
            return Some(glyph_indices.clone());
        }
        None
    })
}
