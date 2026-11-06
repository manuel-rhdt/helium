//! Font fallback: finding a font for the characters that the font chosen by `font-family` has no
//! glyph for.
//!
//! Fallback fonts are resolved during box construction, when the text of every run is known. For
//! every character of a text run that the run's font does not cover, the fonts below are tried in
//! order and the first one that has a glyph for the character is appended to the fallback fonts
//! of the run's font (`Font::fallbacks`):
//!
//! 1. the other families listed in `font-family` (as CSS prescribes),
//! 2. families designed for the script of the character (`script_families`), in their serif or
//!    sans-serif design depending on the `font-family` (`is_serif`), like the default fonts per
//!    script of browsers, and for serif text general serif families (`SERIF_FAMILIES`),
//! 3. a fixed list of families with a broad character coverage (`FALLBACK_FAMILIES`),
//! 4. the font built into Helium (DejaVu Sans, see `Font::bundled`),
//! 5. on Linux, any installed font that covers the character, as found by fontconfig.
//!
//! Shaping (`shape`) then splits the text into runs per font, and painting switches to the
//! fallback font for those runs. The fixed lists and the bundled font come before the fontconfig
//! query so that the output does not depend on the order fontconfig happens to sort fonts in, as
//! far as possible.
//!
//! Characters that no font covers are collected in the `MissingGlyphs` resource and reported
//! to the user (see `MissingGlyphs::report`).

use super::font::{Font, system_font};
use crate::layout::FontMap;
use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, Language,
};

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::sync::Arc;

/// Families with a broad character coverage that are tried, in this order, for characters that
/// none of the families of `font-family` cover. Families that are not installed are skipped.
const FALLBACK_FAMILIES: &[&str] = &[
    // Linux
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "DejaVu Serif",
    "FreeSerif",
    "FreeSans",
    "Symbola",
    // macOS
    "Apple Symbols",
    "Arial Unicode MS",
    "STIX Two Text",
    // Windows
    "Segoe UI Symbol",
    "Cambria Math",
];

/// General-purpose serif families that are tried for text in a serif font before the
/// (mostly sans-serif) `FALLBACK_FAMILIES`, so that e.g. a dash missing from a font comes from a
/// serif font as well.
const SERIF_FAMILIES: &[&str] = &[
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "Times New Roman",
    "Times",
];

