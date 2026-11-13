//! Font fallback: finding a font for the text that the fonts of `font-family` have no glyphs for.
//!
//! Fallback is a pure function of the *font description* (`FallbackKey`: the `font-family` list,
//! weight, style and language) and of a grapheme cluster with its script. It does not depend on
//! what other text is in the document or in which order it is laid out, and every font
//! description has its own fallback fonts (`Font::fallbacks`).
//!
//! The text is itemized first (`segment::clusters`): grapheme clusters, each with a script
//! (UAX #24: `Common` and `Inherited` characters take the script of their run, honouring
//! `Script_Extensions`). Every cluster that the primary font does not cover completely is
//! resolved during box construction. The fonts below are tried in order, and the first one that
//! has glyphs for the whole cluster wins; if there is none, the first that has a glyph for its
//! base character does:
//!
//! 1. the families of `font-family`, in order (as CSS Fonts 4 §5 prescribes). A generic family
//!    (`serif`, `sans-serif`, `monospace`) is resolved for the script of the cluster at its
//!    position in the list: the fonts designed for the script in that design come first, like
//!    the generic font settings per script of browsers,
//! 2. the fonts designed for the script (and language) in the design of the text (`Class`),
//!    then in the other designs, and for serif text general serif families (`SERIF_FAMILIES`),
//! 3. a fixed list of families with a broad character coverage (`FALLBACK_FAMILIES`),
//! 4. the font built into Helium (DejaVu Sans, see `Font::bundled`); the Noto fonts embedded for
//!    other scripts (`EMBEDDED_FONTS`, subset by `build.rs`) are found by name in step 2 like
//!    installed fonts, which take precedence over them,
//! 5. on Linux, any installed font that covers the cluster, as found by fontconfig.
//!
//! The design of the text (`Class`) is the generic family of the first generic in `font-family`.
//! Without one it is taken from the primary font itself: the family class and PANOSE data of its
//! `OS/2` table, unless the font is listed with corrected metadata (`CLASS_CORRECTIONS`).
//!
//! Shaping (`shape`) then splits the text into runs per font, and painting switches to the
//! fallback font for those runs. The fixed lists and the bundled font come before the fontconfig
//! query so that the output does not depend on the order fontconfig happens to sort fonts in, as
//! far as possible.
//!
//! Characters that no font covers are collected in the `MissingGlyphs` resource and reported
//! to the user (see `MissingGlyphs::report`).

use super::font::{Font, system_font, use_system_fonts};
use super::fallback_data::*;
use super::segment::{clusters, needs_font};
use crate::layout::FontMap;
use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, Language,
};

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use unicode_script::Script;

/// Everything that decides the fallback fonts of a text: the `font-family` list, weight, style
/// and language. Fallback fonts are a function of this and the character alone.
#[derive(Debug, Clone, Default)]
pub struct FallbackKey {
    pub lang: Language,
    families: Arc<[FontFamily]>,
    weight: FontWeight,
    style: FontStyle,
    /// Identifies `families`, `weight` and `style`.
    descriptor: Arc<str>,
}

impl FallbackKey {
    pub fn new(style: &ComputedValues) -> Self {
        let font = &style.font;
        let descriptor = format!(
            "{}|{:?}|{:?}",
            format_font_family(&font.font_family),
            font.font_weight,
            font.font_style
        );
        FallbackKey {
            lang: font.lang.clone(),
            families: font.font_family.clone().into(),
            weight: font.font_weight.clone(),
            style: font.font_style.clone(),
            descriptor: descriptor.into(),
        }
    }

    fn id(&self) -> (&Language, &str) {
        (&self.lang, &self.descriptor)
    }
}

impl PartialEq for FallbackKey {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for FallbackKey {}

impl Hash for FallbackKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state)
    }
}

impl PartialOrd for FallbackKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FallbackKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id().cmp(&other.id())
    }
}

/// The fonts of one text (the text of a `TextRun`): the font description, which decides the
/// fallback fonts, and the font of every part of the text.
///
/// A script run is rendered with one font if a single font covers all of it (CSS Fonts 4 asks
/// for the first one of the family list that does), rather than switching fonts at every
/// cluster that the primary font lacks. The runs are computed once for the whole text when its
/// boxes are built (`FontResolver::ensure_coverage`) and stored with it, so that measuring the
/// text and drawing its pieces of a line, which are shaped separately, always use the same
/// fonts. Text that is not part of it (a hyphen) uses the fonts of the clusters alone.
#[derive(Debug, Clone, Default)]
pub struct TextFonts {
    pub key: FallbackKey,
    /// Sorted, non-overlapping runs of the text; empty if the primary font is used throughout
    /// and for text without runs.
    runs: Arc<[FontRun]>,
    /// The `letter-spacing` of the text in 1/1000 em. Optional ligatures are not used if it is
    /// not zero.
    pub letter_spacing: f32,
}

/// The bytes `start..end` of a text are rendered with the font with the index `font` (as in
/// `GlyphInfo::font`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontRun {
    pub start: usize,
    pub end: usize,
    pub font: u16,
}

impl TextFonts {
    /// Fonts for text without known runs: every cluster gets the font chosen for it.
    pub fn without_runs(key: FallbackKey) -> Self {
        TextFonts { key, runs: Arc::from([]), letter_spacing: 0.0 }
    }

    pub fn runs(&self) -> &[FontRun] {
        &self.runs
    }

    /// The index of the font of the text at the byte `offset`, if the text has runs there.
    pub fn font_at(&self, offset: usize) -> Option<u16> {
        let index = self.runs.partition_point(|run| run.end <= offset);
        self.runs
            .get(index)
            .filter(|run| run.start <= offset)
            .map(|run| run.font)
    }
}

/// The fonts of the clusters of one script run, as indices into `set` (`0` is `primary`): the
/// one font that covers the most of the run, ties going to the one that comes first (`primary`,
/// then the fonts in the order of their first cluster); clusters that it lacks get their own.
fn script_run_fonts(
    primary: &Font,
    set: &super::font::FallbackSet,
    clusters: &[super::segment::Cluster<'_>],
) -> Vec<u16> {
    let own: Vec<u16> = clusters
        .iter()
        .map(|cluster| set.index_for(primary, cluster.text, cluster.script))
        .collect();
    let mut candidates = vec![0u16];
    for &index in &own {
        if !candidates.contains(&index) {
            candidates.push(index);
        }
    }
    let covered = |candidate: u16| {
        let font = set.font_at(primary, candidate);
        clusters
            .iter()
            .filter(|cluster| font.covers_cluster(cluster.text))
            .count()
    };
    let mut best = (0, covered(0));
    for &candidate in &candidates[1..] {
        let count = covered(candidate);
        if count > best.1 {
            best = (candidate, count);
        }
    }
    let font = set.font_at(primary, best.0);
    clusters
        .iter()
        .zip(own)
        .map(|(cluster, own)| if font.covers_cluster(cluster.text) { best.0 } else { own })
        .collect()
}

/// The design of a text, which decides the design of the fonts for the scripts that its own
/// font does not cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Serif,
    Sans,
    Mono,
}

impl Class {
    fn of_generic(family: &FontFamily) -> Option<Class> {
        match family {
            FontFamily::Serif => Some(Class::Serif),
            FontFamily::SansSerif => Some(Class::Sans),
            FontFamily::Monospace => Some(Class::Mono),
            FontFamily::Named(_) => None,
        }
    }

    fn generic(self) -> FontFamily {
        match self {
            Class::Serif => FontFamily::Serif,
            Class::Sans => FontFamily::SansSerif,
            Class::Mono => FontFamily::Monospace,
        }
    }
}

/// The CJK variants for `script` in text of language `lang`, in order of preference: the ones of
/// the language, then the others (so that a Japanese font is found for Han characters in text of
/// an unknown language, as fontconfig prefers them by default).
///
/// Kana and Hangul have a variant of their own whatever the language. For Chinese the script
/// subtag decides between simplified and traditional forms (`zh-Hans-HK` is simplified), then
/// the region: Hong Kong and Macao (and Cantonese) have their own traditional forms, Taiwan the
/// standard traditional ones.
fn cjk_variants(script: Script, lang: &Language) -> Vec<&'static CjkVariant> {
    let mut variants: Vec<&'static CjkVariant> = match script {
        Script::Hiragana | Script::Katakana => vec![&JP],
        Script::Hangul => vec![&KR],
        _ => vec![],
    };
    match lang.primary() {
        Some("ja") => variants.push(&JP),
        Some("ko") => variants.push(&KR),
        Some(primary @ ("zh" | "yue" | "cmn" | "wuu" | "gan" | "hsn" | "hak" | "nan" | "lzh")) => {
            let region = lang.region();
            let hong_kong = region == Some("hk") || region == Some("mo") || primary == "yue";
            let traditional = region == Some("tw") || matches!(primary, "nan" | "lzh");
            match lang.script() {
                Some("hans") => variants.extend([&SC, &TC]),
                Some("hant") if hong_kong => variants.extend([&HK, &TC, &SC]),
                Some("hant") => variants.extend([&TC, &HK, &SC]),
                _ if hong_kong => variants.extend([&HK, &TC, &SC]),
                _ if traditional => variants.extend([&TC, &HK, &SC]),
                _ => variants.extend([&SC, &TC]),
            }
        }
        _ => {}
    }
    variants.extend([&JP, &SC, &TC]);
    let mut seen: Vec<*const CjkVariant> = Vec::new();
    variants.retain(|variant| {
        let ptr = *variant as *const CjkVariant;
        let new = !seen.contains(&ptr);
        seen.push(ptr);
        new
    });
    variants
}

