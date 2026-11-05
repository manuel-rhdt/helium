use harfbuzz_rs::{
    ClusterLevel, Font as HbFont, GlyphInfo as HbGlyphInfo, Shared, UnicodeBuffer,
    shape as hb_shape,
};

use super::{Font, font::FontUnit};
use crate::style::properties::computed::Language;

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
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{E0100}'..='\u{E01EF}' | '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

/// Whether `ch` is an invisible formatting character (zero-width spaces and joiners, directional
/// marks, embeddings and isolates). harfbuzz gives them an invisible glyph (often the space
/// glyph), which is not drawn: the ToUnicode map of the font would otherwise map that glyph to the
/// formatting character.
pub fn is_invisible_formatting(ch: char) -> bool {
    matches!(ch,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
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

/// Shapes left-to-right `text` with `font`, using the fallback fonts of `font` for characters it
/// has no glyph for. Glyph positions are in units of 1/1000 em, independent of the font they
/// belong to.
pub fn shape(text: &str, font: &Font, lang: &Language) -> Vec<(GlyphPosition, GlyphInfo)> {
    shape_directed(text, font, lang, false)
}

/// Shapes `text` of embedding levels `levels`: runs `(start, level)` of the text that `text`
/// starts at byte `offset` of (as in `InlineFormattingContext::bidi_runs`). Every run is shaped
/// in its direction. The glyphs are returned in logical order, with clusters relative to `text`.
pub fn shape_with_levels(
    text: &str,
    offset: usize,
    levels: &[(usize, usize, u8)],
    font: &Font,
    lang: &Language,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    let end = offset + text.len();
    let mut result = Vec::new();
    let mut covered = offset;
    for &(start, run_end, level) in levels {
        let (start, run_end) = (start.max(offset), run_end.min(end));
        if start >= run_end {
            continue;
        }
        if covered < start {
            // not part of any run: left-to-right
            result.extend(offset_clusters(
                shape(&text[covered - offset..start - offset], font, lang),
                covered - offset,
            ));
        }
        result.extend(offset_clusters(
            shape_directed(
                &text[start - offset..run_end - offset],
                font,
                lang,
                level % 2 == 1,
            ),
            start - offset,
        ));
        covered = run_end;
    }
    if covered < end {
        result.extend(offset_clusters(
            shape(&text[covered - offset..], font, lang),
            covered - offset,
        ));
    }
    result
}

fn offset_clusters(
    mut glyphs: Vec<(GlyphPosition, GlyphInfo)>,
    offset: usize,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    for (_, info) in &mut glyphs {
        info.cluster += offset as u32;
    }
    glyphs
}

/// Shapes `text` in the given direction. The glyphs are returned in logical order (for
/// right-to-left text this is the reverse of the order in which they are drawn), so that the
/// cluster values increase.
pub fn shape_directed(
    text: &str,
    font: &Font,
    lang: &Language,
    rtl: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    let text = text.trim_end_matches(['\r', '\n']);
    // harfbuzz hands back a null glyph array for an empty buffer, which trips
    // the slice precondition check in harfbuzz_rs.
    if text.is_empty() {
        return Vec::new();
    }

    let fallbacks = font.fallbacks(lang);
    if fallbacks.is_empty() {
        return shape_single(text, font, lang, 0, 0, rtl);
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
                    lang,
                    &fallbacks,
                    current,
                    rtl,
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
            lang,
            &fallbacks,
            current,
            rtl,
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
    lang: &Language,
    fallbacks: &[std::sync::Arc<Font>],
    font_index: u16,
    rtl: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    shape_single(
        text,
        font_at(font, fallbacks, font_index),
        lang,
        offset as u32,
        font_index,
        rtl,
    )
}

/// Shapes `text` with exactly `font`. `cluster_offset` is added to the cluster values and
/// `font_index` is stored in every glyph. The glyphs are returned in logical order.
fn shape_single(
    text: &str,
    font: &Font,
    lang: &Language,
    cluster_offset: u32,
    font_index: u16,
    rtl: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    // The direction is always set: `hb_shape` would otherwise guess it from the script, so that
    // e.g. Hebrew text in a left-to-right run would be shaped right-to-left. The script (for
    // example for the joining of Arabic letters) is guessed by `hb_shape`. The language selects
    // language-specific glyphs (the `locl` feature, e.g. the Chinese or Japanese forms of Han
    // characters). An unknown language is left to HarfBuzz's default.
    let mut buffer = UnicodeBuffer::new()
        .add_str(text)
        .set_cluster_level(ClusterLevel::MonotoneCharacters)
        .set_direction(if rtl {
            harfbuzz_rs::Direction::Rtl
        } else {
            harfbuzz_rs::Direction::Ltr
        });
    if let Some(language) = lang.tag().and_then(|tag| tag.parse().ok()) {
        buffer = buffer.set_language(language);
    }

    let result = hb_shape(&font.hb_font, buffer, &[]);

    let scale = |val| FontUnit(val as f64 * (1000f64 / font.upem as f64));

    let glyphs = result
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
        }));
    if rtl {
        // harfbuzz returns right-to-left glyphs in visual order
        glyphs.rev().collect()
    } else {
        glyphs.collect()
    }
}