/// Families designed for the script of `ch`, in order of preference, for text in a serif font
/// (`serif`) or not. They are tried before the broad-coverage `FALLBACK_FAMILIES`, whose glyphs
/// for these scripts (if any) are rudimentary, like browsers use a default font per script.
///
/// The language of the text is not known, so Han characters get the Japanese fonts first, as
/// fontconfig prefers them by default; Urdu text gets a Naskh font rather than a Nastaliq one.
fn script_families(ch: char, serif: bool) -> &'static [&'static str] {
    // (serif families, sans-serif families)
    let (serif_families, sans_families): (&[&str], &[&str]) = match ch {
        '\u{0600}'..='\u{06FF}'
        | '\u{0750}'..='\u{077F}'
        | '\u{0870}'..='\u{08FF}'
        | '\u{FB50}'..='\u{FDFF}'
        | '\u{FE70}'..='\u{FEFF}' => (
            &[
                "Noto Naskh Arabic",
                "Amiri",
                "Scheherazade New",
                "Noto Sans Arabic",
                "Geeza Pro",
                "Traditional Arabic",
                "Times New Roman",
            ],
            &[
                "Noto Sans Arabic",
                "Noto Naskh Arabic",
                "Geeza Pro",
                "Segoe UI",
                "Arial",
            ],
        ),
        '\u{0590}'..='\u{05FF}' | '\u{FB1D}'..='\u{FB4F}' => (
            &[
                "Noto Serif Hebrew",
                "Frank Ruehl CLM",
                "David CLM",
                "Noto Sans Hebrew",
                "Times New Roman",
            ],
            &["Noto Sans Hebrew", "Noto Serif Hebrew", "Arial Hebrew", "Arial"],
        ),
        '\u{0700}'..='\u{074F}' => (&["Noto Sans Syriac"], &["Noto Sans Syriac"]),
        '\u{0780}'..='\u{07BF}' => (&["Noto Sans Thaana"], &["Noto Sans Thaana"]),
        '\u{0900}'..='\u{097F}' | '\u{A8E0}'..='\u{A8FF}' => (
            &[
                "Noto Serif Devanagari",
                "Noto Sans Devanagari",
                "Kohinoor Devanagari",
                "Mangal",
            ],
            &[
                "Noto Sans Devanagari",
                "Noto Serif Devanagari",
                "Kohinoor Devanagari",
                "Nirmala UI",
            ],
        ),
        '\u{0980}'..='\u{09FF}' => (
            &["Noto Serif Bengali", "Noto Sans Bengali"],
            &["Noto Sans Bengali", "Noto Serif Bengali", "Nirmala UI"],
        ),
        '\u{0A00}'..='\u{0A7F}' => (
            &["Noto Serif Gurmukhi", "Noto Sans Gurmukhi"],
            &["Noto Sans Gurmukhi", "Noto Serif Gurmukhi", "Nirmala UI"],
        ),
        '\u{0A80}'..='\u{0AFF}' => (
            &["Noto Serif Gujarati", "Noto Sans Gujarati"],
            &["Noto Sans Gujarati", "Noto Serif Gujarati", "Nirmala UI"],
        ),
        '\u{0B80}'..='\u{0BFF}' => (
            &["Noto Serif Tamil", "Noto Sans Tamil"],
            &["Noto Sans Tamil", "Noto Serif Tamil", "Nirmala UI"],
        ),
        '\u{0C00}'..='\u{0C7F}' => (
            &["Noto Serif Telugu", "Noto Sans Telugu"],
            &["Noto Sans Telugu", "Noto Serif Telugu", "Nirmala UI"],
        ),
        '\u{0C80}'..='\u{0CFF}' => (
            &["Noto Serif Kannada", "Noto Sans Kannada"],
            &["Noto Sans Kannada", "Noto Serif Kannada", "Nirmala UI"],
        ),
        '\u{0D00}'..='\u{0D7F}' => (
            &["Noto Serif Malayalam", "Noto Sans Malayalam"],
            &["Noto Sans Malayalam", "Noto Serif Malayalam", "Nirmala UI"],
        ),
        '\u{0D80}'..='\u{0DFF}' => (
            &["Noto Serif Sinhala", "Noto Sans Sinhala"],
            &["Noto Sans Sinhala", "Noto Serif Sinhala"],
        ),
        '\u{0E00}'..='\u{0E7F}' => (
            &["Noto Serif Thai", "Noto Sans Thai", "Thonburi", "Tahoma"],
            &["Noto Sans Thai", "Noto Serif Thai", "Thonburi", "Tahoma"],
        ),
        '\u{0E80}'..='\u{0EFF}' => (
            &["Noto Serif Lao", "Noto Sans Lao"],
            &["Noto Sans Lao", "Noto Serif Lao"],
        ),
        '\u{0F00}'..='\u{0FFF}' => (&["Noto Serif Tibetan"], &["Noto Serif Tibetan"]),
        '\u{1000}'..='\u{109F}' => (
            &["Noto Serif Myanmar", "Noto Sans Myanmar"],
            &["Noto Sans Myanmar", "Noto Serif Myanmar"],
        ),
        '\u{1780}'..='\u{17FF}' => (
            &["Noto Serif Khmer", "Noto Sans Khmer"],
            &["Noto Sans Khmer", "Noto Serif Khmer"],
        ),
        '\u{0530}'..='\u{058F}' => (
            &["Noto Serif Armenian", "Noto Sans Armenian"],
            &["Noto Sans Armenian", "Noto Serif Armenian"],
        ),
        '\u{10A0}'..='\u{10FF}' => (
            &["Noto Serif Georgian", "Noto Sans Georgian"],
            &["Noto Sans Georgian", "Noto Serif Georgian"],
        ),
        '\u{1200}'..='\u{139F}' => (
            &["Noto Serif Ethiopic", "Noto Sans Ethiopic"],
            &["Noto Sans Ethiopic", "Noto Serif Ethiopic"],
        ),
        // kana
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' => (
            &[
                "Noto Serif CJK JP",
                "Source Han Serif JP",
                "Hiragino Mincho ProN",
                "Yu Mincho",
                "MS Mincho",
            ],
            &[
                "Noto Sans CJK JP",
                "Source Han Sans JP",
                "Hiragino Sans",
                "Yu Gothic",
                "Meiryo",
            ],
        ),
        // Hangul
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => (
            &["Noto Serif CJK KR", "Source Han Serif K", "AppleMyungjo", "Batang"],
            &["Noto Sans CJK KR", "Source Han Sans K", "Apple SD Gothic Neo", "Malgun Gothic"],
        ),
        // Han and CJK punctuation
        '\u{2E80}'..='\u{2FDF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}'
        | '\u{20000}'..='\u{3FFFF}' => (
            &[
                "Noto Serif CJK JP",
                "Noto Serif CJK SC",
                "Noto Serif CJK TC",
                "Source Han Serif",
                "Songti SC",
                "SimSun",
            ],
            &[
                "Noto Sans CJK JP",
                "Noto Sans CJK SC",
                "Noto Sans CJK TC",
                "Source Han Sans",
                "PingFang SC",
                "Microsoft YaHei",
            ],
        ),
        _ => (&[], &[]),
    };
    if serif {
        serif_families
    } else {
        sans_families
    }
}