fn is_cjk(script: Script) -> bool {
    matches!(
        script,
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul | Script::Bopomofo
    )
}

/// The designs of the fonts for text of a design, in order of preference: its own, then
/// sans-serif (a monospace text without a monospace font of the script, serif text without a
/// serif one).
fn designs(class: Class) -> &'static [Class] {
    match class {
        Class::Serif => &[Class::Serif, Class::Sans],
        Class::Sans => &[Class::Sans],
        Class::Mono => &[Class::Mono, Class::Sans],
    }
}

/// The CJK families for text in the design `class`: for every variant in the order of
/// `cjk_variants`, the fonts of that design and then those of the designs that replace it. A
/// font of the language of the text, even in another design, comes before one for another
/// language: its forms of the Han characters would be wrong.
fn cjk_families(script: Script, class: Class, lang: &Language) -> Vec<&'static str> {
    let mut families = Vec::new();
    for variant in cjk_variants(script, lang) {
        for design in designs(class) {
            families.extend(variant.fonts(*design));
        }
    }
    families
}

/// Families designed for `script`, in order of preference, for text in the design `class` and the
/// language `lang`. Latin and other scripts that the general lists cover have none.
///
/// Characters of the `Common` script have been itemized into the script of their run (see
/// `segment::clusters`), so that e.g. `、` and `。` get a Han font and the Arabic comma a Naskh
/// font. A monospace design has fonts of its own only for CJK; elsewhere it uses the sans-serif
/// ones. For CJK these are the fonts of that design only, in the order of `cjk_variants`.
fn script_families(script: Script, class: Class, lang: &Language) -> Vec<&'static str> {
    match script {
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul | Script::Bopomofo => {
            // only the design asked for (see `cjk_families` for the replacements)
            return cjk_variants(script, lang)
                .into_iter()
                .flat_map(|variant| variant.fonts(class))
                .copied()
                .collect();
        }
        Script::Arabic => {
            let (language, general) = match lang.primary() {
                Some("ur") => (URDU, ARABIC),
                Some("fa") => (PERSIAN, ARABIC),
                _ => (ARABIC_NONE, ARABIC),
            };
            return match class {
                Class::Serif => [language.0, general.0].concat(),
                _ => [language.1, general.1].concat(),
            };
        }
        _ => {}
    }
    match SCRIPT_FAMILIES.iter().find(|(s, ..)| *s == script) {
        Some((_, serif, sans)) => match class {
            Class::Serif => serif.to_vec(),
            Class::Sans | Class::Mono => sans.to_vec(),
        },
        None => Vec::new(),
    }
}

/// The class of a font from the classification in its `OS/2` table: `sFamilyClass` (its high
/// byte) and the PANOSE digits `bFamilyType`, `bSerifStyle` and `bProportion`.
///
/// These describe Latin typefaces only, so PANOSE is used only for Latin Text (`bFamilyType` 2)
/// with a serif style that is filled in; many fonts leave it zero. `sFamilyClass` is the second
/// choice.
fn class_from_metadata(
    family_type: u8,
    serif_style: u8,
    proportion: u8,
    family_class: u8,
) -> Option<Class> {
    if family_type == 2 {
        if proportion == 9 {
            return Some(Class::Mono);
        }
        match serif_style {
            // cove, square, thin, oval, exaggerated, triangle ...
            2..=10 => return Some(Class::Serif),
            // normal, obtuse and perpendicular sans, flared, rounded
            11..=15 => return Some(Class::Sans),
            _ => {}
        }
    }
    match family_class {
        1..=5 | 7 => Some(Class::Serif),
        8 => Some(Class::Sans),
        _ => None,
    }
}

/// The class of the metadata of `font` (`class_from_metadata`), unless the font is listed in
/// `CLASS_CORRECTIONS` as having wrong metadata.
fn classify(font: &Font) -> Option<Class> {
    let postscript_name = font.postscript_name();
    CLASS_CORRECTIONS
        .iter()
        .find(|correction| correction.postscript_name == postscript_name)
        .map(|correction| correction.class)
        .or_else(|| metadata_class(font))
}

/// The class that the `OS/2` table of `font` says, if any.
fn metadata_class(font: &Font) -> Option<Class> {
    let table = font.hb_font.face().table_with_tag(b"OS/2")?;
    let data: &[u8] = table.as_ref();
    (data.len() >= 42)
        .then(|| class_from_metadata(data[32], data[33], data[35], data[30]))
        .flatten()
}

/// The families to try, in this order, for a cluster of `script` in text of the font description
/// `key` and the design `class` (steps 1 to 3 of the module documentation).
fn candidate_families(key: &FallbackKey, class: Class, script: Script) -> Vec<FontFamily> {
    let lang = &key.lang;
    let named = |names: Vec<&'static str>| -> Vec<FontFamily> {
        names.into_iter().map(|name| FontFamily::Named(name.to_owned())).collect()
    };
    let mut families: Vec<FontFamily> = Vec::new();
    // the families of `font-family`; a generic family is resolved for the script
    for family in key.families.iter() {
        match Class::of_generic(family) {
            Some(generic) => {
                families.extend(named(script_families(script, generic, lang)));
                families.push(family.clone());
            }
            None => families.push(family.clone()),
        }
    }
    // the fonts for the script in the design of the text, then in the others
    if is_cjk(script) {
        families.extend(named(cjk_families(script, class, lang)));
    } else {
        for design in designs(class) {
            families.extend(named(script_families(script, *design, lang)));
        }
    }
    if class == Class::Serif {
        families.extend(SERIF_FAMILIES.iter().map(|name| FontFamily::Named(name.to_string())));
    }
    families.extend(FALLBACK_FAMILIES.iter().map(|name| FontFamily::Named(name.to_string())));
    families.extend([class.generic(), FontFamily::SansSerif, FontFamily::Serif]);
    families
}

/// Finds fallback fonts. One resolver is used for the whole document so that every font is
/// loaded at most once.
#[derive(Default)]
pub struct FontResolver {
    /// The installed font (if any) for a family, weight and style.
    family_cache: HashMap<(String, FontWeight, FontStyle), Option<Arc<Font>>>,
    /// The font that fontconfig found for a character, weight and style.
    char_cache: HashMap<(char, FontWeight, FontStyle, Language), Option<Arc<Font>>>,
    /// The fallback font for a cluster in a font description.
    cluster_cache: HashMap<(FallbackKey, String, Script), Option<Arc<Font>>>,
    class_cache: HashMap<FallbackKey, Class>,
    missing: MissingGlyphs,
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

impl FontResolver {
    /// Makes sure that every cluster of `text`, which is rendered with `font` (the font matched
    /// for `style`), is covered by `font` or by a fallback font of `font` for the font
    /// description of `style`, adding fallback fonts where necessary. Characters that no
    /// installed font covers are recorded as missing.
    pub fn ensure_coverage(
        &mut self,
        style: &ComputedValues,
        font: &Font,
        text: &str,
        font_map: &mut FontMap,
    ) -> TextFonts {
        let key = FallbackKey::new(style);
        let all_clusters = clusters(text);
        for cluster in &all_clusters {
            if !cluster.text.chars().any(needs_font) || font.covers_cluster(cluster.text) {
                continue;
            }
            let cache_key = (key.clone(), cluster.text.to_owned(), cluster.script);
            let fallback = match self.cluster_cache.get(&cache_key) {
                Some(fallback) => fallback.clone(),
                None => {
                    let fallback = self.find_fallback(cluster.text, cluster.script, &key, font, font_map);
                    self.cluster_cache.insert(cache_key, fallback.clone());
                    fallback
                }
            };
            let chosen = fallback.as_deref().unwrap_or(font);
            if let Some(fallback) = &fallback {
                if font.add_fallback(&key, cluster.text, cluster.script, fallback.clone()) {
                    log::info!(
                        "Font {} has no glyph for {}; using {} instead.",
                        font.postscript_name(),
                        describe_char(cluster.text.chars().find(|c| needs_font(*c)).unwrap()),
                        fallback.postscript_name()
                    );
                }
            }
            for (offset, ch) in cluster.text.char_indices() {
                if needs_font(ch) && !chosen.covers(ch) {
                    self.missing.record(ch, font, style, text, cluster.start + offset);
                }
            }
        }

        // The font of every script run
        let set = font.fallback_set(&key);
        let mut runs: Vec<FontRun> = Vec::new();
        let mut start = 0;
        while start < all_clusters.len() {
            let script = all_clusters[start].script;
            let end = start
                + all_clusters[start..]
                    .iter()
                    .take_while(|cluster| cluster.script == script)
                    .count();
            let group = &all_clusters[start..end];
            for (cluster, font_index) in group.iter().zip(script_run_fonts(font, &set, group)) {
                let cluster_end = cluster.start + cluster.text.len();
                match runs.last_mut() {
                    Some(last) if last.font == font_index && last.end == cluster.start => {
                        last.end = cluster_end
                    }
                    _ => runs.push(FontRun { start: cluster.start, end: cluster_end, font: font_index }),
                }
            }
            start = end;
        }
        if runs.iter().all(|run| run.font == 0) {
            runs.clear();
        }
        TextFonts { key, runs: runs.into(), letter_spacing: 0.0 }
    }

