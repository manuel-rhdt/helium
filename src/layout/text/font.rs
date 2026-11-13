pub use super::HarfbuzzFont;
pub use lemon_pdf::font::FontUnit;

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, PixelLength,
};

use harfbuzz_rs::{Blob, Face as HbFace, Font as HbFont, HarfbuzzObject, sys};
use parking_lot::{Mutex, RwLock};
use super::fallback::FallbackKey;
use super::segment::needs_font;
use std::collections::{BTreeMap, HashMap};
use unicode_script::Script;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

fn scale_ext(upem: u32, metric: i32) -> FontUnit {
    let upem = upem as f64;
    FontUnit(metric as f64 * 1000f64 / upem)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct GlyphIndex(pub u32);

/// The fallback fonts of a font for one font description and which of them is used for which
/// cluster. Which font a cluster gets depends only on the description and the cluster, not on the
/// order the fonts were added in.
#[derive(Debug, Clone, Default)]
pub(crate) struct FallbackSet {
    /// The fallback fonts. A glyph with `GlyphInfo::font == n > 0` belongs to `fonts[n - 1]`.
    pub(crate) fonts: Vec<Arc<Font>>,
    /// For a cluster, the script it was itemized as and the 1-based index in `fonts`.
    choices: HashMap<String, Vec<(Script, u16)>>,
}

impl FallbackSet {
    /// The font with the index `index` (as in `GlyphInfo::font`): `primary` or a fallback font.
    pub(crate) fn font_at<'a>(&'a self, primary: &'a Font, index: u16) -> &'a Font {
        match index {
            0 => primary,
            n => &self.fonts[n as usize - 1],
        }
    }

    /// The index (as in `GlyphInfo::font`) of the font for `cluster` in `script`: `0` for
    /// `primary` if it covers the cluster or no fallback font was chosen.
    pub(crate) fn index_for(&self, primary: &Font, cluster: &str, script: Script) -> u16 {
        if primary.covers_cluster(cluster) {
            return 0;
        }
        match self.choices.get(cluster) {
            Some(choices) => choices
                .iter()
                .find(|(s, _)| *s == script)
                .or(choices.first())
                .map_or(0, |(_, index)| *index),
            None => 0,
        }
    }
}

/// A font file (a system font or one of the fonts built into Helium). Every font that is used
/// is embedded into the PDF.
#[derive(Debug)]
pub struct Font {
    pub(crate) upem: u32,
    pub(crate) glyph_advances: Mutex<BTreeMap<GlyphIndex, FontUnit>>,
    pub(crate) to_unicode: Mutex<BTreeMap<GlyphIndex, String>>,
    /// Text for the `ToUnicode` map of glyphs that `to_unicode` has nothing for: the text of a
    /// glyph cluster that a glyph is drawn in (which `/ActualText` gives exactly). PDF/UA wants
    /// every glyph mapped to Unicode (ISO 14289-1 7.21.7).
    pub(crate) to_unicode_fallback: Mutex<BTreeMap<GlyphIndex, String>>,
    pub(crate) hb_font: HarfbuzzFont,
    pub(crate) postscript_name: String,
    /// The bytes of the font file and the index of the face in it.
    pub(crate) data: Arc<Vec<u8>>,
    pub(crate) index: u32,
    /// Fonts that provide the clusters this font has no glyphs for, for text of each font
    /// description (`FallbackKey`: the `font-family` list, weight, style and language), which
    /// decides the fallback fonts.
    ///
    /// They are filled in during box construction (see `fallback::FontResolver`), when the text
    /// that is rendered with this font is known, and used by `shape`. Every font in here is also
    /// an entry of the `FontMap`, so that it is embedded into the PDF.
    pub(crate) fallbacks: RwLock<BTreeMap<FallbackKey, Arc<FallbackSet>>>,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Font {
            upem: self.upem,
            glyph_advances: Mutex::new(self.glyph_advances.lock().clone()),
            to_unicode: Mutex::new(self.to_unicode.lock().clone()),
            to_unicode_fallback: Mutex::new(self.to_unicode_fallback.lock().clone()),
            data: self.data.clone(),
            index: self.index,
            hb_font: self.hb_font.clone(),
            postscript_name: self.postscript_name.clone(),
            fallbacks: RwLock::new(self.fallbacks.read().clone()),
        }
    }
}

/// The fonts built into Helium. They are used when no installed font matches the style, so
/// that there always is a font that can be embedded.
const BUNDLED_REGULAR: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans.ttf");
const BUNDLED_BOLD: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans-Bold.ttf");

