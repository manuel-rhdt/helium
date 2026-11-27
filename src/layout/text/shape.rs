use harfrust::{
    Buffer, ClusterLevel, Direction, Feature as HbFeature, GlyphInfo as HbGlyphInfo, Language as HbLanguage, Script as HbScript,
    ShapeOptions, ShaperFont,
};
use read_fonts::types::Tag;

use super::{Font, font::FontUnit};
use super::fallback::TextFonts;
use super::segment::{Cluster, Span, cluster_spans, clusters};
use std::cell::RefCell;
use crate::style::properties::computed::Language;
use crate::style::properties::font_features::Feature;
use unicode_script::Script;

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
            codepoint: other.glyph_id,
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

/// Whether `ch` is an invisible formatting character (zero-width spaces and joiners, directional
/// marks, embeddings and isolates). harfbuzz gives them an invisible glyph (often the space
/// glyph), which is not drawn: the ToUnicode map of the font would otherwise map that glyph to the
/// formatting character.
pub fn is_invisible_formatting(ch: char) -> bool {
    matches!(ch,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Shapes left-to-right `text` with `font`, using the fallback fonts of `font` for characters it
/// has no glyph for. `at` is the byte offset of `text` in the text that `fonts` belongs to, or
/// `None` if `text` is not a part of it (the fonts of its clusters are used then). Glyph
/// positions are in units of 1/1000 em, independent of the font they belong to.
pub fn shape(
    text: &str,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    shape_directed(text, font, fonts, at, false)
}

/// Adds the letter spacing after every grapheme cluster of `glyphs`, which are the glyphs of
/// `text` in logical order. The spacing goes to the glyph that is drawn last in the cluster.
fn apply_letter_spacing(
    glyphs: &mut [(GlyphPosition, GlyphInfo)],
    text: &str,
    spacing: f32,
    rtl: bool,
) {
    let starts: Vec<u32> = clusters(text).into_iter().map(|c| c.start as u32).collect();
    let cluster_of = |glyph: &(GlyphPosition, GlyphInfo)| {
        starts.partition_point(|&start| start <= glyph.1.cluster)
    };
    let ids: Vec<usize> = glyphs.iter().map(cluster_of).collect();
    for (i, (position, _)) in glyphs.iter_mut().enumerate() {
        let last_drawn = if rtl {
            i == 0 || ids[i - 1] != ids[i]
        } else {
            i + 1 == ids.len() || ids[i + 1] != ids[i]
        };
        if last_drawn {
            position.x_advance.0 += spacing as f64;
        }
    }
}

/// Shapes `text` of embedding levels `levels`: runs `(start, level)` of the text that `text`
/// starts at byte `offset` of (as in `InlineFormattingContext::bidi_runs`). Every run is shaped
/// in its direction. The glyphs are returned in logical order, with clusters relative to `text`.
pub fn shape_with_levels(
    text: &str,
    offset: usize,
    levels: &[(usize, usize, u8)],
    font: &Font,
    fonts: &TextFonts,
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
                shape(&text[covered - offset..start - offset], font, fonts, Some(covered)),
                covered - offset,
            ));
        }
        result.extend(offset_clusters(
            shape_directed(
                &text[start - offset..run_end - offset],
                font,
                fonts,
                Some(start),
                level % 2 == 1,
            ),
            start - offset,
        ));
        covered = run_end;
    }
    if covered < end {
        result.extend(offset_clusters(
            shape(&text[covered - offset..], font, fonts, Some(covered)),
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
///
/// The text is split into runs of one script and one font, each of which is shaped on its own.
/// Buffers that shaping reuses so that it does not allocate for every piece of text: a text of
/// a few words is shaped in a few microseconds, so allocating and growing a harfbuzz buffer, the
/// clusters and the features for each of them was a large part of the allocations of a document.
#[derive(Default)]
struct Scratch {
    /// Empty (it is cleared when it is put back) but with the capacity of earlier texts.
    buffer: Option<Buffer>,
    features: Vec<HbFeature>,
    spans: Vec<Span>,
    /// The glyphs of the text that `with_shaped` measures.
    glyphs: Vec<(GlyphPosition, GlyphInfo)>,
    /// The text of a word and its hyphen, for shaping them together.
    hyphenated: String,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// The font of a cluster is the one that `fonts` has for the text at `at` or else the one that
/// was chosen for the cluster and its script (see `FontResolver`), so a base character and its
/// marks stay together.
pub fn shape_directed(
    text: &str,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    let mut glyphs = Vec::new();
    shape_directed_into(text, font, fonts, at, rtl, &mut glyphs);
    glyphs
}

/// Shapes `text` like [`shape`] and calls `f` with the glyphs, for measuring text: the glyphs are
/// in a buffer that is reused, so that measuring does not allocate. `f` must not shape text.
pub fn with_shaped<R>(
    text: &str,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    f: impl FnOnce(&[(GlyphPosition, GlyphInfo)]) -> R,
) -> R {
    let mut glyphs = SCRATCH.with(|scratch| std::mem::take(&mut scratch.borrow_mut().glyphs));
    glyphs.clear();
    shape_directed_into(text, font, fonts, at, false, &mut glyphs);
    let result = f(&glyphs);
    SCRATCH.with(|scratch| scratch.borrow_mut().glyphs = glyphs);
    result
}

/// [`shape_directed`], appending the glyphs to `out`, whose capacity is reused.
pub fn shape_directed_into(
    text: &str,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
    out: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    shape_text_into(text.trim_end_matches(['\r', '\n']), None, font, fonts, at, rtl, out);
}

/// Shapes `text` followed by `hyphen`, the hyphen that is inserted where a line breaks inside of
/// a word. Shaped together, the hyphen is kerned with the letters in front of it like any
/// character of the text. It is drawn with the font of the last character of `text` (see
/// [`hyphen_character`]), not with the font of the text that follows it in the document. Its
/// cluster is `text.len()`.
pub fn shape_hyphenated_into(
    text: &str,
    hyphen: char,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
    out: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    let mut hyphenated = SCRATCH.with(|scratch| std::mem::take(&mut scratch.borrow_mut().hyphenated));
    hyphenated.clear();
    hyphenated.push_str(text);
    hyphenated.push(hyphen);
    shape_text_into(&hyphenated, Some(text.len()), font, fonts, at, rtl, out);
    SCRATCH.with(|scratch| scratch.borrow_mut().hyphenated = hyphenated);
}

/// [`shape_hyphenated_into`] for measuring text, like [`with_shaped`].
pub fn with_shaped_hyphenated<R>(
    text: &str,
    hyphen: char,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    f: impl FnOnce(&[(GlyphPosition, GlyphInfo)]) -> R,
) -> R {
    let mut glyphs = SCRATCH.with(|scratch| std::mem::take(&mut scratch.borrow_mut().glyphs));
    glyphs.clear();
    shape_hyphenated_into(text, hyphen, font, fonts, at, false, &mut glyphs);
    let result = f(&glyphs);
    SCRATCH.with(|scratch| scratch.borrow_mut().glyphs = glyphs);
    result
}

/// The hyphen that a line break inside a word gets: U+2010 HYPHEN if the font that draws the
/// letter in front of it has a glyph for that, else U+002D HYPHEN-MINUS (CSS Text 3,
/// `hyphenate-character`). `last` is the byte offset of that letter in the text that `fonts` is
/// for.
pub fn hyphen_character(font: &Font, fonts: &TextFonts, last: Option<usize>) -> char {
    const HYPHEN: char = '\u{2010}';
    let set = font.fallback_set(&fonts.key);
    let index = last
        .and_then(|offset| fonts.font_at(offset))
        .filter(|&index| index as usize <= set.fonts.len())
        .unwrap_or(0);
    if font_at(font, &set.fonts, index).covers(HYPHEN) { HYPHEN } else { '-' }
}

/// Shapes `text`, of which the characters from the byte `appended_from` on were added to the text
/// that `fonts` is for: they get the font of the character in front of them.
fn shape_text_into(
    text: &str,
    appended_from: Option<usize>,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
    out: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    if text.is_empty() {
        return;
    }

    let start = out.len();
    shape_runs(text, appended_from, font, fonts, at, rtl, out);
    if fonts.letter_spacing != 0.0 {
        apply_letter_spacing(&mut out[start..], text, fonts.letter_spacing, rtl);
    }
    if fonts.word_spacing != 0.0 {
        apply_word_spacing(&mut out[start..], text, fonts.word_spacing);
    }
}

/// Whether a character is a word separator, which `word-spacing` widens (CSS Text 3, 6.1).
fn is_word_separator(ch: char) -> bool {
    matches!(ch, ' ' | '\u{a0}' | '\u{1361}' | '\u{10100}' | '\u{10101}' | '\u{1039f}' | '\u{1091f}')
}

/// Adds the word spacing to the advance of the glyphs of the word separators of `text`.
fn apply_word_spacing(glyphs: &mut [(GlyphPosition, GlyphInfo)], text: &str, spacing: f32) {
    for (position, info) in glyphs.iter_mut() {
        if text.get(info.cluster as usize..).and_then(|rest| rest.chars().next()).is_some_and(is_word_separator) {
            position.x_advance.0 += spacing as f64;
        }
    }
}

fn shape_runs(
    text: &str,
    appended_from: Option<usize>,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
    result: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    let lang = &fonts.key.lang;
    let ligatures = fonts.letter_spacing == 0.0;
    let toggles = Toggles { features: &fonts.features };
    let set = font.fallback_set(&fonts.key);
    if set.fonts.is_empty() && text.is_ascii() && fonts.orientation.is_none() {
        // the script is guessed
        shape_single(text, font, lang, None, 0, 0, rtl, ligatures, false, toggles, result);
        return;
    }
    let fallbacks = &set.fonts;

    let mut spans = SCRATCH.with(|scratch| std::mem::take(&mut scratch.borrow_mut().spans));
    cluster_spans(text, &mut spans);
    // where the font runs of the text are looked up for a cluster: appended characters are
    // looked up where the last character of the text is
    let last_start = appended_from.and_then(|end| text[..end].char_indices().next_back().map(|(index, _)| index));
    let looked_up = |start: usize| match (appended_from, last_start) {
        (Some(end), Some(last)) if start >= end => last,
        _ => start,
    };
    let mut run_start = 0;
    // The font index and the script of the current run
    let mut run: Option<(u16, Script, bool)> = None;
    for span in &spans {
        let cluster = Cluster { start: span.start, text: &text[span.start..span.end], script: span.script };
        // in vertical lines: whether the characters stand upright, which changes their glyphs
        let upright = fonts.orientation.is_some_and(|orientation| {
            cluster
                .text
                .chars()
                .next()
                .is_some_and(|ch| crate::layout::writing_mode::is_upright(ch, orientation))
        });
        let from_runs = at
            .and_then(|at| fonts.font_at(at + looked_up(cluster.start)))
            .filter(|&index| index as usize <= fallbacks.len());
        let ch_font = match (from_runs, run) {
            (Some(index), _) => index,
            // Spaces stay in the current run (if its font has them) so that a run of fallback
            // characters is not split at every space.
            // A colour emoji font is no exception: the space of Noto Color Emoji is as wide as an
            // emoji.
            (None, Some((current, _, _)))
                if single_space(cluster.text).is_some_and(|space| {
                    let current = font_at(font, fallbacks, current);
                    current.covers(space) && !current.has_color_glyphs()
                }) =>
            {
                current
            }
            (None, _) => set.index_for(font, cluster.text, cluster.script),
        };
        if let Some((current, script, was_upright)) = run
            && (current != ch_font || script != cluster.script || was_upright != upright)
        {
            shape_run(
                &text[run_start..cluster.start],
                run_start,
                font,
                lang,
                fallbacks,
                (current, script),
                rtl,
                ligatures,
                was_upright,
                toggles,
                result,
            );
            run_start = cluster.start;
        }
        run = Some((ch_font, cluster.script, upright));
    }
    if let Some((index, script, upright)) = run {
        shape_run(
            &text[run_start..],
            run_start,
            font,
            lang,
            fallbacks,
            (index, script),
            rtl,
            ligatures,
            upright,
            toggles,
            result,
        );
    }
    SCRATCH.with(|scratch| scratch.borrow_mut().spans = spans);
}

/// The OpenType features that the author turns on or off for a text.
#[derive(Clone, Copy)]
struct Toggles<'a> {
    features: &'a [Feature],
}

/// The character of a cluster that consists of a single white space character.
fn single_space(cluster: &str) -> Option<char> {
    let mut chars = cluster.chars();
    chars
        .next()
        .filter(|ch| ch.is_whitespace())
        .filter(|_| chars.next().is_none())
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
    (font_index, script): (u16, Script),
    rtl: bool,
    ligatures: bool,
    upright: bool,
    toggles: Toggles<'_>,
    out: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    shape_single(
        text,
        font_at(font, fallbacks, font_index),
        lang,
        Some(script),
        offset as u32,
        font_index,
        rtl,
        ligatures,
        upright,
        toggles,
        out,
    )
}

/// Shapes `text` with exactly `font`. `script` is the script of the text, if known.
/// `cluster_offset` is added to the cluster values and `font_index` is stored in every glyph.
/// The glyphs are appended to `out` in logical order.
fn shape_single(
    text: &str,
    font: &Font,
    lang: &Language,
    script: Option<Script>,
    cluster_offset: u32,
    font_index: u16,
    rtl: bool,
    ligatures: bool,
    upright: bool,
    toggles: Toggles<'_>,
    out: &mut Vec<(GlyphPosition, GlyphInfo)>,
) {
    // The direction is always set: harfbuzz would otherwise guess it from the script, so that
    // e.g. Hebrew text in a left-to-right run would be shaped right-to-left. The script (for
    // example for the joining of Arabic letters) is the one of the run, which also resolves
    // punctuation and digits by their neighbours. The language selects language-specific glyphs
    // (the `locl` feature, e.g. the Chinese or Japanese forms of Han characters). What is still
    // unset (an unknown script or language) is guessed by harfbuzz from the text.
    let (recycled, mut features) = SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        (scratch.buffer.take(), std::mem::take(&mut scratch.features))
    });
    let mut buffer = recycled.unwrap_or_default();
    buffer.clear();
    buffer.push_str(text);
    buffer.set_cluster_level(ClusterLevel::MonotoneCharacters);
    buffer.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
    if let Some(script) =
        script.filter(|s| !matches!(s, Script::Common | Script::Inherited | Script::Unknown))
    {
        // a harfbuzz script is the ISO 15924 tag
        buffer.set_script(HbScript::from_iso15924_tag(Tag::new(&script.as_iso15924_tag().to_be_bytes())));
    }
    if let Some(language) = lang.tag().and_then(HbLanguage::new) {
        buffer.set_language(Some(language));
    }
    buffer.guess_segment_properties();

    // Letter spaced text does not use the optional ligatures:
    // https://www.w3.org/TR/css-text-3/#letter-spacing-property
    let no_ligatures = [
        HbFeature::new(Tag::new(b"liga"), 0, ..),
        HbFeature::new(Tag::new(b"clig"), 0, ..),
    ];
    // Characters that stand upright in vertical lines use the glyphs that the font has for them
    // (rotated punctuation, small kana, ...).
    let vertical = [
        HbFeature::new(Tag::new(b"vert"), 1, ..),
        HbFeature::new(Tag::new(b"vrt2"), 1, ..),
    ];
    features.clear();
    if !ligatures {
        features.extend(no_ligatures);
    }
    if upright {
        features.extend(vertical);
    }
    // the author's features come last: they override the ones above (letter-spaced text can have
    // ligatures if the author asks for them)
    features.extend(toggles.features.iter().map(|tag_value| HbFeature::new(Tag::new(&tag_value.0), tag_value.1, ..)));
    let shaper_font = ShaperFont::new(&font.face);
    let plan = font.shape_plan(&buffer, &features);
    let shaped = harfrust::shape(&shaper_font, &mut buffer, ShapeOptions::new().plan(Some(&plan)).features(&features));

    let scale = |val| FontUnit(val as f64 * (1000f64 / font.upem as f64));

    let start = out.len();
    match shaped {
        Ok(()) => out.extend(
            buffer
                .glyph_positions()
                .iter()
                .map(|gp| GlyphPosition {
                    x_advance: scale(gp.x_advance),
                    x_offset: scale(gp.x_offset),
                    y_offset: scale(gp.y_offset),
                })
                .zip(buffer.glyph_infos().iter().map(|gi| GlyphInfo {
                    cluster: gi.cluster + cluster_offset,
                    font: font_index,
                    ..gi.clone().into()
                })),
        ),
        Err(error) => {
            // not reachable for a font that parsed; the text shows as missing glyphs
            log::warn!("cannot shape with {}: {error}", font.postscript_name);
            out.extend(text.char_indices().map(|(at, _)| {
                (
                    GlyphPosition { x_advance: FontUnit(0.0), x_offset: FontUnit(0.0), y_offset: FontUnit(0.0) },
                    GlyphInfo { codepoint: 0, cluster: at as u32 + cluster_offset, font: font_index },
                )
            }));
        }
    }
    if rtl {
        // harfbuzz returns right-to-left glyphs in visual order
        out[start..].reverse();
    }
    SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        buffer.clear();
        scratch.buffer = Some(buffer);
        scratch.features = features;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::FontMap;
    use crate::layout::text::fallback::{FontResolver, register_font};
    use crate::layout::text::font::system_font;
    use crate::style::properties::computed::{ComputedValues, FontFamily, FontStyle, FontWeight};

    struct Setup {
        font: std::sync::Arc<Font>,
        fonts: TextFonts,
    }

    /// `text` set in Termes (which has neither Hebrew nor Arabic), with fallback fonts.
    fn setup(text: &str) -> Setup {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let mut style = ComputedValues::default();
        std::sync::Arc::make_mut(&mut style.font).font_family =
            vec![FontFamily::Named("TeX Gyre Termes".into()), FontFamily::Serif];
        let font = register_font(
            &mut font_map,
            system_font(&style.font.font_family, &FontWeight::NORMAL, &FontStyle::Normal).unwrap(),
        );
        let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
        Setup { font, fonts }
    }

    #[test]
    fn spaces_between_emoji_are_set_in_the_font_of_the_text() {
        // the space of the colour emoji font is as wide as an emoji
        let s = setup("a 😀 😀 b");
        let glyphs = shape_directed("a 😀 😀 b", &s.font, &s.fonts, None, false);
        let text = "a 😀 😀 b";
        for (_, info) in &glyphs {
            if text[info.cluster as usize..].starts_with(' ') {
                assert_eq!(info.font, 0, "the space at {} is not in the font of the text", info.cluster);
            }
        }
        assert!(glyphs.iter().any(|(_, info)| info.font != 0), "the emoji have a font of their own");
    }

    fn fonts_of(glyphs: &[(GlyphPosition, GlyphInfo)]) -> Vec<u16> {
        glyphs.iter().map(|(_, g)| g.font).collect()
    }

    #[test]
    fn empty_text_and_line_breaks_give_no_glyphs() {
        let s = setup("a");
        assert!(shape("", &s.font, &s.fonts, Some(0)).is_empty());
        assert!(shape("\r\n", &s.font, &s.fonts, None).is_empty());
        // trailing line breaks are not shaped
        assert_eq!(shape("ab\n", &s.font, &s.fonts, Some(0)).len(), 2);
    }

    #[test]
    fn plain_ascii_is_shaped_with_the_primary_font() {
        let s = setup("hello");
        let glyphs = shape("hello", &s.font, &s.fonts, Some(0));
        assert_eq!(fonts_of(&glyphs), vec![0; 5]);
        assert_eq!(glyphs.iter().map(|(_, g)| g.cluster).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(glyphs.iter().all(|(p, _)| p.x_advance.0 > 0.0));
    }

    #[test]
    fn right_to_left_text_is_returned_in_logical_order() {
        let text = "אבג";
        let s = setup(text);
        let glyphs = shape_directed(text, &s.font, &s.fonts, Some(0), true);
        assert_eq!(glyphs.len(), 3);
        // two bytes per letter, increasing in logical order, all in the Hebrew font
        assert_eq!(glyphs.iter().map(|(_, g)| g.cluster).collect::<Vec<_>>(), vec![0, 2, 4]);
        assert!(fonts_of(&glyphs).iter().all(|f| *f != 0));
    }

    #[test]
    fn every_bidi_run_is_shaped_in_its_direction_with_its_fonts() {
        let text = "abc אבג";
        let s = setup(text);
        // "abc " is left-to-right, the Hebrew word right-to-left
        let levels = [(0, 4, 0u8), (4, 10, 1u8)];
        let glyphs = shape_with_levels(text, 0, &levels, &s.font, &s.fonts);
        assert_eq!(glyphs.len(), 7);
        assert_eq!(fonts_of(&glyphs)[..4], [0, 0, 0, 0]);
        assert!(fonts_of(&glyphs)[4..].iter().all(|f| *f != 0));
        let clusters: Vec<_> = glyphs.iter().map(|(_, g)| g.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 2, 3, 4, 6, 8]);

        // text between the runs is shaped left-to-right, and a part of the text is shaped
        // with the fonts of the whole
        let gaps = shape_with_levels(text, 0, &[(4, 10, 1)], &s.font, &s.fonts);
        assert_eq!(fonts_of(&gaps), fonts_of(&glyphs));
        let tail = shape_with_levels(&text[4..], 4, &[(4, 10, 1)], &s.font, &s.fonts);
        assert_eq!(fonts_of(&tail), fonts_of(&glyphs)[4..]);
        // clusters of a part are relative to it
        assert_eq!(tail[0].1.cluster, 0);
        let tail_only = shape_with_levels(&text[4..], 4, &[], &s.font, &s.fonts);
        assert_eq!(tail_only.len(), 3);
        // a run outside of the text is ignored
        let outside = shape_with_levels("abc", 0, &[(10, 20, 1)], &s.font, &s.fonts);
        assert_eq!(outside.len(), 3);
    }

    /// Text that is not part of the text of the fonts (a hyphen, a margin box) uses the font
    /// chosen for each cluster, and a space stays in the run of the text before it.
    #[test]
    fn text_without_runs_uses_the_fonts_of_its_clusters() {
        let s = setup("אב גד a");
        let text = "אב גד a";
        let glyphs = shape(text, &s.font, &s.fonts, None);
        let fonts = fonts_of(&glyphs);
        // the Hebrew words and the space between them are one font; the Latin letter is not
        assert!(fonts[..5].iter().all(|f| *f == fonts[0] && *f != 0), "{fonts:?}");
        assert_eq!(*fonts.last().unwrap(), 0);
        // the same through the runs of the text
        let with_runs = shape(text, &s.font, &s.fonts, Some(0));
        assert_eq!(fonts_of(&with_runs)[..5], fonts[..5]);
    }

    /// A text that has runs for other offsets falls back to the cluster's own font.
    #[test]
    fn offsets_outside_of_the_runs_use_the_fonts_of_the_clusters() {
        let s = setup("א");
        let glyphs = shape("א", &s.font, &s.fonts, Some(1000));
        assert!(glyphs[0].1.font != 0);
    }

    #[test]
    fn the_script_of_the_run_makes_arabic_letters_join() {
        let text = "بب";
        let s = setup(text);
        let alone = shape("ب", &s.font, &s.fonts, None);
        let joined = shape(text, &s.font, &s.fonts, Some(0));
        assert_eq!(joined.len(), 2);
        // the initial form of the first letter is another glyph than the isolated form
        assert_ne!(joined[0].1.codepoint, alone[0].1.codepoint);
    }

    #[test]
    fn scripts_are_shaped_separately_with_their_own_properties() {
        let text = "abc שלום 123";
        let s = setup(text);
        let glyphs = shape(text, &s.font, &s.fonts, Some(0));
        let fonts = fonts_of(&glyphs);
        // the Latin run, the Hebrew word
        assert!(fonts[..4].iter().all(|f| *f == 0), "{fonts:?}");
        let hebrew_font = fonts[4];
        assert_ne!(hebrew_font, 0);
        assert!(fonts[4..8].iter().all(|f| *f == hebrew_font), "{fonts:?}");
        // The digits are in the Hebrew run, but the Hebrew font has none: they keep the
        // primary font instead of showing an empty box.
        let hebrew = s.font.fallbacks(&s.fonts.key)[hebrew_font as usize - 1].clone();
        assert!(!hebrew.covers('1'));
        assert!(fonts[9..].iter().all(|f| *f == 0), "{fonts:?}");
    }

    #[test]
    fn invisible_formatting_is_recognized() {
        for ch in ['\u{200B}', '\u{200D}', '\u{200F}', '\u{202E}', '\u{2066}', '\u{FEFF}'] {
            assert!(is_invisible_formatting(ch), "{ch:?}");
        }
        assert!(!is_invisible_formatting('a'));
        assert!(!is_invisible_formatting(' '));
    }

    fn spaced(s: &Setup, spacing: f32) -> TextFonts {
        let mut fonts = s.fonts.clone();
        fonts.letter_spacing = spacing;
        fonts
    }

    fn total(glyphs: &[(GlyphPosition, GlyphInfo)]) -> f64 {
        glyphs.iter().map(|(p, _)| p.x_advance.0).sum()
    }

    #[test]
    fn word_spacing_is_added_to_the_spaces_only() {
        let s = setup("a b c\u{a0}d");
        let plain = shape("a b c\u{a0}d", &s.font, &s.fonts, Some(0));
        let mut fonts = s.fonts.clone();
        fonts.word_spacing = 100.0;
        let wide = shape("a b c\u{a0}d", &s.font, &fonts, Some(0));
        assert_eq!(plain.len(), wide.len());
        // two spaces and a no-break space
        assert!((total(&wide) - total(&plain) - 300.0).abs() < 1e-6);
    }

    #[test]
    fn letter_spacing_is_added_after_every_character() {
        let s = setup("hello");
        let plain = shape("hello", &s.font, &s.fonts, Some(0));
        let wide = shape("hello", &s.font, &spaced(&s, 100.0), Some(0));
        assert_eq!(plain.len(), wide.len());
        assert!((total(&wide) - total(&plain) - 500.0).abs() < 1e-6);
    }

    #[test]
    fn letter_spacing_keeps_marks_with_their_base() {
        let s = setup("e\u{301}x");
        let plain = shape("e\u{301}x", &s.font, &s.fonts, Some(0));
        let wide = shape("e\u{301}x", &s.font, &spaced(&s, 100.0), Some(0));
        // two grapheme clusters
        assert!((total(&wide) - total(&plain) - 200.0).abs() < 1e-6);
    }

    #[test]
    fn letter_spacing_disables_ligatures() {
        let s = setup("office");
        let plain = shape("office", &s.font, &s.fonts, Some(0));
        let wide = shape("office", &s.font, &spaced(&s, 1.0), Some(0));
        assert!(plain.len() < 6, "the font has no ffi ligature: {}", plain.len());
        assert_eq!(wide.len(), 6);
    }

    #[test]
    fn negative_letter_spacing_narrows_the_text() {
        let s = setup("abc");
        let plain = shape("abc", &s.font, &s.fonts, Some(0));
        let narrow = shape("abc", &s.font, &spaced(&s, -50.0), Some(0));
        assert!((total(&plain) - total(&narrow) - 150.0).abs() < 1e-6);
    }

    #[test]
    fn letter_spacing_applies_to_right_to_left_text_once_per_character() {
        let s = setup("אבג");
        let plain = shape_directed("אבג", &s.font, &s.fonts, Some(0), true);
        let wide = shape_directed("אבג", &s.font, &spaced(&s, 100.0), Some(0), true);
        assert!((total(&wide) - total(&plain) - 300.0).abs() < 1e-6);
    }

    fn featured(s: &Setup, features: &[Feature]) -> TextFonts {
        let mut fonts = s.fonts.clone();
        fonts.features = features.to_vec().into();
        fonts
    }

    fn glyph_ids(glyphs: &[(GlyphPosition, GlyphInfo)]) -> Vec<u32> {
        glyphs.iter().map(|(_, g)| g.codepoint).collect()
    }

    #[test]
    fn small_caps_replace_the_lowercase_letters() {
        let s = setup("small");
        let plain = shape("small", &s.font, &s.fonts, Some(0));
        let caps = shape("small", &s.font, &featured(&s, &[(*b"smcp", 1)]), Some(0));
        assert_eq!(plain.len(), caps.len());
        assert_ne!(glyph_ids(&plain), glyph_ids(&caps));
        // the capitals of the text stay the same
        let upper = shape("SMALL", &s.font, &s.fonts, Some(0));
        assert_eq!(glyph_ids(&upper).len(), caps.len());
        assert!(total(&caps) < total(&upper), "small capitals are narrower than capitals");
    }

    #[test]
    fn all_small_caps_also_replace_the_capitals() {
        let s = setup("Ab");
        let small = shape("Ab", &s.font, &featured(&s, &[(*b"smcp", 1)]), Some(0));
        let all = shape("Ab", &s.font, &featured(&s, &[(*b"smcp", 1), (*b"c2sc", 1)]), Some(0));
        assert_eq!(small[0].1.codepoint, shape("A", &s.font, &s.fonts, Some(0))[0].1.codepoint);
        assert_ne!(all[0].1.codepoint, small[0].1.codepoint);
        assert_eq!(all[0].1.codepoint != small[0].1.codepoint, true);
        assert_eq!(all[1].1.codepoint, small[1].1.codepoint);
    }

    #[test]
    fn old_style_figures_and_slashed_zeros_are_other_glyphs() {
        let s = setup("0123");
        let plain = glyph_ids(&shape("0123", &s.font, &s.fonts, Some(0)));
        let old = glyph_ids(&shape("0123", &s.font, &featured(&s, &[(*b"onum", 1)]), Some(0)));
        let slashed = glyph_ids(&shape("0123", &s.font, &featured(&s, &[(*b"zero", 1)]), Some(0)));
        assert_ne!(plain, old);
        assert_ne!(plain[0], slashed[0]);
        assert_eq!(plain[1..], slashed[1..]);
    }

    #[test]
    fn ligatures_can_be_turned_off_and_discretionary_ones_on() {
        let s = setup("fi a--b");
        let plain = shape("fi a--b", &s.font, &s.fonts, Some(0));
        let off = shape("fi a--b", &s.font, &featured(&s, &[(*b"liga", 0)]), Some(0));
        assert_eq!(plain.len() + 1, off.len(), "the fi ligature");
        // the font replaces two hyphens by a dash
        let discretionary = shape("fi a--b", &s.font, &featured(&s, &[(*b"dlig", 1)]), Some(0));
        assert_eq!(plain.len(), discretionary.len() + 1);
    }

    #[test]
    fn later_features_override_earlier_ones() {
        let s = setup("fi");
        let off_then_on =
            shape("fi", &s.font, &featured(&s, &[(*b"liga", 0), (*b"liga", 1)]), Some(0));
        assert_eq!(off_then_on.len(), 1);
        let on_then_off =
            shape("fi", &s.font, &featured(&s, &[(*b"liga", 1), (*b"liga", 0)]), Some(0));
        assert_eq!(on_then_off.len(), 2);
    }

    #[test]
    fn a_feature_the_author_asks_for_wins_over_the_letter_spacing() {
        let s = setup("fi");
        let mut fonts = featured(&s, &[(*b"liga", 1)]);
        fonts.letter_spacing = 10.0;
        assert_eq!(shape("fi", &s.font, &fonts, Some(0)).len(), 1);
        fonts.features = std::sync::Arc::from([]);
        assert_eq!(shape("fi", &s.font, &fonts, Some(0)).len(), 2);
    }

    #[test]
    fn kerning_can_be_turned_off() {
        let s = setup("AVAV");
        let kerned = shape("AVAV", &s.font, &s.fonts, Some(0));
        let plain = shape("AVAV", &s.font, &featured(&s, &[(*b"kern", 0)]), Some(0));
        assert!(total(&kerned) < total(&plain));
    }

    /// The advance of the glyphs of `glyphs`, in thousandths of an em.
    fn advance(glyphs: &[(GlyphPosition, GlyphInfo)]) -> f64 {
        glyphs.iter().map(|(position, _)| position.x_advance.0).sum()
    }

    #[test]
    fn the_hyphen_is_u2010_if_the_font_has_it() {
        let s = setup("abc");
        assert_eq!(hyphen_character(&s.font, &s.fonts, Some(2)), '\u{2010}');
    }

    /// A font without U+2010 gets the hyphen-minus.
    #[test]
    fn the_hyphen_is_the_hyphen_minus_if_the_font_lacks_u2010() {
        use harfbuzz_rs::subset::SubsetInput;
        let s = setup("abc");
        let mut input = SubsetInput::new().unwrap();
        input.unicode_set_mut().insert_str("abc-");
        let bytes = harfbuzz_rs::Face::from_bytes(&s.font.data, s.font.index).subset(&input).unwrap().face_data().as_ref().to_vec();
        let font = Font::load(std::sync::Arc::new(bytes), 0).unwrap();
        assert!(!font.covers('\u{2010}') && font.covers('-'));
        assert_eq!(hyphen_character(&font, &TextFonts::default(), Some(2)), '-');
    }

    /// The hyphen is shaped with the letters in front of it: the kerning pair of the two is
    /// applied, which a hyphen that is shaped on its own cannot have. (Termes kerns "T" and "y"
    /// with the hyphen-minus but has no pairs for U+2010.)
    #[test]
    fn the_hyphen_is_kerned_with_the_letters_in_front_of_it() {
        let s = setup("Ty");
        for letters in ["T", "y"] {
            let mut together = Vec::new();
            shape_hyphenated_into(letters, '-', &s.font, &s.fonts, Some(0), false, &mut together);
            let apart = advance(&shape_directed(letters, &s.font, &s.fonts, Some(0), false))
                + advance(&shape_directed("-", &s.font, &s.fonts, None, false));
            assert!(advance(&together) < apart, "{letters}: {} {apart}", advance(&together));
        }
    }

    /// The glyphs of the letters are the same as without the hyphen, and the hyphen is the last
    /// glyph, with the cluster that follows the text.
    #[test]
    fn the_hyphen_follows_the_text_in_the_glyphs() {
        let s = setup("abc");
        let mut together = Vec::new();
        shape_hyphenated_into("abc", '\u{2010}', &s.font, &s.fonts, Some(0), false, &mut together);
        let clusters: Vec<u32> = together.iter().map(|(_, info)| info.cluster).collect();
        assert_eq!(clusters, [0, 1, 2, 3]);
        let hyphen = s.font.nominal_glyph('\u{2010}').unwrap();
        assert_eq!(together.last().unwrap().1.codepoint, hyphen);
    }

    /// A word that ends in a character of a fallback font gets the hyphen from that font, not
    /// from the font of the text that comes after the word.
    #[test]
    fn the_hyphen_is_drawn_with_the_font_of_the_last_letter() {
        let text = "a日 b";
        let s = setup(text);
        let mut together = Vec::new();
        shape_hyphenated_into("a日", '\u{2010}', &s.font, &s.fonts, Some(0), false, &mut together);
        let last = |glyphs: &[(GlyphPosition, GlyphInfo)], cluster: u32| {
            glyphs.iter().find(|(_, info)| info.cluster == cluster).map(|(_, info)| info.font).unwrap()
        };
        assert_ne!(last(&together, 1), 0, "the character is drawn with a fallback font");
        assert_eq!(last(&together, 4), last(&together, 1), "so is the hyphen");
        assert_eq!(last(&together, 0), 0);
    }
}