    /// The characters that could not be rendered with any font.
    pub fn take_missing_glyphs(&mut self) -> MissingGlyphs {
        std::mem::take(&mut self.missing)
    }

    /// The design of the text of `key`, whose primary font is `primary`: the generic family it
    /// lists first or else the class of the primary font.
    fn class(&mut self, key: &FallbackKey, primary: &Font) -> Class {
        if let Some(class) = self.class_cache.get(key) {
            return *class;
        }
        let class = key
            .families
            .iter()
            .find_map(Class::of_generic)
            .or_else(|| classify(primary))
            .unwrap_or(Class::Sans);
        self.class_cache.insert(key.clone(), class);
        class
    }

    /// The font for the cluster, a pure function of the font description and the cluster: the
    /// first font that covers all of it or else its base character, in the order described in
    /// the module documentation. `None` if there is none or the primary font is the best.
    fn find_fallback(
        &mut self,
        cluster: &str,
        script: Script,
        key: &FallbackKey,
        primary: &Font,
        font_map: &mut FontMap,
    ) -> Option<Arc<Font>> {
        let class = self.class(key, primary);
        let lang = &key.lang;
        let families = candidate_families(key, class, script);

        let base = cluster.chars().find(|c| needs_font(*c))?;
        let mut base_only: Option<Arc<Font>> = None;
        for family in families {
            let Some(font) = self.family_font(family, &key.weight, &key.style, font_map) else {
                continue;
            };
            if font.covers_cluster(cluster) {
                return Some(font);
            }
            if base_only.is_none() && font.covers(base) {
                base_only = Some(font);
            }
        }

        let bundled = register_font(font_map, Font::bundled(&key.weight));
        if bundled.covers_cluster(cluster) {
            return Some(bundled);
        }
        if base_only.is_none() && bundled.covers(base) {
            base_only = Some(bundled);
        }

        if use_system_fonts() {
            let (weight, style) = (&key.weight, &key.style);
            let found = self
                .char_cache
                .entry((base, weight.clone(), style.clone(), lang.clone()))
                .or_insert_with(|| {
                    system::font_file_for_char(base, weight, style, lang)
                        .and_then(|(path, index)| {
                            Font::load(Arc::new(std::fs::read(path).ok()?), index)
                        })
                        .map(|font| register_font(font_map, font))
                })
                .clone()
                .filter(|font| font.covers(base));
            if let Some(found) = found {
                if found.covers_cluster(cluster) {
                    return Some(found);
                }
                base_only = base_only.or(Some(found));
            }
        }
        // no font covers the whole cluster: the first with its base character; it is not
        // a fallback if that is the primary font
        base_only.filter(|font| font.postscript_name != primary.postscript_name)
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
                system_font(&[family], weight, style).map(|font| register_font(font_map, font))
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
        FC_WEIGHT,
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

    /// The fontconfig weight of an OpenType weight, interpolated between the values that
    /// fontconfig defines (`FcWeightFromOpenType`).
    fn fc_weight(weight: f32) -> i32 {
        const TABLE: [(f32, f32); 13] = [
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 40.0),
            (300.0, 50.0),
            (350.0, 55.0),
            (380.0, 75.0),
            (400.0, 80.0),
            (500.0, 100.0),
            (600.0, 180.0),
            (700.0, 200.0),
            (800.0, 205.0),
            (900.0, 210.0),
            (1000.0, 215.0),
        ];
        let weight = weight.clamp(0.0, 1000.0);
        let i = TABLE.iter().position(|&(ot, _)| ot >= weight).unwrap_or(TABLE.len() - 1).max(1);
        let ((a, fa), (b, fb)) = (TABLE[i - 1], TABLE[i]);
        (fa + (fb - fa) * (weight - a) / (b - a)).round() as i32
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
        let weight = fc_weight(weight.0);
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

    fn style_with(families: Vec<FontFamily>, lang: &str) -> ComputedValues {
        let mut style = ComputedValues::default();
        let font = std::sync::Arc::make_mut(&mut style.font);
        font.font_family = families;
        font.lang = Language::from_tag(lang);
        style
    }

    fn primary(font_map: &mut FontMap, style: &ComputedValues) -> Arc<Font> {
        register_font(
            font_map,
            system_font(&style.font.font_family, &FontWeight::NORMAL, &FontStyle::Normal).unwrap(),
        )
    }

    #[test]
    fn han_characters_get_the_fonts_of_their_language() {
        let lang = Language::from_tag;
        let first = |script, class, tag: &str| script_families(script, class, &lang(tag))[0];
        assert_eq!(first(Script::Han, Class::Serif, "ja"), "Noto Serif CJK JP");
        assert_eq!(first(Script::Han, Class::Serif, "zh"), "Noto Serif CJK SC");
        assert_eq!(first(Script::Han, Class::Sans, "zh-Hans-CN"), "Noto Sans CJK SC");
        assert_eq!(first(Script::Han, Class::Serif, "zh-Hant"), "Noto Serif CJK TC");
        assert_eq!(first(Script::Han, Class::Serif, "zh-TW"), "Noto Serif CJK TC");
        assert_eq!(first(Script::Han, Class::Serif, "zh-HK"), "Noto Serif CJK HK");
        assert_eq!(first(Script::Han, Class::Sans, "ko"), "Noto Sans CJK KR");
        // an unknown language gets the Japanese forms first
        assert_eq!(first(Script::Han, Class::Serif, ""), "Noto Serif CJK JP");
        // kana and Hangul are not affected by the language of the text
        assert_eq!(first(Script::Hiragana, Class::Serif, "zh"), "Noto Serif CJK JP");
        assert_eq!(first(Script::Hangul, Class::Sans, "ja"), "Noto Sans CJK KR");
        // other scripts have no CJK fonts
        assert!(script_families(Script::Latin, Class::Serif, &lang("ja")).is_empty());
    }

    #[test]
    fn monospace_text_gets_monospace_cjk_fonts() {
        let ja = Language::from_tag("ja");
        let strict = script_families(Script::Han, Class::Mono, &ja);
        assert_eq!(strict[0], "Noto Sans Mono CJK JP");
        // only monospace fonts: the replacements are the business of `cjk_families`
        assert!(strict.iter().all(|name| name.contains("Mono") || name.contains("Code")));
        let all = cjk_families(Script::Han, Class::Mono, &ja);
        assert!(all.contains(&"Noto Sans CJK JP"));
    }

    #[test]
    fn urdu_gets_nastaliq() {
        let urdu = Language::from_tag("ur");
        assert_eq!(script_families(Script::Arabic, Class::Serif, &urdu)[0], "Noto Nastaliq Urdu");
        let arabic = Language::from_tag("ar");
        assert_eq!(script_families(Script::Arabic, Class::Serif, &arabic)[0], "Noto Naskh Arabic");
    }

    #[test]
    fn scripts_get_fonts_designed_for_them_in_the_matching_style() {
        let lang = Language::default();
        assert_eq!(script_families(Script::Arabic, Class::Sans, &lang)[0], "Noto Sans Arabic");
        assert_eq!(script_families(Script::Hebrew, Class::Serif, &lang)[0], "Noto Serif Hebrew");
        assert_eq!(script_families(Script::Devanagari, Class::Sans, &lang)[0], "Noto Sans Devanagari");
        // Latin is covered by the general lists
        assert!(script_families(Script::Latin, Class::Serif, &lang).is_empty());
    }

    #[test]
    fn metadata_classifies_latin_text_faces() {
        // (bFamilyType, bSerifStyle, bProportion, sFamilyClass high byte)
        let meta = class_from_metadata;
        assert_eq!(meta(2, 2, 3, 0), Some(Class::Serif)); // cove
        assert_eq!(meta(2, 10, 3, 0), Some(Class::Serif));
        assert_eq!(meta(2, 11, 3, 0), Some(Class::Sans)); // normal sans
        assert_eq!(meta(2, 13, 3, 0), Some(Class::Sans));
        assert_eq!(meta(2, 15, 3, 0), Some(Class::Sans)); // rounded
        assert_eq!(meta(2, 2, 9, 0), Some(Class::Mono));
        // serif style 0 (any) or 1 (no fit) says nothing
        assert_eq!(meta(2, 0, 3, 0), None);
        assert_eq!(meta(2, 1, 3, 0), None);
        // only Latin Text is described by the serif style
        assert_eq!(meta(0, 0, 0, 0), None);
        assert_eq!(meta(3, 2, 0, 0), None); // Latin Script
        assert_eq!(meta(4, 11, 0, 0), None); // Latin Decorative
        assert_eq!(meta(3, 2, 9, 0), None);
        // the family class decides when PANOSE does not
        assert_eq!(meta(0, 0, 0, 1), Some(Class::Serif)); // oldstyle serifs
        assert_eq!(meta(0, 0, 0, 5), Some(Class::Serif)); // clarendon
        assert_eq!(meta(0, 0, 0, 7), Some(Class::Serif)); // freeform serifs
        assert_eq!(meta(0, 0, 0, 8), Some(Class::Sans));
        assert_eq!(meta(0, 0, 0, 10), None); // scripts
        assert_eq!(meta(0, 0, 0, 12), None); // symbolic
        // PANOSE wins over the family class
        assert_eq!(meta(2, 11, 3, 1), Some(Class::Sans));
    }

    #[test]
    fn corrections_replace_wrong_metadata() {
        let load = |name: &str| {
            system_font(&[named(name)], &FontWeight::NORMAL, &FontStyle::Normal).unwrap()
        };
        // Noto Naskh Arabic claims "normal sans" in its PANOSE data
        let naskh = load("Noto Naskh Arabic");
        assert_eq!(metadata_class(&naskh), Some(Class::Sans));
        assert_eq!(classify(&naskh), Some(Class::Serif));
        // TeX Gyre leaves the metadata zero
        assert_eq!(metadata_class(&load("TeX Gyre Termes")), None);
        assert_eq!(classify(&load("TeX Gyre Termes")), Some(Class::Serif));
        assert_eq!(classify(&load("TeX Gyre Cursor")), Some(Class::Mono));
        // fonts that are not listed have the class of their metadata
        assert_eq!(classify(&load("Noto Serif Hebrew")), Some(Class::Serif));
        assert_eq!(classify(&load("Noto Sans Hebrew")), Some(Class::Sans));
        assert_eq!(classify(&load("TeX Gyre Heros")), None);
    }

    /// A correction for a font that Helium ships must still be needed: if the metadata gives the
    /// class already, the entry is stale.
    #[test]
    fn corrections_of_bundled_fonts_are_needed() {
        let mut seen = std::collections::HashSet::new();
        for correction in CLASS_CORRECTIONS {
            assert!(seen.insert(correction.postscript_name), "{} is listed twice", correction.postscript_name);
        }
        for data in super::super::font::bundled_font_data() {
            let Some(font) = Font::load(Arc::new(data), 0) else { continue };
            let name = font.postscript_name();
            if let Some(correction) = CLASS_CORRECTIONS.iter().find(|c| c.postscript_name == name) {
                assert_ne!(
                    metadata_class(&font),
                    Some(correction.class),
                    "the metadata of {name} is right: remove its correction"
                );
            }
        }
    }

    #[test]
    fn the_first_generic_family_decides_the_class() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Heros"), FontFamily::Monospace, FontFamily::Serif], "");
        let font = primary(&mut font_map, &style);
        assert_eq!(resolver.class(&FallbackKey::new(&style), &font), Class::Mono);
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        assert_eq!(resolver.class(&FallbackKey::new(&style), &font), Class::Serif);
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
        let style = style_with(vec![named("Lato"), named("Noto Serif")], "");
        let font = Font::bundled(&FontWeight::NORMAL);
        missing.record('→', &font, &style, "see → there", 4);
        missing.record('→', &font, &style, "see → there", 4);
        let report = missing.report().unwrap();
        assert!(report.contains("1 character cannot be rendered"), "{report}");
        assert!(report.contains("U+2192 '→' (2 occurrences)"), "{report}");
        assert!(report.contains("\"see → there\""), "{report}");
        assert!(report.contains("font-family: \"Lato\", \"Noto Serif\""), "{report}");
        assert!(report.contains("font: DejaVuSans"), "{report}");
    }

