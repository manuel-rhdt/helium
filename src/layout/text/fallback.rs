//! Font fallback: finding a font for the characters that the font chosen by `font-family` has no
//! glyph for.
//!
//! Fallback fonts are resolved during box construction, when the text of every run is known. For
//! every character of a text run that the run's font does not cover, the fonts below are tried in
//! order and the first one that has a glyph for the character is appended to the fallback fonts
//! of the run's font (`Font::fallbacks`):
//!
//! 1. the other families listed in `font-family` (as CSS prescribes),
//! 2. a fixed list of families with a broad character coverage (`FALLBACK_FAMILIES`),
//! 3. on Linux, any installed font that covers the character, as found by fontconfig.
//!
//! Shaping (`shape`) then splits the text into runs per font, and painting switches to the
//! fallback font for those runs. The fixed list comes before the fontconfig query so that the
//! output does not depend on the order fontconfig happens to sort fonts in, as far as possible.
//!
//! Characters that no font covers are collected in the `MissingGlyphs` resource and reported
//! to the user (see `MissingGlyphs::report`).

use super::font::Font;
use crate::layout::FontMap;
use crate::style::properties::computed::{ComputedValues, FontFamily, FontStyle, FontWeight};

use font_kit::family_name::FamilyName;
use font_kit::handle::Handle;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

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

/// Finds fallback fonts. One resolver is used for the whole document so that every font is
/// loaded at most once.
pub struct FontResolver {
    source: SystemSource,
    /// The installed font (if any) for a family, weight and style.
    family_cache: HashMap<(String, FontWeight, FontStyle), Option<Arc<Font>>>,
    /// The font that fontconfig found for a character, weight and style.
    char_cache: HashMap<(char, FontWeight, FontStyle), Option<Arc<Font>>>,
    missing: MissingGlyphs,
}

impl Default for FontResolver {
    fn default() -> Self {
        FontResolver {
            source: SystemSource::new(),
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
                || font.fallbacks.read().iter().any(|f| f.covers(ch))
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
                    font.add_fallback(fallback);
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

        let families = style
            .font
            .font_family
            .iter()
            .map(|family| FamilyName::from(family.clone()))
            .chain(
                FALLBACK_FAMILIES
                    .iter()
                    .map(|name| FamilyName::Title(name.to_string())),
            )
            .chain([FamilyName::SansSerif, FamilyName::Serif]);
        for family in families {
            if let Some(font) = self.family_font(family, &weight, &font_style, font_map)
                && font.covers(ch)
            {
                return Some(font);
            }
        }

        self.char_cache
            .entry((ch, weight.clone(), font_style.clone()))
            .or_insert_with(|| {
                system::font_file_for_char(ch, &weight, &font_style)
                    .and_then(|(path, index)| Font::external(Handle::from_path(path, index)))
                    .map(|font| register_font(font_map, font))
            })
            .clone()
            .filter(|font| font.covers(ch))
    }

    fn family_font(
        &mut self,
        family: FamilyName,
        weight: &FontWeight,
        style: &FontStyle,
        font_map: &mut FontMap,
    ) -> Option<Arc<Font>> {
        let source = &self.source;
        self.family_cache
            .entry((format!("{:?}", family), weight.clone(), style.clone()))
            .or_insert_with(|| {
                let mut properties = Properties::new();
                properties
                    .weight(weight.clone().into())
                    .style(style.clone().into());
                source
                    .select_best_match(&[family], &properties)
                    .ok()
                    .and_then(Font::external)
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
/// with the `.notdef` glyph of the font (usually an empty box), or as `?` with built-in fonts.
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
            "  These characters are drawn as an empty box (or '?' with built-in PDF fonts). \
             Install a font that covers them (e.g. Noto Sans Symbols 2, Noto Sans Math or \
             DejaVu Sans) or add such a font to `font-family`."
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

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
mod system {
    //! Finding a font for a character with fontconfig, like browsers do on Linux.

    use crate::style::properties::computed::{FontStyle, FontWeight};
    use fontconfig_sys::constants::{
        FC_CHARSET, FC_FILE, FC_INDEX, FC_SLANT, FC_SLANT_ITALIC, FC_SLANT_OBLIQUE, FC_SLANT_ROMAN,
        FC_WEIGHT, FC_WEIGHT_BOLD, FC_WEIGHT_REGULAR,
    };
    use fontconfig_sys::*;
    use std::ffi::{CStr, c_char};
    use std::path::PathBuf;
    use std::ptr;

    /// The file and face index of the installed font that fontconfig considers the best match
    /// for the weight and style among the fonts that cover `ch`.
    pub fn font_file_for_char(
        ch: char,
        weight: &FontWeight,
        style: &FontStyle,
    ) -> Option<(PathBuf, u32)> {
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

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "ios"))]
mod system {
    use crate::style::properties::computed::{FontStyle, FontWeight};
    use std::path::PathBuf;

    /// Not implemented on this platform; only `FALLBACK_FAMILIES` are tried.
    pub fn font_file_for_char(
        _ch: char,
        _weight: &FontWeight,
        _style: &FontStyle,
    ) -> Option<(PathBuf, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::font::BuiltInFont;
    use super::*;

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
        let font = Font::built_in(BuiltInFont::Helvetica);
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
        assert!(report.contains("font: Helvetica"), "{report}");
    }

    #[test]
    fn builtin_font_falls_back_for_characters_outside_win_ansi() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = ComputedValues::default();
        let font = register_font(&mut font_map, Font::built_in(BuiltInFont::Helvetica));
        assert!(!font.covers('→'));
        resolver.ensure_coverage(&style, &font, "a → b", &mut font_map);
        // Depends on the installed fonts: either a fallback was found, or the character was
        // reported as missing.
        let covered = font.fallbacks().iter().any(|f| f.covers('→'));
        let missing = resolver.take_missing_glyphs();
        assert_ne!(covered, missing.0.contains_key(&'→'));
        for fallback in font.fallbacks() {
            assert!(font_map.contains_key(&fallback.postscript_name()));
        }
    }
}
