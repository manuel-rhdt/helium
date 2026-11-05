use harfbuzz_rs::{
    ClusterLevel, Font as HbFont, GlyphInfo as HbGlyphInfo, Shared, UnicodeBuffer,
    shape as hb_shape,
};

use super::{Font, font::FontUnit};

pub type HarfbuzzFont = Shared<HbFont<'static>>;

#[derive(Debug, Copy, Clone)]
pub struct GlyphInfo {
    pub codepoint: u32,
    pub cluster: u32,
    /// The font the glyph belongs to: `0` is the font that was passed to `shape`, `n > 0` is
    /// its `n`-th fallback font (see `Font::fallbacks`).
    pub font: u16,
}

impl From<HbGlyphInfo> for GlyphInfo {
    fn from(other: HbGlyphInfo) -> GlyphInfo {
        GlyphInfo {
            codepoint: other.codepoint,
            cluster: other.cluster,
            font: 0,
        }
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub struct GlyphPosition {
    pub x_advance: FontUnit,
    pub x_offset: FontUnit,
    pub y_offset: FontUnit,
}

/// Whether `ch` is invisible formatting (joiners, variation selectors, ...) that is shaped
/// together with the preceding character instead of choosing a font of its own.
pub(super) fn follows_previous_font(ch: char) -> bool {
    matches!(ch,
        '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{E0100}'..='\u{E01EF}' | '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

/// The index (as in `GlyphInfo::font`) of the first font of `font` and its fallbacks that has a
/// glyph for `ch`. Falls back to `font` itself if no font covers the character.
fn font_index_for(ch: char, font: &Font, fallbacks: &[std::sync::Arc<Font>]) -> u16 {
    if font.covers(ch) {
        return 0;
    }
    fallbacks
        .iter()
        .position(|fallback| fallback.covers(ch))
        .map_or(0, |index| index as u16 + 1)
}

/// Shapes `text` with `font`, using the fallback fonts of `font` for characters it has no glyph
/// for. Glyph positions are in units of 1/1000 em, independent of the font they belong to.
pub fn shape(text: &str, font: &Font) -> Vec<(GlyphPosition, GlyphInfo)> {
    let text = text.trim_end_matches(['\r', '\n']);
    // harfbuzz hands back a null glyph array for an empty buffer, which trips
    // the slice precondition check in harfbuzz_rs.
    if text.is_empty() {
        return Vec::new();
    }

    let fallbacks = font.fallbacks.read();
    if fallbacks.is_empty() {
        return shape_single(text, font, 0, 0);
    }

    // Split the text into runs of characters that are rendered with the same font and shape
    // every run on its own.
    let mut result = Vec::with_capacity(text.len());
    let mut run_start = 0;
    let mut run_font = None;
    for (index, ch) in text.char_indices() {
        // Spaces stay in the current run (if its font has them) so that a run of fallback
        // characters is not split at every space.
        let ch_font = match run_font {
            Some(current)
                if follows_previous_font(ch)
                    || (ch.is_whitespace() && font_at(font, &fallbacks, current).covers(ch)) =>
            {
                current
            }
            _ => font_index_for(ch, font, &fallbacks),
        };
        match run_font {
            Some(current) if current != ch_font => {
                result.extend(shape_run(
                    &text[run_start..index],
                    run_start,
                    font,
                    &fallbacks,
                    current,
                ));
                run_start = index;
            }
            _ => {}
        }
        run_font = Some(ch_font);
    }
    if let Some(current) = run_font {
        result.extend(shape_run(
            &text[run_start..],
            run_start,
            font,
            &fallbacks,
            current,
        ));
    }
    result
}

fn font_at<'a>(font: &'a Font, fallbacks: &'a [std::sync::Arc<Font>], font_index: u16) -> &'a Font {
    match font_index {
        0 => font,
        n => &fallbacks[n as usize - 1],
    }
}

fn shape_run(
    text: &str,
    offset: usize,
    font: &Font,
    fallbacks: &[std::sync::Arc<Font>],
    font_index: u16,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    shape_single(
        text,
        font_at(font, fallbacks, font_index),
        offset as u32,
        font_index,
    )
}

/// Shapes `text` with exactly `font`. `cluster_offset` is added to the cluster values and
/// `font_index` is stored in every glyph.
fn shape_single(
    text: &str,
    font: &Font,
    cluster_offset: u32,
    font_index: u16,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    match &font.font_data {
        super::font::FontData::BuiltIn(builtin) => text
            .char_indices()
            .map(|(byte_index, character)| {
                // Characters without a glyph were reported during box construction; render them
                // as a question mark rather than failing.
                let index = super::font::builtin_glyph(*builtin, character)
                    .or_else(|| super::font::builtin_glyph(*builtin, '?'))
                    .expect("built-in fonts contain a question mark");
                let advance = font.advance(index);
                (
                    GlyphPosition {
                        x_advance: advance,
                        ..Default::default()
                    },
                    GlyphInfo {
                        codepoint: index.0,
                        cluster: cluster_offset + byte_index as u32,
                        font: font_index,
                    },
                )
            })
            .collect(),
        super::font::FontData::External(_) => {
            let hb_font = &font.hb_font;

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
                .zip(result.get_glyph_infos().iter().map(|&gi| GlyphInfo {
                    cluster: gi.cluster + cluster_offset,
                    font: font_index,
                    ..gi.into()
                }))
                .collect()
        }
    }
}
