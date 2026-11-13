use harfbuzz_rs::{
    ClusterLevel, Font as HbFont, GlyphInfo as HbGlyphInfo, Shared, UnicodeBuffer,
    shape as hb_shape,
};

use super::{Font, font::FontUnit};
use super::fallback::TextFonts;
use super::segment::clusters;
use crate::style::properties::computed::Language;
use unicode_script::Script;

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
    let text = text.trim_end_matches(['\r', '\n']);
    // harfbuzz hands back a null glyph array for an empty buffer, which trips
    // the slice precondition check in harfbuzz_rs.
    if text.is_empty() {
        return Vec::new();
    }

    let mut glyphs = shape_runs(text, font, fonts, at, rtl);
    if fonts.letter_spacing != 0.0 {
        apply_letter_spacing(&mut glyphs, text, fonts.letter_spacing, rtl);
    }
    glyphs
}

fn shape_runs(
    text: &str,
    font: &Font,
    fonts: &TextFonts,
    at: Option<usize>,
    rtl: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    let lang = &fonts.key.lang;
    let ligatures = fonts.letter_spacing == 0.0;
    let set = font.fallback_set(&fonts.key);
    if set.fonts.is_empty() && text.is_ascii() {
        // the script is guessed
        return shape_single(text, font, lang, None, 0, 0, rtl, ligatures);
    }
    let fallbacks = &set.fonts;

    let mut result = Vec::with_capacity(text.len());
    let mut run_start = 0;
    // The font index and the script of the current run
    let mut run: Option<(u16, Script)> = None;
    for cluster in clusters(text) {
        let from_runs = at
            .and_then(|at| fonts.font_at(at + cluster.start))
            .filter(|&index| index as usize <= fallbacks.len());
        let ch_font = match (from_runs, run) {
            (Some(index), _) => index,
            // Spaces stay in the current run (if its font has them) so that a run of fallback
            // characters is not split at every space.
            (None, Some((current, _)))
                if single_space(cluster.text)
                    .is_some_and(|space| font_at(font, fallbacks, current).covers(space)) =>
            {
                current
            }
            (None, _) => set.index_for(font, cluster.text, cluster.script),
        };
        if let Some((current, script)) = run
            && (current != ch_font || script != cluster.script)
        {
            result.extend(shape_run(
                &text[run_start..cluster.start],
                run_start,
                font,
                lang,
                fallbacks,
                (current, script),
                rtl,
                ligatures,
            ));
            run_start = cluster.start;
        }
        run = Some((ch_font, cluster.script));
    }
    if let Some(current) = run {
        result.extend(shape_run(
            &text[run_start..],
            run_start,
            font,
            lang,
            fallbacks,
            current,
            rtl,
            ligatures,
        ));
    }
    result
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
) -> Vec<(GlyphPosition, GlyphInfo)> {
    shape_single(
        text,
        font_at(font, fallbacks, font_index),
        lang,
        Some(script),
        offset as u32,
        font_index,
        rtl,
        ligatures,
    )
}

/// Shapes `text` with exactly `font`. `script` is the script of the text, if known.
/// `cluster_offset` is added to the cluster values and `font_index` is stored in every glyph.
/// The glyphs are returned in logical order.
fn shape_single(
    text: &str,
    font: &Font,
    lang: &Language,
    script: Option<Script>,
    cluster_offset: u32,
    font_index: u16,
    rtl: bool,
    ligatures: bool,
) -> Vec<(GlyphPosition, GlyphInfo)> {
    // The direction is always set: harfbuzz would otherwise guess it from the script, so that
    // e.g. Hebrew text in a left-to-right run would be shaped right-to-left. The script (for
    // example for the joining of Arabic letters) is the one of the run, which also resolves
    // punctuation and digits by their neighbours. The language selects language-specific glyphs
    // (the `locl` feature, e.g. the Chinese or Japanese forms of Han characters). What is still
    // unset (an unknown script or language) is guessed by harfbuzz from the text.
    let mut buffer = UnicodeBuffer::new()
        .add_str(text)
        .set_cluster_level(ClusterLevel::MonotoneCharacters)
        .set_direction(if rtl {
            harfbuzz_rs::Direction::Rtl
        } else {
            harfbuzz_rs::Direction::Ltr
        });
    if let Some(script) =
        script.filter(|s| !matches!(s, Script::Common | Script::Inherited | Script::Unknown))
    {
        // a harfbuzz script is the ISO 15924 tag
        buffer = buffer.set_script(harfbuzz_rs::Tag(script.as_iso15924_tag()));
    }
    if let Some(language) = lang.tag().and_then(|tag| tag.parse().ok()) {
        buffer = buffer.set_language(language);
    }
    buffer = buffer.guess_segment_properties();

    // Letter spaced text does not use the optional ligatures:
    // https://www.w3.org/TR/css-text-3/#letter-spacing-property
    let no_ligatures = [
        harfbuzz_rs::Feature::new(harfbuzz_rs::Tag::new('l', 'i', 'g', 'a'), 0, ..),
        harfbuzz_rs::Feature::new(harfbuzz_rs::Tag::new('c', 'l', 'i', 'g'), 0, ..),
    ];
    let features: &[harfbuzz_rs::Feature] = if ligatures { &[] } else { &no_ligatures };
    let result = hb_shape(&font.hb_font, buffer, features);

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
}