    #[test]
    fn missing_characters_get_a_fallback_font_or_are_reported() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = ComputedValues::default();
        let key = FallbackKey::new(&style);
        let font = register_font(&mut font_map, Font::bundled(&FontWeight::NORMAL));
        // DejaVu Sans has no CJK and U+0378 is unassigned.
        assert!(!font.covers('漢'));
        resolver.ensure_coverage(&style, &font, "a 漢 b \u{0378}", &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert!(missing.0.contains_key(&'\u{0378}'));
        // Unit tests use only the built-in fonts, which include a CJK font.
        assert!(font.fallbacks(&key).iter().any(|f| f.covers('漢')));
        assert!(!missing.0.contains_key(&'漢'));
        for fallback in font.fallbacks(&key) {
            assert!(font_map.contains_key(&fallback.postscript_name()));
        }
    }

    /// Text in a serif font gets the embedded sans-serif fonts for scripts that no serif font
    /// covers, rather than an empty box: without installed fonts, they are all there is.
    #[test]
    fn serif_text_falls_back_to_the_embedded_sans_serif_fonts() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "");
        let key = FallbackKey::new(&style);
        let font = primary(&mut font_map, &style);
        let text = "道可道，非常道。 ქართული ไทย ខ្មែរ";
        resolver.ensure_coverage(&style, &font, text, &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert!(missing.is_empty(), "{}", missing.report().unwrap_or_default());
        let fallbacks = font.fallbacks(&key);
        assert!(fallbacks.iter().any(|f| f.postscript_name() == "NotoSansCJKjp-Regular"));
    }