// `EMBEDDED_FONTS`: the Noto fonts for the scripts that DejaVu Sans lacks, subset by `build.rs`.
include!(concat!(env!("OUT_DIR"), "/fonts/bundled_fonts.rs"));

/// Free, metric-compatible stand-ins for the Helvetica, Times and Courier families of the PDF
/// standard fonts (TeX Gyre Heros, Termes and Cursor), always available through the font
/// database so that the generic families and the standard font names resolve without any
/// installed fonts.
macro_rules! tex_gyre {
    ($($file:literal),* $(,)?) => {
        &[$(include_bytes!(concat!("../../../resources/fonts/", $file))),*]
    };
}
const TEX_GYRE: &[&[u8]] = tex_gyre![
    "texgyreheros-regular.otf",
    "texgyreheros-bold.otf",
    "texgyreheros-italic.otf",
    "texgyreheros-bolditalic.otf",
    "texgyretermes-regular.otf",
    "texgyretermes-bold.otf",
    "texgyretermes-italic.otf",
    "texgyretermes-bolditalic.otf",
    "texgyrecursor-regular.otf",
    "texgyrecursor-bold.otf",
    "texgyrecursor-italic.otf",
    "texgyrecursor-bolditalic.otf",
];

/// The OpenType math fonts (Latin Modern Math and STIX Two Math), used for MathML when
/// `font-family` does not name a font with a `MATH` table.
const MATH_FONTS: &[&[u8]] = &[
    include_bytes!("../../../resources/fonts/latinmodern-math.otf"),
    include_bytes!("../../../resources/fonts/STIXTwoMath-Regular.ttf"),
];

/// Families of `MATH_FONTS` in order of preference.
const MATH_FAMILIES: &[&str] = &["Latin Modern Math", "STIX Two Math"];

/// Names of the PDF standard fonts and their common look-alikes, with the bundled family that
/// stands in for them when they are not installed.
const BUNDLED_ALIASES: &[(&str, &str)] = &[
    ("Helvetica", "TeX Gyre Heros"),
    ("Arial", "TeX Gyre Heros"),
    ("Times", "TeX Gyre Termes"),
    ("Times New Roman", "TeX Gyre Termes"),
    ("Courier", "TeX Gyre Cursor"),
    ("Courier New", "TeX Gyre Cursor"),
];

impl Font {
    /// Loads face `index` of a font file, or returns `None` if it cannot be parsed.
    pub(crate) fn load(data: Arc<Vec<u8>>, index: u32) -> Option<Self> {
        let blob = Blob::with_bytes_owned(data.clone(), |b| &**b);
        let hb_face = HbFace::new(blob, index);
        if hb_face.glyph_count() == 0 || hb_face.table_with_tag(b"head").is_none() {
            return None;
        }
        let upem = hb_face.upem();
        let name = name_of(&hb_face, sys::HB_OT_NAME_ID_POSTSCRIPT_NAME)
            .or_else(|| name_of(&hb_face, sys::HB_OT_NAME_ID_FULL_NAME))
            .or_else(|| name_of(&hb_face, sys::HB_OT_NAME_ID_FONT_FAMILY))?;
        let hb_font = HbFont::new(hb_face).into();
        Some(Font {
            upem,
            glyph_advances: Default::default(),
            to_unicode: Default::default(),
            to_unicode_fallback: Default::default(),
            hb_font,
            postscript_name: name,
            data,
            index,
            fallbacks: Default::default(),
        })
    }

    /// A metric of the OpenType `OS/2` or `MVAR` tables in font units, if the font has it.
    pub(crate) fn ot_metric(&self, tag: sys::hb_ot_metrics_tag_t) -> Option<i32> {
        let mut position = 0;
        // SAFETY: the font is alive and `position` is a valid out pointer.
        let found =
            unsafe { sys::hb_ot_metrics_get_position(self.hb_font.as_raw(), tag, &mut position) };
        (found != 0).then_some(position)
    }

    /// The font built into Helium (DejaVu Sans) in the given weight.
    pub fn bundled(weight: &FontWeight) -> Self {
        static REGULAR: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
        static BOLD: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
        // the closer of the two weights, a tie goes to the heavier one like in font matching
        let bytes = if weight.0 >= 550.0 {
            BOLD.get_or_init(|| Arc::new(BUNDLED_BOLD.to_vec()))
        } else {
            REGULAR.get_or_init(|| Arc::new(BUNDLED_REGULAR.to_vec()))
        };
        Font::load(bytes.clone(), 0).expect("the bundled fonts are valid")
    }