/// Families for the script of `ch` that are preferred for text in a particular language `lang`
/// (in a serif font or not), before the general ones of `script_families`: the Chinese,
/// Japanese and Korean forms of Han characters and punctuation, Nastaliq for Urdu.
fn language_families(ch: char, serif: bool, lang: &Language) -> &'static [&'static str] {
    let Some(primary) = lang.primary() else {
        return &[];
    };
    let is_han_or_cjk_punctuation = matches!(ch,
        '\u{2E80}'..='\u{2FDF}' | '\u{3000}'..='\u{303F}' | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}'
        | '\u{20000}'..='\u{3FFFF}');
    let is_arabic = matches!(ch,
        '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' | '\u{0870}'..='\u{08FF}'
        | '\u{FB50}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}');
    let (serif_families, sans_families): (&[&str], &[&str]) = if is_han_or_cjk_punctuation {
        match primary {
            "ja" => (
                &["Noto Serif CJK JP", "Source Han Serif JP", "Hiragino Mincho ProN", "Yu Mincho"],
                &["Noto Sans CJK JP", "Source Han Sans JP", "Hiragino Sans", "Yu Gothic"],
            ),
            "ko" => (
                &["Noto Serif CJK KR", "Source Han Serif K", "AppleMyungjo", "Batang"],
                &["Noto Sans CJK KR", "Source Han Sans K", "Apple SD Gothic Neo", "Malgun Gothic"],
            ),
            "zh" | "yue" | "cmn" => {
                let region = lang.region();
                if region == Some("hk") || region == Some("mo") || primary == "yue" {
                    (
                        &["Noto Serif CJK HK", "Noto Serif CJK TC", "Source Han Serif TC"],
                        &["Noto Sans CJK HK", "Noto Sans CJK TC", "Source Han Sans HC"],
                    )
                } else if lang.script() == Some("hant") || region == Some("tw") {
                    (
                        &["Noto Serif CJK TC", "Source Han Serif TC", "Songti TC", "PMingLiU"],
                        &["Noto Sans CJK TC", "Source Han Sans TC", "PingFang TC", "Microsoft JhengHei"],
                    )
                } else {
                    (
                        &["Noto Serif CJK SC", "Source Han Serif SC", "Songti SC", "SimSun"],
                        &["Noto Sans CJK SC", "Source Han Sans SC", "PingFang SC", "Microsoft YaHei"],
                    )
                }
            }
            _ => (&[], &[]),
        }
    } else if is_arabic {
        match primary {
            "ur" => (
                &["Noto Nastaliq Urdu", "Jameel Noori Nastaleeq", "Urdu Typesetting"],
                &["Noto Nastaliq Urdu", "Noto Sans Arabic"],
            ),
            "fa" => (&["Noto Naskh Arabic"], &["Vazirmatn", "Noto Sans Arabic"]),
            _ => (&[], &[]),
        }
    } else {
        (&[], &[])
    };
    if serif {
        serif_families
    } else {
        sans_families
    }
}

