//! Contains the builtin PDF fonts

pub use lemon_pdf::font::FontUnit;
pub use lemon_pdf::font::builtin::BuiltInFont;

use super::whitespace;
use crate::style::properties::computed::PixelLength;

impl From<FontUnit> for PixelLength {
    fn from(fu: FontUnit) -> PixelLength {
        PixelLength((fu.0 / 1000.0) as _)
    }
}

fn scale(fu: FontUnit, font_size: PixelLength) -> PixelLength {
    font_size * PixelLength::from(fu)
}

/// Returns lines
pub fn layout_text<'a>(
    text: &'a mut String,
    font_size: PixelLength,
    container_width: PixelLength,
) -> impl 'a + Iterator<Item = Vec<u8>> {
    let font = BuiltInFont::Helvetica.metrics();
    let mut cursor = PixelLength(0.0);

    whitespace::transform_text(text);

    // tracks the string index of the last space and the cursor position for the text after the last space
    let mut last_space = (0, PixelLength(0.0));

    // make an immutable borrow of text.
    let text: &'a str = &*text;

    let mut encoded_str = vec![];
    text.char_indices().filter_map(move |(index, character)| {
        let char_index = (font.unicode_mapping)(character);
        let char_metric = char_index.and_then(|index| font.char_metrics.get(index));

        if let Some(char_metric) = char_metric {
            encoded_str.push(char_metric.character_code as u8);
            cursor += scale(char_metric.advance_width, font_size);
            if character == ' ' {
                last_space = (encoded_str.len() - 1, cursor);
            }
            if cursor > container_width {
                let full_line = &encoded_str[..last_space.0];
                let result = Some(full_line.to_owned());

                let mut i = 0;
                encoded_str.retain(|_| {
                    i += 1;
                    (i - 1) > last_space.0
                });

                cursor = cursor - last_space.1;
                return result;
            }
        } else {
            return None
        }
        if index + 1 == text.len() {
            return Some(encoded_str.clone());
        }
        None
    })
}

pub fn font_height(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFont::Helvetica.metrics();
    scale(metrics.ascender - metrics.descender, font_size)
}

pub fn font_ascender(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFont::Helvetica.metrics();
    scale(metrics.ascender, font_size)
}