    /// Whether this font has a glyph for `ch`.
    pub fn covers(&self, ch: char) -> bool {
        self.hb_font.get_nominal_glyph(ch).is_some()
    }

    /// Whether this font has glyphs for all characters of the grapheme cluster that need one.
    pub fn covers_cluster(&self, cluster: &str) -> bool {
        cluster.chars().all(|ch| !needs_font(ch) || self.covers(ch))
    }

    /// A snapshot of the fallback fonts of this font for text of the font description `key`.
    pub fn fallbacks(&self, key: &FallbackKey) -> Vec<Arc<Font>> {
        self.fallback_set(key).fonts.clone()
    }

    /// A snapshot of the fallback fonts of this font for `key` and the clusters that they were
    /// chosen for.
    pub(crate) fn fallback_set(&self, key: &FallbackKey) -> Arc<FallbackSet> {
        self.fallbacks.read().get(key).cloned().unwrap_or_default()
    }

    /// Uses `fallback` for `cluster` (itemized as `script`) in text of the description `key`.
    /// Returns whether the cluster is new.
    pub(crate) fn add_fallback(
        &self,
        key: &FallbackKey,
        cluster: &str,
        script: Script,
        fallback: Arc<Font>,
    ) -> bool {
        if fallback.postscript_name == self.postscript_name {
            return false;
        }
        let mut fallbacks = self.fallbacks.write();
        let set = Arc::make_mut(fallbacks.entry(key.clone()).or_default());
        let index = match set
            .fonts
            .iter()
            .position(|font| font.postscript_name == fallback.postscript_name)
        {
            Some(index) => index,
            None => {
                set.fonts.push(fallback);
                set.fonts.len() - 1
            }
        } as u16
            + 1;
        let choices = set.choices.entry(cluster.to_owned()).or_default();
        if choices.iter().any(|(s, _)| *s == script) {
            return false;
        }
        choices.push((script, index));
        true
    }

    pub fn postscript_name(&self) -> String {
        self.postscript_name.clone()
    }

    pub fn ascent(&self) -> FontUnit {
        scale_ext(
            self.upem,
            self.hb_font
                .get_font_h_extents()
                .map(|f| f.ascender)
                .unwrap_or_default(),
        )
    }

    pub fn descent(&self) -> FontUnit {
        scale_ext(
            self.upem,
            self.hb_font
                .get_font_h_extents()
                .map(|f| f.descender)
                .unwrap_or_default(),
        )
    }

    pub fn advance(&self, glyph: GlyphIndex) -> FontUnit {
        *self
            .glyph_advances
            .lock()
            .entry(glyph)
            .or_insert_with(|| scale_ext(self.upem, self.hb_font.get_glyph_h_advance(glyph.0)))
    }

    /// Whether the font has an OpenType `MATH` table.
    pub fn has_math_table(&self) -> bool {
        // SAFETY: the font is alive; `hb_font_get_face` returns its face.
        unsafe { sys::hb_ot_math_has_data(sys::hb_font_get_face(self.hb_font.as_raw())) != 0 }
    }

    pub fn scale_to_px(&self, val: FontUnit, font_size: PixelLength) -> PixelLength {
        val.0 as f32 * font_size / 1000.0f32
    }

    pub fn font_height(&self) -> FontUnit {
        self.ascent() - self.descent()
    }
}

/// A string from the `name` table of a font (`id` is a `HB_OT_NAME_ID_*`).
pub(crate) fn name_of(face: &HbFace, id: u32) -> Option<String> {
    let raw = face.as_raw();
    let language = std::ptr::null();
    let mut size = 0;
    // SAFETY: with a null buffer harfbuzz only reports the length of the name.
    let length =
        unsafe { sys::hb_ot_name_get_utf8(raw, id, language, &mut size, std::ptr::null_mut()) };
    if length == 0 {
        return None;
    }
    // room for the terminating NUL that harfbuzz appends
    let mut buffer = vec![0u8; length as usize + 1];
    let mut size = buffer.len() as u32;
    // SAFETY: `buffer` has room for `size` bytes.
    unsafe { sys::hb_ot_name_get_utf8(raw, id, language, &mut size, buffer.as_mut_ptr().cast()) };
    buffer.truncate(size as usize);
    String::from_utf8(buffer).ok()
}

/// Whether the fonts installed on the system are used; if not, only the fonts built into Helium
/// are. Unit tests use only the built-in fonts by default, so
/// that their results do not depend on the machine.
static USE_SYSTEM_FONTS: AtomicBool = AtomicBool::new(!cfg!(test));

