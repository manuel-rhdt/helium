//! Contains the builtin PDF fonts

pub use lemon_pdf::font::builtin::BuiltInFont;
pub use lemon_pdf::font::FontUnit;

use super::super::entity::Text;
use super::super::inline::InlineFormatter;
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
    line_height: PixelLength,
    inline_formatter: &mut InlineFormatter,
) {
    let font_metrics = match font {
        Font::BuiltIn(builtin_font) => builtin_font.metrics(),
    };
    let mut cursor = PixelLength(0.0);

    whitespace::transform_text(text);

    // tracks the string index of the last space and the cursor position for the text after the last space
    let mut last_space = (0, PixelLength(0.0));

    // make an immutable borrow of text.
    let text: &'a str = &*text;

    let mut glyph_indices = vec![];
    for (index, character) in text.char_indices() {
        let char_index = (font_metrics.unicode_mapping)(character);
        let char_metric = char_index.and_then(|index| font_metrics.char_metrics.get(index));

        if let Some(char_metric) = char_metric {
            glyph_indices.push(char_index.unwrap() as u32);
            cursor += scale(char_metric.advance_width, font_size);
            if character == ' ' {
                last_space = (glyph_indices.len() - 1, cursor);
            }
            if cursor > inline_formatter.remaining_width() {
                let full_line = glyph_indices[..last_space.0].to_owned();
                if !full_line.is_empty() {
                    split_fragment(
                        full_line,
                        font,
                        font_size,
                        line_height,
                        last_space.1,
                        inline_formatter,
                    );
                }
                inline_formatter.new_line();

                let mut i = 0;
                glyph_indices.retain(|_| {
                    i += 1;
                    (i - 1) > last_space.0
                });

                cursor = cursor - last_space.1;
            }
        } else {
            continue;
        }
        if index + 1 == text.len() {
            split_fragment(
                glyph_indices.clone(),
                font,
                font_size,
                line_height,
                cursor,
                inline_formatter,
            );
        }
    }
}

fn split_fragment(
    glyphs: Vec<u32>,
    font: &Font,
    font_size: PixelLength,
    line_height: PixelLength,
    line_width: PixelLength,
    inline_formatter: &mut InlineFormatter,
) {
    let line_gap = line_height - font.font_height(font_size);
    let fragment = inline_formatter
        .entities
        .build_entity()
        .with(
            Text {
                glyphs,
                font: font.clone(),
                font_size,
                baseline: font.ascender(font_size) + line_gap / 2.0,
            },
            inline_formatter.texts,
        )
        .build();
    inline_formatter.append_fragment(fragment, line_width, line_height);
}