    /// The fallback fonts of a description do not depend on which other descriptions with the
    /// same primary font were resolved before: a family that the element lists itself wins over
    /// the fallback of another element.
    #[test]
    fn fallback_does_not_depend_on_other_elements() {
        let text = "道可道 ქართული";
        let elements = [
            style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], ""),
            style_with(vec![named("TeX Gyre Termes"), named("Noto Sans Georgian")], ""),
        ];
        let resolve = |order: &[usize]| {
            let mut font_map = FontMap::new();
            let mut resolver = FontResolver::default();
            let mut names = Vec::new();
            for &i in order {
                let font = primary(&mut font_map, &elements[i]);
                resolver.ensure_coverage(&elements[i], &font, text, &mut font_map);
                let key = FallbackKey::new(&elements[i]);
                let set = font.fallback_set(&key);
                let chosen: Vec<_> = clusters(text)
                    .iter()
                    .map(|c| (i, set.index_for(&font, c.text, c.script).checked_sub(1).map(|n| set.fonts[n as usize].postscript_name())))
                    .collect();
                names.push(chosen);
            }
            names.sort();
            names
        };
        assert_eq!(resolve(&[0, 1]), resolve(&[1, 0]));
        // and the explicitly listed family is used for Georgian by the second element
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        for style in &elements {
            let font = primary(&mut font_map, style);
            resolver.ensure_coverage(style, &font, text, &mut font_map);
        }
        let font = primary(&mut font_map, &elements[1]);
        let set = font.fallback_set(&FallbackKey::new(&elements[1]));
        let georgian = clusters(text).into_iter().find(|c| c.script == Script::Georgian).unwrap();
        let index = set.index_for(&font, georgian.text, georgian.script);
        assert!(set.fonts[index as usize - 1].postscript_name().contains("Georgian"));
    }

    /// Fonts of the clusters of `text` (as in `GlyphInfo::font`) when it is shaped as a whole.
    fn glyph_fonts(text: &str, families: Vec<FontFamily>) -> (Vec<(usize, u16)>, TextFonts) {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(families, "");
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
        let glyphs = crate::layout::text::shape_directed(text, &font, &fonts, Some(0), false);
        (glyphs.iter().map(|(_, g)| (g.cluster as usize, g.font)).collect(), fonts)
    }

    #[test]
    fn a_script_run_is_shaped_with_one_font() {
        // Termes has no Arabic, but has the digit: the digit goes with the Arabic font
        let text = "ب1ب";
        let (glyphs, fonts) = glyph_fonts(text, vec![named("TeX Gyre Termes"), FontFamily::Serif]);
        assert!(glyphs.len() >= 3);
        assert_ne!(glyphs[0].1, 0);
        assert!(glyphs.iter().all(|(_, font)| *font == glyphs[0].1), "{glyphs:?}");
        assert_eq!(fonts.runs().len(), 1);
        assert_eq!((fonts.runs()[0].start, fonts.runs()[0].end), (0, text.len()));
    }

    #[test]
    fn scripts_get_their_own_fonts() {
        let text = "abc بسم 漢字";
        let (glyphs, _) = glyph_fonts(text, vec![named("TeX Gyre Termes"), FontFamily::Serif]);
        let font_of = |needle: &str| {
            let at = text.find(needle).unwrap();
            glyphs.iter().find(|(cluster, _)| *cluster == at).unwrap().1
        };
        assert_eq!(font_of("abc"), 0);
        assert_ne!(font_of("بسم"), 0);
        assert_ne!(font_of("漢字"), 0);
        assert_ne!(font_of("بسم"), font_of("漢字"));
        // the space between the Latin and the Arabic word belongs to the Latin run
        assert_eq!(font_of(" بسم"), 0);
    }

    /// Pieces of a line are shaped on their own but use the fonts of the whole text.
    #[test]
    fn pieces_are_shaped_with_the_fonts_of_the_whole_text() {
        let text = "ب1ب ب2ب";
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "");
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
        let shape = crate::layout::text::shape_directed;
        let whole: Vec<_> = shape(text, &font, &fonts, Some(0), false).iter().map(|(_, g)| g.font).collect();
        // the second word on its own: the digit is Arabic text there as well
        let split = text.find(' ').unwrap() + 1;
        let piece: Vec<_> = shape(&text[split..], &font, &fonts, Some(split), false)
            .iter()
            .map(|(_, g)| g.font)
            .collect();
        assert!(piece.iter().all(|f| *f == piece[0] && *f != 0));
        assert_eq!(piece[0], whole[0]);
    }

    #[test]
    fn a_cluster_stays_in_one_font() {
        // bet with a sheva: Termes has neither, the Hebrew font has both
        let text = "\u{05D1}\u{05B0}";
        let (glyphs, _) = glyph_fonts(text, vec![named("TeX Gyre Termes"), FontFamily::Serif]);
        assert!(glyphs.len() >= 2);
        assert!(glyphs.iter().all(|(_, font)| *font == glyphs[0].1 && *font != 0), "{glyphs:?}");
    }

    #[test]
    fn text_fonts_are_looked_up_by_offset() {
        let fonts = TextFonts {
            key: FallbackKey::default(),
            runs: Arc::from([
                FontRun { start: 0, end: 3, font: 0 },
                FontRun { start: 3, end: 8, font: 2 },
                FontRun { start: 10, end: 12, font: 1 },
            ]),
            letter_spacing: 0.0,
        };
        assert_eq!(fonts.font_at(0), Some(0));
        assert_eq!(fonts.font_at(2), Some(0));
        assert_eq!(fonts.font_at(3), Some(2));
        assert_eq!(fonts.font_at(7), Some(2));
        assert_eq!(fonts.font_at(8), None);
        assert_eq!(fonts.font_at(11), Some(1));
        assert_eq!(fonts.font_at(12), None);
        assert_eq!(TextFonts::default().font_at(0), None);
    }

    /// Han text of an unknown language is shaped without an explicit language: harfbuzz fills in
    /// what is unset, and the script comes from the run.
    #[test]
    fn text_without_a_language_is_shaped() {
        let (glyphs, _) = glyph_fonts("漢字 ب", vec![FontFamily::SansSerif]);
        assert!(glyphs.len() >= 4);
            }

    fn bold(mut style: ComputedValues) -> ComputedValues {
        std::sync::Arc::make_mut(&mut style.font).font_weight = FontWeight::BOLD;
        style
    }

    fn fallback_names(font: &Font, key: &FallbackKey) -> Vec<String> {
        font.fallbacks(key).iter().map(|f| f.postscript_name()).collect()
    }

    // -- characters that no font has

    /// U+10000 (Linear B) is in none of the bundled fonts (system fonts are off in unit tests).
    #[test]
    fn a_character_in_no_font_is_reported_and_drawn_with_the_primary_font() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "");
        let font = primary(&mut font_map, &style);
        let text = "a \u{10000} b";
        let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);

        let missing = resolver.take_missing_glyphs();
        assert_eq!(missing.0.len(), 1);
        let entry = &missing.0[&'\u{10000}'];
        assert_eq!(entry.count, 1);
        assert_eq!(entry.fonts, vec!["TeXGyreTermes-Regular".to_owned()]);
        assert_eq!(entry.font_families, vec!["\"TeX Gyre Termes\", serif".to_owned()]);
        assert!(entry.context.contains("a \u{10000} b"));
        let report = missing.report().unwrap();
        assert!(report.contains("U+10000"), "{report}");
        assert!(report.contains("1 character cannot be rendered"), "{report}");

        // no fallback font was added and the whole text is shaped with the primary font
        assert!(font.fallbacks(&fonts.key).is_empty());
        assert!(fonts.runs().is_empty());
        let glyphs = crate::layout::text::shape_directed(text, &font, &fonts, Some(0), false);
        assert!(glyphs.iter().all(|(_, g)| g.font == 0));
        // taking the missing glyphs resets them
        assert!(resolver.take_missing_glyphs().is_empty());
    }

    #[test]
    fn missing_characters_are_counted_for_every_occurrence() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        resolver.ensure_coverage(&style, &font, "\u{10000}\u{10001}\u{10000}", &mut font_map);
        // the cached answer for the second text is a miss as well
        resolver.ensure_coverage(&style, &font, "x\u{10000}", &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert_eq!(missing.0[&'\u{10000}'].count, 3);
        assert_eq!(missing.0[&'\u{10001}'].count, 1);
        let report = missing.report().unwrap();
        assert!(report.contains("2 characters cannot be rendered"), "{report}");
        assert!(report.contains("(3 occurrences)"), "{report}");
        assert!(report.contains("(1 occurrence)"), "{report}");
        assert!(report.contains("them"), "{report}");
    }

    #[test]
    fn text_with_and_without_fonts_is_resolved_together() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "ja");
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, "漢 \u{10000} 字", &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert_eq!(missing.0.keys().collect::<Vec<_>>(), vec![&'\u{10000}']);
        assert!(fallback_names(&font, &fonts.key).contains(&"NotoSansCJKjp-Regular".to_owned()));
    }

    #[test]
    fn controls_and_invisible_formatting_need_no_font() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, "a\n\u{200D}\u{00AD}\u{FE0F}b\t", &mut font_map);
        assert!(resolver.take_missing_glyphs().is_empty());
        assert!(font.fallbacks(&fonts.key).is_empty());
    }

    // -- clusters with marks

    /// A mark that no font has leaves the base character in the font that has it.
    #[test]
    fn a_mark_in_no_font_is_reported_on_its_own() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        assert!(font.covers('e') && !font.covers('\u{0350}'));
        let fonts = resolver.ensure_coverage(&style, &font, "e\u{0350}", &mut font_map);
        let missing = resolver.take_missing_glyphs();
        assert_eq!(missing.0.keys().collect::<Vec<_>>(), vec![&'\u{0350}']);
        assert!(font.fallbacks(&fonts.key).is_empty());
    }

    /// A font that covers the whole cluster wins over one that has only its base character.
    #[test]
    fn a_font_covering_the_whole_cluster_is_preferred() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        let dejavu = Font::bundled(&FontWeight::NORMAL);
        let mark = ('\u{0300}'..='\u{036F}')
            .find(|mark| !font.covers(*mark) && dejavu.covers(*mark))
            .expect("DejaVu Sans has a combining mark that Termes lacks");
        let cluster = format!("e{mark}");
        let fonts = resolver.ensure_coverage(&style, &font, &cluster, &mut font_map);
        assert!(resolver.take_missing_glyphs().is_empty());
        let names = fallback_names(&font, &fonts.key);
        assert_eq!(names.len(), 1);
        assert!(font.fallbacks(&fonts.key)[0].covers_cluster(&cluster));
        // the cluster is one run in that font
        assert_eq!(fonts.runs(), &[FontRun { start: 0, end: cluster.len(), font: 1 }]);
    }

    // -- the font description decides

    #[test]
    fn every_font_description_has_its_own_fallback_fonts() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let plain = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "");
        let japanese = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], "ja");
        let font = primary(&mut font_map, &plain);
        let (plain_key, japanese_key) = (FallbackKey::new(&plain), FallbackKey::new(&japanese));
        resolver.ensure_coverage(&plain, &font, "漢", &mut font_map);
        assert!(!font.fallbacks(&plain_key).is_empty());
        assert!(font.fallbacks(&japanese_key).is_empty());
        resolver.ensure_coverage(&japanese, &font, "漢", &mut font_map);
        assert!(!font.fallbacks(&japanese_key).is_empty());
    }

    #[test]
    fn keys_differ_by_families_weight_style_and_language() {
        let base = style_with(vec![named("A"), FontFamily::Serif], "en");
        let key = FallbackKey::new(&base);
        assert_eq!(key, FallbackKey::new(&style_with(vec![named("A"), FontFamily::Serif], "en")));
        assert_ne!(key, FallbackKey::new(&style_with(vec![named("B"), FontFamily::Serif], "en")));
        assert_ne!(key, FallbackKey::new(&style_with(vec![FontFamily::Serif, named("A")], "en")));
        assert_ne!(key, FallbackKey::new(&style_with(vec![named("A"), FontFamily::Serif], "de")));
        assert_ne!(key, FallbackKey::new(&bold(base.clone())));
        let mut italic = base.clone();
        std::sync::Arc::make_mut(&mut italic.font).font_style = FontStyle::Italic;
        assert_ne!(key, FallbackKey::new(&italic));
        // the order is consistent with the equality
        assert_eq!(key.cmp(&key.clone()), std::cmp::Ordering::Equal);
        let other = FallbackKey::new(&bold(base));
        assert_eq!(key.cmp(&other), other.cmp(&key).reverse());
        assert_eq!(key.partial_cmp(&other), Some(key.cmp(&other)));
        let hash = |key: &FallbackKey| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&key), hash(&key.clone()));
    }

    #[test]
    fn weight_selects_the_fallback_font_weight() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = bold(style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], ""));
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, "א", &mut font_map);
        assert_eq!(fallback_names(&font, &fonts.key), vec!["NotoSerifHebrew-Bold".to_owned()]);
    }

    // -- generic families resolve for the script at their position in the list

    fn hebrew_font(families: Vec<FontFamily>) -> Vec<String> {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(families, "");
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, "א", &mut font_map);
        fallback_names(&font, &fonts.key)
    }

    #[test]
    fn generic_families_give_the_design_for_the_script() {
        let termes = || named("TeX Gyre Termes");
        assert_eq!(hebrew_font(vec![termes(), FontFamily::Serif]), ["NotoSerifHebrew-Regular"]);
        assert_eq!(hebrew_font(vec![termes(), FontFamily::SansSerif]), ["NotoSansHebrew-Regular"]);
        // the first generic family decides, wherever it is
        assert_eq!(
            hebrew_font(vec![termes(), FontFamily::SansSerif, FontFamily::Serif]),
            ["NotoSansHebrew-Regular"]
        );
        // without one, the primary font decides: Termes is serif, Heros sans-serif
        assert_eq!(hebrew_font(vec![termes()]), ["NotoSerifHebrew-Regular"]);
        assert_eq!(hebrew_font(vec![named("TeX Gyre Heros")]), ["NotoSansHebrew-Regular"]);
        // monospace has no Hebrew fonts of its own
        assert_eq!(hebrew_font(vec![named("TeX Gyre Cursor")]), ["NotoSansHebrew-Regular"]);
    }

    #[test]
    fn a_family_of_the_list_wins_over_the_design_of_the_text() {
        assert_eq!(
            hebrew_font(vec![named("TeX Gyre Termes"), named("Noto Sans Hebrew"), FontFamily::Serif]),
            ["NotoSansHebrew-Regular"]
        );
    }

    #[test]
    fn the_class_is_cached_per_description() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let font = primary(&mut font_map, &style);
        let key = FallbackKey::new(&style);
        assert_eq!(resolver.class(&key, &font), Class::Serif);
        // a second answer comes from the cache, whatever primary font is asked about
        assert_eq!(resolver.class(&key, &Font::bundled(&FontWeight::NORMAL)), Class::Serif);
        // a font without any classification is sans-serif
        let unknown = style_with(vec![named("TeX Gyre Heros")], "");
        let heros = primary(&mut font_map, &unknown);
        assert_eq!(resolver.class(&FallbackKey::new(&unknown), &heros), Class::Sans);
        assert_eq!(Class::Mono.generic(), FontFamily::Monospace);
        assert_eq!(Class::Serif.generic(), FontFamily::Serif);
        assert_eq!(Class::Sans.generic(), FontFamily::SansSerif);
    }

    // -- the fonts of a script run

    fn cluster(start: usize, text: &str, script: Script) -> crate::layout::text::segment::Cluster<'_> {
        crate::layout::text::segment::Cluster { start, text, script }
    }

    #[test]
    fn the_font_covering_most_of_a_run_is_used_for_all_of_it() {
        let mut font_map = FontMap::new();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let key = FallbackKey::new(&style);
        let font = primary(&mut font_map, &style);
        let mut load = |name: &str| {
            register_font(
                &mut font_map,
                system_font(&[named(name)], &FontWeight::NORMAL, &FontStyle::Normal).unwrap(),
            )
        };
        let (hebrew, arabic) = (load("Noto Sans Hebrew"), load("Noto Naskh Arabic"));
        // "אבב" and "ب" are put in one run, as the chosen fonts differ per cluster
        font.add_fallback(&key, "א", Script::Hebrew, hebrew.clone());
        font.add_fallback(&key, "ב", Script::Hebrew, hebrew.clone());
        font.add_fallback(&key, "ب", Script::Hebrew, arabic.clone());
        let set = font.fallback_set(&key);
        let hebrew_index = set.index_for(&font, "א", Script::Hebrew);
        let arabic_index = set.index_for(&font, "ب", Script::Hebrew);
        assert_ne!(hebrew_index, arabic_index);

        let clusters = [
            cluster(0, "א", Script::Hebrew),
            cluster(2, "ב", Script::Hebrew),
            cluster(4, "ب", Script::Hebrew),
        ];
        // Hebrew covers two of the three clusters; the third keeps its own font
        assert_eq!(
            script_run_fonts(&font, &set, &clusters),
            vec![hebrew_index, hebrew_index, arabic_index]
        );
        // on a tie the font of the first cluster wins
        assert_eq!(
            script_run_fonts(&font, &set, &clusters[1..]),
            vec![hebrew_index, arabic_index]
        );
        // the primary font wins if it covers everything
        let latin = [cluster(0, "a", Script::Latin), cluster(1, "b", Script::Latin)];
        assert_eq!(script_run_fonts(&font, &set, &latin), vec![0, 0]);
        assert!(script_run_fonts(&font, &set, &[]).is_empty());
    }

    #[test]
    fn adding_a_fallback_twice_changes_nothing() {
        let mut font_map = FontMap::new();
        let style = style_with(vec![named("TeX Gyre Termes")], "");
        let key = FallbackKey::new(&style);
        let font = primary(&mut font_map, &style);
        let hebrew = register_font(
            &mut font_map,
            system_font(&[named("Noto Sans Hebrew")], &FontWeight::NORMAL, &FontStyle::Normal).unwrap(),
        );
        assert!(font.add_fallback(&key, "א", Script::Hebrew, hebrew.clone()));
        assert!(!font.add_fallback(&key, "א", Script::Hebrew, hebrew.clone()));
        // the same cluster in another script is a new choice, in the same font
        assert!(font.add_fallback(&key, "א", Script::Common, hebrew.clone()));
        assert_eq!(font.fallbacks(&key).len(), 1);
        // the font itself is never its own fallback
        assert!(!font.add_fallback(&key, "x", Script::Latin, font_map["TeXGyreTermes-Regular"].clone()));
        // a choice for a script that was not itemized is found by the cluster alone
        let set = font.fallback_set(&key);
        assert_eq!(set.index_for(&font, "א", Script::Arabic), 1);
        // the primary font covers its own clusters, and unknown ones are not changed
        assert_eq!(set.index_for(&font, "a", Script::Latin), 0);
        assert_eq!(set.index_for(&font, "ب", Script::Arabic), 0);
    }

    // -- the tables

    fn variant_names(script: Script, lang: &str) -> Vec<&'static str> {
        cjk_variants(script, &Language::from_tag(lang)).iter().map(|v| v.sans[0]).collect()
    }

    #[test]
    fn cjk_variants_are_ordered_by_script_and_language() {
        assert_eq!(variant_names(Script::Han, "ja")[0], "Noto Sans CJK JP");
        assert_eq!(variant_names(Script::Han, "ko")[0], "Noto Sans CJK KR");
        assert_eq!(variant_names(Script::Han, "zh")[..2], ["Noto Sans CJK SC", "Noto Sans CJK TC"]);
        assert_eq!(
            variant_names(Script::Han, "zh-HK")[..3],
            ["Noto Sans CJK HK", "Noto Sans CJK TC", "Noto Sans CJK SC"]
        );
        assert_eq!(variant_names(Script::Han, "yue")[0], "Noto Sans CJK HK");
        assert_eq!(variant_names(Script::Han, "zh-Hant")[0], "Noto Sans CJK TC");
        assert_eq!(variant_names(Script::Han, "zh-MO")[0], "Noto Sans CJK HK");
        assert_eq!(variant_names(Script::Han, "cmn")[0], "Noto Sans CJK SC");
        assert_eq!(variant_names(Script::Katakana, "ko")[0], "Noto Sans CJK JP");
        assert_eq!(variant_names(Script::Hangul, "zh")[0], "Noto Sans CJK KR");
        assert_eq!(variant_names(Script::Bopomofo, "zh-TW")[0], "Noto Sans CJK TC");
        // a variant is listed once, and every script gets all the common ones
        for (script, lang) in [(Script::Han, "ja"), (Script::Han, "zh-HK"), (Script::Hangul, "ko")] {
            let names = variant_names(script, lang);
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), names.len(), "{names:?}");
            assert!(names.contains(&"Noto Sans CJK JP") && names.contains(&"Noto Sans CJK SC"));
        }
    }

    #[test]
    fn language_tables_for_arabic_and_other_designs() {
        let fa = Language::from_tag("fa");
        assert_eq!(script_families(Script::Arabic, Class::Serif, &fa)[0], "Noto Naskh Arabic");
        assert_eq!(script_families(Script::Arabic, Class::Sans, &fa)[0], "Vazirmatn");
        assert_eq!(script_families(Script::Arabic, Class::Mono, &fa)[0], "Vazirmatn");
        let ur = Language::from_tag("ur");
        assert_eq!(script_families(Script::Arabic, Class::Sans, &ur)[0], "Noto Nastaliq Urdu");
        // the general fonts follow the language's
        assert!(script_families(Script::Arabic, Class::Serif, &ur).contains(&"Noto Naskh Arabic"));
        // other scripts: monospace uses the sans-serif fonts
        let none = Language::default();
        assert_eq!(
            script_families(Script::Thai, Class::Mono, &none),
            script_families(Script::Thai, Class::Sans, &none)
        );
        assert!(script_families(Script::Latin, Class::Mono, &none).is_empty());
        assert!(script_families(Script::Cyrillic, Class::Serif, &none).is_empty());
        // every script of the table has fonts for both designs
        for (script, serif, sans) in SCRIPT_FAMILIES {
            assert!(!serif.is_empty() && !sans.is_empty(), "{script:?}");
        }
    }

    #[test]
    fn small_helpers() {
        assert_eq!(describe_char('→'), "U+2192 '→'");
        assert_eq!(describe_char('\n'), "U+000A");
        assert_eq!(describe_char(' '), "U+0020");
        assert_eq!(
            format_font_family(&[named("A B"), FontFamily::SansSerif, FontFamily::Serif, FontFamily::Monospace]),
            "\"A B\", sans-serif, serif, monospace"
        );
        assert_eq!(context_around("short", 2), "short");
        assert_eq!(
            context_around("0123456789 0123456789 0123456789 X", 33),
            "…23456789 0123456789 X"
        );
        assert_eq!(context_around("a\nb\tc", 2), "a b c");
        assert!(MissingGlyphs::default().is_empty());
        assert!(MissingGlyphs::default().report().is_none());
        assert_eq!(Class::of_generic(&named("x")), None);
    }

    // -- every bundled font is a fallback somewhere

    fn bundled_faces() -> Vec<Font> {
        super::super::font::bundled_font_data()
            .into_iter()
            .filter_map(|data| Font::load(Arc::new(data), 0))
            .collect()
    }

    /// The first assigned character that only the bundled faces whose PostScript name starts
    /// with `prefix` cover. The faces of the prefix are checked first, which rejects nearly all
    /// characters without looking at the other faces.
    fn first_only_covered_by(faces: &[Font], prefix: &str) -> Option<char> {
        use unicode_script::UnicodeScript;
        let (targets, others): (Vec<&Font>, Vec<&Font>) =
            faces.iter().partition(|face| face.postscript_name().starts_with(prefix));
        (0x20u32..0x2FFFF)
            .filter_map(char::from_u32)
            .filter(|c| !c.is_control() && targets.iter().any(|face| face.covers(*c)))
            .find(|c| c.script() != Script::Unknown && !others.iter().any(|face| face.covers(*c)))
    }

    /// The names of the fallback fonts that `text` in `families` (with the given weight, style
    /// and language) gets, with only the bundled fonts installed.
    fn selected(families: Vec<FontFamily>, lang: &str, bold: bool, italic: bool, text: &str) -> Vec<String> {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let mut style = style_with(families, lang);
        if bold {
            style = self::bold(style);
        }
        if italic {
            std::sync::Arc::make_mut(&mut style.font).font_style = FontStyle::Italic;
        }
        let font = primary(&mut font_map, &style);
        let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
        fallback_names(&font, &fonts.key)
    }

    /// A character that only DejaVu Sans has (none of the other bundled fonts) is found in the
    /// DejaVu Sans of the weight of the text, whatever the font family.
    #[test]
    fn a_character_only_dejavu_has_gets_dejavu() {
        let faces = bundled_faces();
        let character =
            first_only_covered_by(&faces, "DejaVuSans").expect("DejaVu Sans has characters no other font has");
        let text = character.to_string();
        for families in [
            vec![named("TeX Gyre Termes"), FontFamily::Serif],
            vec![named("TeX Gyre Heros"), FontFamily::SansSerif],
            vec![named("TeX Gyre Cursor"), FontFamily::Monospace],
        ] {
            assert_eq!(selected(families.clone(), "", false, false, &text), ["DejaVuSans"], "{character:?}");
            assert_eq!(selected(families, "", true, false, &text), ["DejaVuSans-Bold"], "{character:?}");
        }
    }

    /// DejaVu Sans is a family of the broad fallback list, found by name before the font that
    /// is built into Helium (which is the same font), so that is never needed.
    #[test]
    fn the_built_in_font_is_found_by_its_family_name() {
        let mut font_map = FontMap::new();
        let mut resolver = FontResolver::default();
        let family = resolver
            .family_font(named("DejaVu Sans"), &FontWeight::NORMAL, &FontStyle::Normal, &mut font_map)
            .unwrap();
        assert_eq!(family.postscript_name(), Font::bundled(&FontWeight::NORMAL).postscript_name());
    }

    /// Every bundled face must be reachable as a fallback font: text in the right script,
    /// weight and style gets it. Adding a font to the bundle without a way to select it fails
    /// here (the embedded fonts would never be used).
    #[test]
    fn every_bundled_font_is_selected_as_a_fallback_in_some_circumstance() {
        use std::collections::BTreeSet;
        let faces = bundled_faces();
        let mut chosen: BTreeSet<String> = BTreeSet::new();

        // a letter of every script with an embedded font, in every design, weight and for
        // the languages with fonts of their own
        let letters = "بאաকक\u{12A0}აકਕಕកກകက\u{0B15}සதతไ\u{0710}\u{0780}漢";
        let designs = [
            vec![named("TeX Gyre Termes"), FontFamily::Serif],
            vec![named("TeX Gyre Heros"), FontFamily::SansSerif],
        ];
        for families in &designs {
            for lang in ["", "ur", "ja"] {
                for bold in [false, true] {
                    chosen.extend(selected(families.clone(), lang, bold, false, letters));
                }
            }
        }

        // The TeX Gyre families have the same characters, so they are fallbacks for text that
        // the primary font has no Latin letters for, through the generic family of the target
        // at the end of the list (all weights and styles)
        let hebrew = system_font(&[named("Noto Sans Hebrew")], &FontWeight::NORMAL, &FontStyle::Normal).unwrap();
        assert!(!hebrew.covers('a'));
        for generic in [FontFamily::SansSerif, FontFamily::Serif, FontFamily::Monospace] {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                chosen.extend(selected(
                    vec![named("Noto Sans Hebrew"), generic.clone()],
                    "",
                    bold,
                    italic,
                    "a",
                ));
            }
        }

        // the math fonts are named in the font family; the other fonts for symbols come from
        // the broad fallback list
        for math in ["Latin Modern Math", "STIX Two Math"] {
            let face = system_font(&[named(math)], &FontWeight::NORMAL, &FontStyle::Normal).unwrap();
            let termes = system_font(&[named("TeX Gyre Termes")], &FontWeight::NORMAL, &FontStyle::Normal).unwrap();
            let ch = ('\u{2200}'..='\u{22FF}')
                .find(|c| face.covers(*c) && !termes.covers(*c))
                .unwrap();
            chosen.extend(selected(vec![named("TeX Gyre Termes"), named(math)], "", false, false, &ch.to_string()));
        }
        for prefix in ["NotoSansMath-", "NotoSansSymbols-", "NotoSansSymbols2-"] {
            let ch = first_only_covered_by(&faces, prefix)
                .unwrap_or_else(|| panic!("{prefix} has characters no other bundled font has"));
            chosen.extend(selected(vec![named("TeX Gyre Termes"), FontFamily::Serif], "", false, false, &ch.to_string()));
        }

        // DejaVu Sans, in both weights
        let text = first_only_covered_by(&faces, "DejaVuSans").unwrap().to_string();
        for bold in [false, true] {
            chosen.extend(selected(vec![named("TeX Gyre Termes")], "", bold, false, &text));
        }

        let missing: Vec<String> = faces
            .iter()
            .map(|face| face.postscript_name())
            .filter(|name| !chosen.contains(name))
            .collect();
        assert!(missing.is_empty(), "bundled fonts that are never selected as a fallback: {missing:?}");
    }


    // -- Chinese, Japanese and Korean text

    fn cjk_candidates(lang: &str, script: Script, class: Class, families: Vec<FontFamily>) -> Vec<String> {
        let key = FallbackKey::new(&style_with(families, lang));
        candidate_families(&key, class, script)
            .into_iter()
            .filter_map(|family| match family {
                FontFamily::Named(name) => Some(name),
                _ => None,
            })
            .collect()
    }

    /// The first CJK family of the candidates.
    fn first_cjk(candidates: &[String]) -> &str {
        candidates.iter().find(|name| name.contains("CJK")).map(String::as_str).unwrap()
    }

    /// The font that is used if only the `installed` families exist.
    fn first_installed<'a>(candidates: &'a [String], installed: &[&str]) -> Option<&'a str> {
        candidates.iter().map(String::as_str).find(|name| installed.contains(name))
    }

    #[test]
    fn han_characters_get_the_variant_of_the_language_in_every_design() {
        let designs = [
            (Class::Serif, "Serif"),
            (Class::Sans, "Sans"),
            (Class::Mono, "Sans Mono"),
        ];
        let cases: &[(&[&str], &str)] = &[
            (&["ja", "ja-JP"], "JP"),
            (&["ko", "ko-KR", "ko-Kore"], "KR"),
            // simplified Chinese, whatever the country and the language code
            (&["zh", "zh-CN", "zh-Hans", "zh-Hans-CN", "zh-SG", "cmn", "cmn-Hans", "wuu", "gan", "hsn", "hak"], "SC"),
            // the script decides over the region
            (&["zh-Hans-HK", "zh-Hans-TW", "zh-Hans-MO", "yue-Hans"], "SC"),
            (&["zh-TW", "zh-Hant", "zh-Hant-TW", "zh-hant-cn", "nan", "lzh", "zh_TW"], "TC"),
            (&["zh-HK", "zh-MO", "zh-Hant-HK", "zh-Hant-MO", "yue", "yue-HK", "zh_hk"], "HK"),
            // unknown languages and other languages get the Japanese forms, as fontconfig does
            (&["", "en", "de-CH", "fr", "und", "vi"], "JP"),
        ];
        for (langs, variant) in cases {
            for lang in *langs {
                for (class, design) in &designs {
                    let candidates = cjk_candidates(lang, Script::Han, *class, vec![]);
                    assert_eq!(
                        first_cjk(&candidates),
                        format!("Noto {design} CJK {variant}").replace("Sans Mono CJK", "Sans Mono CJK"),
                        "{lang:?} {class:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn kana_and_hangul_do_not_depend_on_the_language() {
        for lang in ["ja", "zh", "zh-TW", "ko", "en", ""] {
            for class in [Class::Serif, Class::Sans, Class::Mono] {
                for script in [Script::Hiragana, Script::Katakana] {
                    let candidates = cjk_candidates(lang, script, class, vec![]);
                    assert!(first_cjk(&candidates).ends_with("JP"), "{lang:?} {script:?}");
                }
                let candidates = cjk_candidates(lang, Script::Hangul, class, vec![]);
                assert!(first_cjk(&candidates).ends_with("KR"), "{lang:?}");
            }
        }
        // Bopomofo follows the Chinese of the language
        let tw = cjk_candidates("zh-TW", Script::Bopomofo, Class::Sans, vec![]);
        assert_eq!(first_cjk(&tw), "Noto Sans CJK TC");
        let cn = cjk_candidates("zh-CN", Script::Bopomofo, Class::Sans, vec![]);
        assert_eq!(first_cjk(&cn), "Noto Sans CJK SC");
    }

    /// CJK punctuation is itemized with the script of the text around it, so it gets the font
    /// of the language of the Han characters, kana or Hangul that it is set with.
    #[test]
    fn cjk_punctuation_gets_the_font_of_the_text_around_it() {
        use crate::layout::text::segment::clusters;
        let font_for = |lang: &str, text: &str, at: &str| {
            let found = clusters(text)
                .into_iter()
                .find(|c| c.text == at)
                .unwrap_or_else(|| panic!("no cluster {at:?} in {:?}", clusters(text).iter().map(|c| c.text).collect::<Vec<_>>()));
            let candidates = cjk_candidates(lang, found.script, Class::Sans, vec![]);
            first_cjk(&candidates).to_owned()
        };
        assert_eq!(font_for("zh-CN", "你好。", "。"), "Noto Sans CJK SC");
        assert_eq!(font_for("zh-TW", "你好。", "。"), "Noto Sans CJK TC");
        assert_eq!(font_for("ja", "你好。", "。"), "Noto Sans CJK JP");
        assert_eq!(font_for("ko", "漢字、", "、"), "Noto Sans CJK KR");
        // brackets and the ideographic space in a Han run
        assert_eq!(font_for("zh-CN", "你「好」", "「"), "Noto Sans CJK SC");
        assert_eq!(font_for("zh-CN", "你\u{3000}好", "\u{3000}"), "Noto Sans CJK SC");
        // the text decides, not the language: kana run in a Chinese document
        assert_eq!(font_for("zh-CN", "かな、", "、"), "Noto Sans CJK JP");
        assert_eq!(font_for("zh-CN", "한국、", "、"), "Noto Sans CJK KR");
        // after Latin letters the punctuation is a Han one of its own (unlike a Latin comma)
        assert_eq!(font_for("zh-CN", "abc。", "。"), "Noto Sans CJK SC");
        // fullwidth forms
        assert_eq!(font_for("ja", "A！", "！"), "Noto Sans CJK JP");
    }

    /// A font of the language wins over one of another language, even in another design: the
    /// forms of the shared Han characters would be wrong.
    #[test]
    fn a_font_of_the_language_beats_the_design_of_another_language() {
        let installed = ["Noto Serif CJK SC", "Noto Sans CJK JP"];
        let pick = |lang, class| {
            let candidates = cjk_candidates(lang, Script::Han, class, vec![]);
            first_installed(&candidates, &installed).map(str::to_owned)
        };
        // serif text in Japanese: the serif font is Chinese, so the Japanese sans-serif one
        assert_eq!(pick("ja", Class::Serif).as_deref(), Some("Noto Sans CJK JP"));
        // in Chinese the Chinese serif font is the one
        assert_eq!(pick("zh", Class::Serif).as_deref(), Some("Noto Serif CJK SC"));
        // sans-serif text never takes a serif font
        assert_eq!(pick("zh", Class::Sans).as_deref(), Some("Noto Sans CJK JP"));
        // monospace text: the proportional font of the language beats another language's
        // monospace font, but not its own
        let installed = ["Noto Sans CJK JP", "Noto Sans Mono CJK SC"];
        let pick = |lang| {
            let candidates = cjk_candidates(lang, Script::Han, Class::Mono, vec![]);
            first_installed(&candidates, &installed).map(str::to_owned)
        };
        assert_eq!(pick("ja").as_deref(), Some("Noto Sans CJK JP"));
        assert_eq!(pick("zh").as_deref(), Some("Noto Sans Mono CJK SC"));
        // the order of the first variants: serif, then sans, then the next variant's
        let names = cjk_families(Script::Han, Class::Serif, &Language::from_tag("ja"));
        let position = |name: &str| names.iter().position(|n| *n == name).unwrap();
        assert!(position("Noto Serif CJK JP") < position("Noto Sans CJK JP"));
        assert!(position("Noto Sans CJK JP") < position("Noto Serif CJK SC"));
        assert!(position("Noto Serif CJK SC") < position("Noto Sans CJK SC"));
    }

    #[test]
    fn the_font_family_list_comes_before_the_fonts_for_the_script() {
        let kr = named("Noto Sans CJK KR");
        // an explicit family wins over the font of the language
        let candidates = cjk_candidates("zh-CN", Script::Han, Class::Serif, vec![kr.clone(), FontFamily::Serif]);
        assert_eq!(candidates[0], "Noto Sans CJK KR");
        // `serif` at its position stands for the serif fonts of the language
        assert_eq!(candidates[1], "Noto Serif CJK SC");
        // and a generic family before an explicit one comes first
        let candidates = cjk_candidates("zh-CN", Script::Han, Class::Serif, vec![FontFamily::Serif, kr]);
        let position = |name: &str| candidates.iter().position(|n| n == name).unwrap();
        assert!(position("Noto Serif CJK SC") < position("Noto Sans CJK KR"));
        // each generic family resolves for the script at its own position
        let candidates = cjk_candidates("ja", Script::Han, Class::Sans, vec![FontFamily::Monospace]);
        assert_eq!(candidates[0], "Noto Sans Mono CJK JP");
        let candidates = cjk_candidates("ja", Script::Han, Class::Serif, vec![FontFamily::SansSerif, FontFamily::Serif]);
        assert_eq!(candidates[0], "Noto Sans CJK JP");
        // the design of the text follows the first generic family
        let key = FallbackKey::new(&style_with(vec![named("TeX Gyre Termes"), FontFamily::Monospace], "ja"));
        let mut font_map = FontMap::new();
        let font = primary(&mut font_map, &style_with(vec![named("TeX Gyre Termes")], ""));
        assert_eq!(FontResolver::default().class(&key, &font), Class::Mono);
    }

    /// With only the bundled fonts, all the Chinese, Japanese and Korean text is set in the one
    /// bundled CJK font, in every design, language and weight, and nothing is missing.
    #[test]
    fn cjk_text_is_set_in_the_bundled_cjk_font_in_every_circumstance() {
        let text = "漢字 かな カナ 한글 、。「」！ ㄅㄆ";
        for lang in ["", "ja", "ko", "zh", "zh-TW", "zh-HK", "zh-Hans-CN"] {
            for families in [
                vec![named("TeX Gyre Termes"), FontFamily::Serif],
                vec![named("TeX Gyre Heros"), FontFamily::SansSerif],
                vec![named("TeX Gyre Cursor"), FontFamily::Monospace],
            ] {
                for bold in [false, true] {
                    let mut font_map = FontMap::new();
                    let mut resolver = FontResolver::default();
                    let mut style = style_with(families.clone(), lang);
                    if bold {
                        style = self::bold(style);
                    }
                    let font = primary(&mut font_map, &style);
                    let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
                    let missing = resolver.take_missing_glyphs();
                    let names = fallback_names(&font, &fonts.key);
                    let expected = if bold { "NotoSansCJKjp-Bold" } else { "NotoSansCJKjp-Regular" };
                    assert_eq!(names, [expected], "{lang:?} {families:?} bold={bold}");
                    // Bopomofo is not in the subset font: it is the only thing reported
                    assert!(
                        missing.0.keys().all(|c| ('\u{3105}'..='\u{312F}').contains(c)),
                        "{lang:?}: {:?}",
                        missing.0.keys().collect::<Vec<_>>()
                    );
                    assert!(fonts.runs().iter().any(|run| run.font == 1));
                }
            }
        }
    }

    /// The bundled font has the regional forms of Han characters, selected by the language of
    /// the text, so the language reaches the shaper.
    #[test]
    fn the_language_selects_the_regional_forms_of_han_characters() {
        let glyph_ids = |lang: &str| -> Vec<u32> {
            let text = "直骨返海令刃";
            let (glyphs, _) = {
                let mut font_map = FontMap::new();
                let mut resolver = FontResolver::default();
                let style = style_with(vec![named("TeX Gyre Termes"), FontFamily::Serif], lang);
                let font = primary(&mut font_map, &style);
                let fonts = resolver.ensure_coverage(&style, &font, text, &mut font_map);
                let glyphs = crate::layout::text::shape_directed(text, &font, &fonts, Some(0), false);
                (glyphs, fonts)
            };
            glyphs.iter().map(|(_, g)| g.codepoint).collect()
        };
        let (ja, sc, tc, kr) = (glyph_ids("ja"), glyph_ids("zh-Hans"), glyph_ids("zh-Hant"), glyph_ids("ko"));
        assert_eq!(ja.len(), 6);
        assert_ne!(ja, sc, "Japanese and simplified Chinese forms differ");
        assert_ne!(sc, tc, "simplified and traditional Chinese forms differ");
        assert_ne!(ja, kr, "Japanese and Korean forms differ");
        // the same language gives the same glyphs, with or without a region
        assert_eq!(sc, glyph_ids("zh-CN"));
    }
}