/// Whether the text of a `font-family` is set in a serif typeface, judged by the generic family
/// or the name of the first family that says so: fallback fonts for it are then chosen from
/// serif families.
fn is_serif(families: &[FontFamily]) -> bool {
    const SERIF_NAMES: &[&str] = &[
        "serif", "times", "roman", "georgia", "garamond", "palatino", "baskerville", "cambria",
        "caslon", "bodoni", "didot", "century", "charter", "constantia", "libertin", "minion",
        "book antiqua", "naskh", "mincho", "song", "ming", "frank ruehl", "david",
    ];
    for family in families {
        match family {
            FontFamily::Serif => return true,
            FontFamily::SansSerif | FontFamily::Monospace => return false,
            FontFamily::Named(name) => {
                let name = name.to_lowercase();
                if name.contains("sans") || name.contains("mono") || name.contains("gothic") {
                    return false;
                }
                if SERIF_NAMES.iter().any(|serif| name.contains(serif)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Finds fallback fonts. One resolver is used for the whole document so that every font is
/// loaded at most once.
pub struct FontResolver {
    /// The installed font (if any) for a family, weight and style.
    family_cache: HashMap<(String, FontWeight, FontStyle), Option<Arc<Font>>>,
    /// The font that fontconfig found for a character, weight and style.
    char_cache: HashMap<(char, FontWeight, FontStyle, Language), Option<Arc<Font>>>,
    missing: MissingGlyphs,
}

impl Default for FontResolver {
    fn default() -> Self {
        FontResolver {
            family_cache: HashMap::new(),
            char_cache: HashMap::new(),
            missing: MissingGlyphs::default(),
        }
    }
}

impl std::fmt::Debug for FontResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("FontResolver").finish()
    }
}

/// Inserts `font` into the font map, unless a font of the same PostScript name is already in
/// it, and returns the font of the map.
///
/// Fonts are referred to by name in the display list, so there must only ever be one `Font` per
/// name: glyphs that are recorded in a second instance would be missing from the embedded font.
pub(crate) fn register_font(font_map: &mut FontMap, font: Font) -> Arc<Font> {
    font_map
        .entry(font.postscript_name())
        .or_insert_with(|| Arc::new(font))
        .clone()
}

/// Whether `ch` needs no glyph of its own (line breaks, joiners, combining marks, ...).
fn needs_no_font(ch: char) -> bool {
    ch.is_control() || super::shape::follows_previous_font(ch) || ch == '\u{00AD}'
}

impl FontResolver {
    /// Makes sure that every character of `text`, which is rendered with `font` (the font matched
    /// for `style`), is covered by `font` or one of its fallback fonts, adding fallback fonts
    /// where necessary. Characters that no installed font covers are recorded as missing.
    pub fn ensure_coverage(
        &mut self,
        style: &ComputedValues,
        font: &Font,
        text: &str,
        font_map: &mut FontMap,
    ) {
        for (byte_index, ch) in text.char_indices() {
            if needs_no_font(ch)
                || font.covers(ch)
                || font.fallbacks(&style.font.lang).iter().any(|f| f.covers(ch))
            {
                continue;
            }
            match self.find_fallback(ch, style, font_map) {
                Some(fallback) => {
                    log::info!(
                        "Font {} has no glyph for {}; using {} instead.",
                        font.postscript_name(),
                        describe_char(ch),
                        fallback.postscript_name()
                    );
                    font.add_fallback(&style.font.lang, fallback);
                }
                None => self.missing.record(ch, font, style, text, byte_index),
            }
        }
    }

    /// The characters that could not be rendered with any font.
    pub fn take_missing_glyphs(&mut self) -> MissingGlyphs {
        std::mem::take(&mut self.missing)
    }

    fn find_fallback(
        &mut self,
        ch: char,
        style: &ComputedValues,
        font_map: &mut FontMap,
    ) -> Option<Arc<Font>> {
        let weight = style.font.font_weight.clone();
        let font_style = style.font.font_style.clone();

        let serif = is_serif(&style.font.font_family);
        let families = style
            .font
            .font_family
            .iter()
            .cloned()
            .chain(
                language_families(ch, serif, &style.font.lang)
                    .iter()
                    .chain(script_families(ch, serif))
                    .chain(if serif { SERIF_FAMILIES } else { &[] })
                    .chain(FALLBACK_FAMILIES)
                    .map(|name| FontFamily::Named(name.to_string())),
            )
            .chain([FontFamily::SansSerif, FontFamily::Serif]);
        for family in families {
            if let Some(font) = self.family_font(family, &weight, &font_style, font_map)
                && font.covers(ch)
            {
                return Some(font);
            }
        }

        let bundled = register_font(font_map, Font::bundled(&weight));
        if bundled.covers(ch) {
            return Some(bundled);
        }

        let lang = style.font.lang.clone();
        self.char_cache
            .entry((ch, weight.clone(), font_style.clone(), lang.clone()))
            .or_insert_with(|| {
                system::font_file_for_char(ch, &weight, &font_style, &lang)
                    .and_then(|(path, index)| Font::load(Arc::new(std::fs::read(path).ok()?), index))
                    .map(|font| register_font(font_map, font))
            })
            .clone()
            .filter(|font| font.covers(ch))
    }

    fn family_font(
        &mut self,
        family: FontFamily,
        weight: &FontWeight,
        style: &FontStyle,
        font_map: &mut FontMap,
    ) -> Option<Arc<Font>> {
        self.family_cache
            .entry((format!("{:?}", family), weight.clone(), style.clone()))
            .or_insert_with(|| {
                system_font(&[family], weight, style)
                    .map(|font| register_font(font_map, font))
            })
            .clone()
    }
}

/// `U+2192 '→'`
pub fn describe_char(ch: char) -> String {
    if ch.is_control() || ch.is_whitespace() {
        format!("U+{:04X}", ch as u32)
    } else {
        format!("U+{:04X} '{}'", ch as u32, ch)
    }
}

/// A character that could not be rendered with any font.
#[derive(Debug, Clone)]
pub struct MissingGlyph {
    /// How often the character occurs in the document.
    pub count: usize,
    /// The fonts that the text was rendered with (PostScript names).
    pub fonts: Vec<String>,
    /// The `font-family` values of the text the character occurs in.
    pub font_families: Vec<String>,
    /// The text around the first occurrence of the character.
    pub context: String,
}

/// The characters of the document that could not be rendered with any font. They are drawn
/// with the `.notdef` glyph of the font (usually an empty box).
#[derive(Debug, Clone, Default)]
pub struct MissingGlyphs(pub BTreeMap<char, MissingGlyph>);

impl MissingGlyphs {
    fn record(
        &mut self,
        ch: char,
        font: &Font,
        style: &ComputedValues,
        text: &str,
        byte_index: usize,
    ) {
        let font_name = font.postscript_name();
        let font_family = format_font_family(&style.font.font_family);
        let entry = self.0.entry(ch).or_insert_with(|| MissingGlyph {
            count: 0,
            fonts: vec![],
            font_families: vec![],
            context: context_around(text, byte_index),
        });
        entry.count += 1;
        if !entry.fonts.contains(&font_name) {
            entry.fonts.push(font_name);
        }
        if !entry.font_families.contains(&font_family) {
            entry.font_families.push(font_family);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A human-readable description of the missing characters, or `None` if there are none.
    pub fn report(&self) -> Option<String> {
        if self.0.is_empty() {
            return None;
        }
        let mut report = String::new();
        let num = self.0.len();
        let _ = writeln!(
            report,
            "warning: {} character{} cannot be rendered, because no installed font has a glyph for {}:",
            num,
            if num == 1 { "" } else { "s" },
            if num == 1 { "it" } else { "them" },
        );
        for (&ch, missing) in &self.0 {
            let _ = writeln!(
                report,
                "  {} ({} occurrence{}), e.g. in \"{}\"",
                describe_char(ch),
                missing.count,
                if missing.count == 1 { "" } else { "s" },
                missing.context,
            );
            let _ = writeln!(
                report,
                "      font-family: {}; font: {}",
                missing.font_families.join(" | "),
                missing.fonts.join(", "),
            );
        }
        let _ = write!(
            report,
            "  These characters are drawn as an empty box. \
             Install a font that covers them (e.g. Noto Sans Symbols 2, Noto Sans Math or a \
             Noto Sans CJK font) or add such a font to `font-family`."
        );
        Some(report)
    }
}

/// About 20 characters of `text` on either side of the character at `byte_index`, on one line.
fn context_around(text: &str, byte_index: usize) -> String {
    const RADIUS: usize = 20;
    let before: Vec<char> = text[..byte_index].chars().rev().take(RADIUS + 1).collect();
    let after: Vec<char> = text[byte_index..].chars().take(RADIUS + 2).collect();
    let mut context = String::new();
    if before.len() > RADIUS {
        context.push('…');
    }
    context.extend(before.iter().take(RADIUS).rev());
    context.extend(after.iter().take(RADIUS + 1));
    if after.len() > RADIUS + 1 {
        context.push('…');
    }
    context
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

fn format_font_family(families: &[FontFamily]) -> String {
    families
        .iter()
        .map(|family| match family {
            FontFamily::Serif => "serif".to_owned(),
            FontFamily::SansSerif => "sans-serif".to_owned(),
            FontFamily::Monospace => "monospace".to_owned(),
            FontFamily::Named(name) => format!("\"{}\"", name),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) use system::default_family;

#[cfg(all(
    feature = "fontconfig",
    not(any(target_os = "windows", target_os = "macos", target_os = "ios"))
))]
mod system {
    //! Finding a font for a character with fontconfig, like browsers do on Linux.

    use crate::style::properties::computed::{FontStyle, FontWeight, Language};
    use fontconfig_sys::constants::{
        FC_CHARSET, FC_FAMILY, FC_FILE, FC_INDEX, FC_LANG, FC_SLANT, FC_SLANT_ITALIC, FC_SLANT_OBLIQUE, FC_SLANT_ROMAN,
        FC_WEIGHT, FC_WEIGHT_BOLD, FC_WEIGHT_REGULAR,
    };
    use fontconfig_sys::*;
    use std::ffi::{CStr, c_char};
    use std::path::PathBuf;
    use std::ptr;

    /// The family that fontconfig substitutes for a generic family name such as `sans-serif`.
    pub fn default_family(generic: &str) -> Option<String> {
        let generic = std::ffi::CString::new(generic).ok()?;
        // SAFETY: the pattern and the match are destroyed before returning, and the family
        // name is copied before the match is destroyed.
        unsafe {
            let pattern = FcPatternCreate();
            if pattern.is_null() {
                return None;
            }
            FcPatternAddString(pattern, FC_FAMILY.as_ptr(), generic.as_ptr() as *const FcChar8);
            FcConfigSubstitute(ptr::null_mut(), pattern, FcMatchPattern);
            FcDefaultSubstitute(pattern);
            let mut result = FcResultMatch;
            let matched = FcFontMatch(ptr::null_mut(), pattern, &mut result);
            let mut family = None;
            if !matched.is_null() {
                let mut name = ptr::null_mut();
                if FcPatternGetString(matched, FC_FAMILY.as_ptr(), 0, &mut name) == FcResultMatch {
                    family = CStr::from_ptr(name as *const c_char).to_str().ok().map(str::to_owned);
                }
                FcPatternDestroy(matched);
            }
            FcPatternDestroy(pattern);
            family
        }
    }

    /// The file and face index of the installed font that fontconfig considers the best match
    /// for the weight and style among the fonts that cover `ch`.
    pub fn font_file_for_char(
        ch: char,
        weight: &FontWeight,
        style: &FontStyle,
        lang: &Language,
    ) -> Option<(PathBuf, u32)> {
        // the language makes fontconfig prefer fonts for it (e.g. among the CJK fonts)
        let lang = lang.tag().and_then(|tag| std::ffi::CString::new(tag).ok());
        let weight = match weight {
            FontWeight::Normal => FC_WEIGHT_REGULAR,
            FontWeight::Bold => FC_WEIGHT_BOLD,
        };
        let slant = match style {
            FontStyle::Normal => FC_SLANT_ROMAN,
            FontStyle::Italic => FC_SLANT_ITALIC,
            FontStyle::Oblique => FC_SLANT_OBLIQUE,
        };
        // SAFETY: every object that is created here is destroyed before returning, and the
        // strings that are read from the font set are copied before the set is destroyed.
        unsafe {
            let pattern = FcPatternCreate();
            if pattern.is_null() {
                return None;
            }
            let charset = FcCharSetCreate();
            FcCharSetAddChar(charset, ch as u32);
            FcPatternAddCharSet(pattern, FC_CHARSET.as_ptr(), charset);
            FcPatternAddInteger(pattern, FC_WEIGHT.as_ptr(), weight);
            FcPatternAddInteger(pattern, FC_SLANT.as_ptr(), slant);
            if let Some(lang) = &lang {
                let langset = FcLangSetCreate();
                FcLangSetAdd(langset, lang.as_ptr() as *const FcChar8);
                FcPatternAddLangSet(pattern, FC_LANG.as_ptr(), langset);
                FcLangSetDestroy(langset);
            }
            FcConfigSubstitute(ptr::null_mut(), pattern, FcMatchPattern);
            FcDefaultSubstitute(pattern);

            let mut result = FcResultMatch;
            let fonts = FcFontSort(ptr::null_mut(), pattern, 1, ptr::null_mut(), &mut result);
            let mut found = None;
            if !fonts.is_null() {
                for i in 0..(*fonts).nfont.max(0) as usize {
                    let font = *(*fonts).fonts.add(i);
                    let mut font_charset = ptr::null_mut();
                    if FcPatternGetCharSet(font, FC_CHARSET.as_ptr(), 0, &mut font_charset)
                        != FcResultMatch
                        || FcCharSetHasChar(font_charset, ch as u32) == 0
                    {
                        continue;
                    }
                    let mut file = ptr::null_mut();
                    if FcPatternGetString(font, FC_FILE.as_ptr(), 0, &mut file) != FcResultMatch {
                        continue;
                    }
                    let mut index = 0;
                    if FcPatternGetInteger(font, FC_INDEX.as_ptr(), 0, &mut index) != FcResultMatch
                    {
                        index = 0;
                    }
                    let path = CStr::from_ptr(file as *const c_char);
                    if let Ok(path) = path.to_str() {
                        found = Some((PathBuf::from(path), index.max(0) as u32));
                        break;
                    }
                }
                FcFontSetDestroy(fonts);
            }
            FcCharSetDestroy(charset);
            FcPatternDestroy(pattern);
            found
        }
    }
}

#[cfg(not(all(
    feature = "fontconfig",
    not(any(target_os = "windows", target_os = "macos", target_os = "ios"))
)))]
mod system {
    use crate::style::properties::computed::{FontStyle, FontWeight, Language};
    use std::path::PathBuf;

    /// Not implemented on this platform.
    pub fn default_family(_generic: &str) -> Option<String> {
        None
    }

    /// Not implemented on this platform; only `FALLBACK_FAMILIES` are tried.
    pub fn font_file_for_char(
        _ch: char,
        _weight: &FontWeight,
        _style: &FontStyle,
        _lang: &Language,
    ) -> Option<(PathBuf, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> FontFamily {
        FontFamily::Named(name.to_string())
    }

    #[test]
    fn han_characters_get_the_fonts_of_their_language() {
        let lang = Language::from_tag;
        assert_eq!(language_families('直', true, &lang("ja"))[0], "Noto Serif CJK JP");
        assert_eq!(language_families('直', true, &lang("zh"))[0], "Noto Serif CJK SC");
        assert_eq!(language_families('直', false, &lang("zh-Hans-CN"))[0], "Noto Sans CJK SC");
        assert_eq!(language_families('直', true, &lang("zh-Hant"))[0], "Noto Serif CJK TC");
        assert_eq!(language_families('直', true, &lang("zh-TW"))[0], "Noto Serif CJK TC");
        assert_eq!(language_families('直', true, &lang("zh-HK"))[0], "Noto Serif CJK HK");
        assert_eq!(language_families('直', false, &lang("ko"))[0], "Noto Sans CJK KR");
        // CJK punctuation follows the language as well
        assert_eq!(language_families('。', true, &lang("zh"))[0], "Noto Serif CJK SC");
        // an unknown language or another script leaves the choice to `script_families`
        assert!(language_families('直', true, &Language::default()).is_empty());
        assert!(language_families('a', true, &lang("ja")).is_empty());
    }

    #[test]
    fn urdu_gets_nastaliq() {
        let urdu = Language::from_tag("ur");
        assert_eq!(language_families('ک', true, &urdu)[0], "Noto Nastaliq Urdu");
        assert!(language_families('ک', true, &Language::from_tag("ar")).is_empty());
    }

    #[test]
    fn serif_text_is_recognized_by_the_family_names() {
        assert!(is_serif(&[named("Noto Serif")]));
        assert!(is_serif(&[named("EB Garamond"), FontFamily::SansSerif]));
        assert!(is_serif(&[named("Unknown Font"), FontFamily::Serif]));
        assert!(is_serif(&[named("Noto Naskh Arabic")]));
        assert!(!is_serif(&[named("Noto Sans")]));
        assert!(!is_serif(&[named("DejaVu Sans Mono"), FontFamily::Serif]));
        assert!(!is_serif(&[FontFamily::SansSerif]));
        assert!(!is_serif(&[]));
    }

    #[test]
    fn scripts_get_fonts_designed_for_them_in_the_matching_style() {
        assert_eq!(script_families('ك', true)[0], "Noto Naskh Arabic");
        assert_eq!(script_families('ك', false)[0], "Noto Sans Arabic");
        assert_eq!(script_families('\u{FEFB}', true)[0], "Noto Naskh Arabic");
        assert_eq!(script_families('א', true)[0], "Noto Serif Hebrew");
        assert_eq!(script_families('क', false)[0], "Noto Sans Devanagari");
        assert_eq!(script_families('か', true)[0], "Noto Serif CJK JP");
        assert_eq!(script_families('한', false)[0], "Noto Sans CJK KR");
        // Latin is covered by the general lists
        assert!(script_families('a', true).is_empty());
    }

    #[test]
    fn context_is_cut_around_the_character() {
        let text = "a long sentence with an arrow → pointing to the next sentence";
        let index = text.find('→').unwrap();
        assert_eq!(
            context_around(text, index),
            "…tence with an arrow → pointing to the nex…"
        );
        assert_eq!(context_around("x → y", 2), "x → y");
    }

    #[test]
    fn report_names_the_character_and_font() {
        let mut missing = MissingGlyphs::default();
        let mut style = ComputedValues::default();
        std::sync::Arc::make_mut(&mut style.font).font_family = vec![
            FontFamily::Named("Lato".to_owned()),
            FontFamily::Named("Noto Serif".to_owned()),
        ];
        let font = Font::bundled(&FontWeight::Normal);
        missing.record('→', &font, &style, "see → there", 4);
        missing.record('→', &font, &style, "see → there", 4);
        let report = missing.report().unwrap();
        assert!(
            report.contains("1 character cannot be rendered"),
            "{report}"
        );
        assert!(report.contains("U+2192 '→' (2 occurrences)"), "{report}");
        assert!(report.contains("\"see → there\""), "{report}");
        assert!(
            report.contains("font-family: \"Lato\", \"Noto Serif\""),
            "{report}"
        );
        assert!(report.contains("font: DejaVuSans"), "{report}");
    }

    #[test]
    fn missing_characters_get_a_fallback_font_or_are_reported() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = ComputedValues::default();
        let font = register_font(&mut font_map, Font::bundled(&FontWeight::Normal));
        // DejaVu Sans has no CJK and U+0378 is unassigned.
        assert!(!font.covers('漢'));
        resolver.ensure_coverage(&style, &font, "a 漢 b \u{0378}", &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert!(missing.0.contains_key(&'\u{0378}'));
        // Depends on the installed fonts: either a fallback was found, or the character was
        // reported as missing.
        let covered = font.fallbacks(&style.font.lang).iter().any(|f| f.covers('漢'));
        assert_ne!(covered, missing.0.contains_key(&'漢'));
        for fallback in font.fallbacks(&style.font.lang) {
            assert!(font_map.contains_key(&fallback.postscript_name()));
        }
    }
}