/// Sets whether the fonts installed on the system are used (see `USE_SYSTEM_FONTS`). Must be
/// called before the first font is looked up: the installed fonts are scanned only once.
pub fn set_use_system_fonts(enabled: bool) {
    USE_SYSTEM_FONTS.store(enabled, Ordering::Relaxed);
}

/// Whether the fonts installed on the system are used.
pub(crate) fn use_system_fonts() -> bool {
    USE_SYSTEM_FONTS.load(Ordering::Relaxed)
}

/// The fonts installed on the system (unless disabled, see `use_system_fonts`) and the fonts
/// built into Helium, scanned once.
fn system_fonts() -> &'static fontdb::Database {
    static DATABASE: OnceLock<fontdb::Database> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut db = fontdb::Database::new();
        if use_system_fonts() {
            db.load_system_fonts();
        }
        // Loaded after the system fonts so that installed fonts win ties; the bundled
        // families are only used when asked for by name or when nothing else matches.
        let dejavu: &[&[u8]] = &[BUNDLED_REGULAR, BUNDLED_BOLD];
        for data in TEX_GYRE.iter().chain(MATH_FONTS).chain(EMBEDDED_FONTS).chain(dejavu) {
            db.load_font_source(fontdb::Source::Binary(Arc::new(*data)));
        }
        // `fontdb` names fixed families for the generic ones, which are rarely installed on
        // Linux; use what the system prefers, or else the bundled TeX Gyre family. DejaVu is
        // what fontconfig answers on an unconfigured system, so it is not taken as a
        // preference: DejaVu stays a fallback for characters (see `fallback`) but does not
        // decide the look of the default fonts.
        let has = |db: &fontdb::Database, name: &str| {
            db.faces()
                .any(|face| face.families.iter().any(|(f, _)| f.eq_ignore_ascii_case(name)))
        };
        let generics: [(&str, &[&str]); 3] = [
            ("sans-serif", &["TeX Gyre Heros"]),
            ("serif", &["TeX Gyre Termes"]),
            ("monospace", &["TeX Gyre Cursor"]),
        ];
        for (generic, candidates) in generics {
            let family = use_system_fonts()
                .then(|| super::fallback::default_family(generic))
                .flatten()
                .filter(|family| !family.starts_with("DejaVu") && has(&db, family))
                .or_else(|| candidates.iter().find(|c| has(&db, c)).map(|c| c.to_string()));
            if let Some(family) = family {
                match generic {
                    "sans-serif" => db.set_sans_serif_family(family),
                    "serif" => db.set_serif_family(family),
                    _ => db.set_monospace_family(family),
                }
            }
        }
        db
    })
}

/// The installed font that best matches the first available of `families`.
pub(crate) fn system_font(
    families: &[FontFamily],
    weight: &FontWeight,
    style: &FontStyle,
) -> Option<Font> {
    let db = system_fonts();
    // A requested standard font name is followed by its bundled stand-in, which is only used
    // when the name itself is not installed.
    let families = families
        .iter()
        .flat_map(|family| match family {
            FontFamily::Monospace => vec![fontdb::Family::Monospace],
            FontFamily::SansSerif => vec![fontdb::Family::SansSerif],
            FontFamily::Serif => vec![fontdb::Family::Serif],
            FontFamily::Named(name) => {
                let alias = BUNDLED_ALIASES
                    .iter()
                    .find(|(from, _)| from.eq_ignore_ascii_case(name))
                    .map(|(_, to)| fontdb::Family::Name(to));
                [Some(fontdb::Family::Name(name)), alias].into_iter().flatten().collect()
            }
        })
        .collect::<Vec<_>>();
    let id = db.query(&fontdb::Query {
        families: &families,
        weight: fontdb::Weight(weight.0.round().clamp(1.0, 1000.0) as u16),
        stretch: fontdb::Stretch::Normal,
        style: match style {
            FontStyle::Normal => fontdb::Style::Normal,
            FontStyle::Italic => fontdb::Style::Italic,
            FontStyle::Oblique => fontdb::Style::Oblique,
        },
    })?;
    let (data, index) = db.with_face_data(id, |data, index| (data.to_vec(), index))?;
    Font::load(Arc::new(data), index)
}

/// The font for `style`: the first installed family of `font-family`, else the system's
/// sans-serif font, else the font built into Helium.
pub fn match_font(style: &ComputedValues) -> Font {
    let families = style
        .font
        .font_family
        .iter()
        .cloned()
        .chain([FontFamily::SansSerif])
        .collect::<Vec<_>>();
    system_font(&families, &style.font.font_weight, &style.font.font_style)
        .unwrap_or_else(|| Font::bundled(&style.font.font_weight))
}

/// The font for MathML text: the font of `font-family` if it has a `MATH` table, else Latin
/// Modern Math, else STIX Two Math (both are built in), else the font of `match_font`.
pub fn match_math_font(style: &ComputedValues) -> Font {
    let font = match_font(style);
    if font.has_math_table() {
        return font;
    }
    let bundled = MATH_FAMILIES
        .iter()
        .map(|name| [FontFamily::Named((*name).to_owned())])
        .find_map(|family| system_font(&family, &style.font.font_weight, &style.font.font_style))
        .filter(Font::has_math_table);
    bundled.unwrap_or(font)
}

#[cfg(test)]
mod tests {
    use super::{ComputedValues, Font, FontWeight, match_font};
    use crate::style::properties::specified::FontFamily;

    #[test]
    fn missing_system_font_falls_back_to_an_embeddable_font() {
        let mut style = ComputedValues::default();
        std::sync::Arc::make_mut(&mut style.font).font_family =
            vec![FontFamily::Named("Helium Missing Font".to_owned())];

        let font = match_font(&style);

        assert!(!font.postscript_name().is_empty());
        assert!(font.covers('a'));
    }

    #[test]
    fn standard_font_names_resolve_without_installed_fonts() {
        use super::system_font;
        let style = crate::style::properties::computed::FontStyle::Italic;
        for family in ["Helvetica", "Arial", "Times", "Times New Roman", "Courier", "Courier New"] {
            let font = system_font(
                &[FontFamily::Named(family.to_owned())],
                &FontWeight::BOLD,
                &style,
            )
            .expect(family);
            // an installed font of that name is fine too
            assert!(font.covers('a'), "{family}");
        }
        for generic in [FontFamily::SansSerif, FontFamily::Serif, FontFamily::Monospace] {
            assert!(system_font(&[generic], &FontWeight::NORMAL, &style).is_some());
        }
    }

    #[test]
    fn embedded_fonts_cover_the_major_scripts() {
        use super::EMBEDDED_FONTS;
        let fonts: Vec<Font> = EMBEDDED_FONTS
            .iter()
            .map(|data| Font::load(std::sync::Arc::new(data.to_vec()), 0).expect("embedded fonts are valid"))
            .collect();
        for ch in "中漢字あカ한ع پ ٹ ی ת क ব த ไ ລ ក မ ა ա አ ∑ ♥".chars().filter(|c| !c.is_whitespace()) {
            assert!(fonts.iter().any(|font| font.covers(ch)), "no embedded font covers {ch}");
        }
    }

    #[test]
    fn bundled_fonts_load() {
        let regular = Font::bundled(&FontWeight::NORMAL);
        let bold = Font::bundled(&FontWeight::BOLD);
        assert_eq!(regular.postscript_name(), "DejaVuSans");
        assert_eq!(bold.postscript_name(), "DejaVuSans-Bold");
        assert!(regular.covers('→'));
        assert!(regular.is_truetype());
    }

    #[test]
    fn numeric_weights_pick_the_closer_face() {
        use super::system_font;
        use crate::style::properties::computed::FontStyle;
        for (weight, name) in [
            (100.0, "DejaVuSans"),
            (400.0, "DejaVuSans"),
            (500.0, "DejaVuSans"),
            (600.0, "DejaVuSans-Bold"),
            (700.0, "DejaVuSans-Bold"),
            (900.0, "DejaVuSans-Bold"),
        ] {
            assert_eq!(Font::bundled(&FontWeight(weight)).postscript_name(), name, "{weight}");
            // the installed faces are matched like the bundled ones
            let installed = system_font(
                &[FontFamily::Named("DejaVu Sans".into())],
                &FontWeight(weight),
                &FontStyle::Normal,
            )
            .unwrap();
            assert_eq!(installed.postscript_name(), name, "{weight}");
        }
    }
}
#[cfg(test)]
pub(super) fn bundled_font_data() -> Vec<Vec<u8>> {
    let dejavu: &[&[u8]] = &[BUNDLED_REGULAR, BUNDLED_BOLD];
    TEX_GYRE.iter().chain(MATH_FONTS).chain(EMBEDDED_FONTS).chain(dejavu).map(|d| d.to_vec()).collect()
}
