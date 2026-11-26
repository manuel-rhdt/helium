pub use super::HarfbuzzFont;
pub use lemon_pdf::font::FontUnit;

use crate::style::font_face::{FontFaceRule, FontSource};
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
        // an emoji has the font that was chosen for it even if the primary font has a glyph
        let emoji = !primary.color && super::segment::is_emoji_cluster(cluster);
        if !emoji && primary.covers_cluster(cluster) {
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

/// A font file (a system font or one of the fonts built into Umbruch). Every font that is used
/// is embedded into the PDF.
#[derive(Debug)]
pub struct Font {
    pub(crate) upem: u32,
    pub(crate) hb_font: HarfbuzzFont,
    pub(crate) postscript_name: String,
    /// The bytes of the font file and the index of the face in it.
    pub(crate) data: Arc<Vec<u8>>,
    pub(crate) index: u32,
    /// Whether the font has colour glyphs that are painted from a `COLR` table (emoji fonts).
    /// Their base glyphs have no outline of their own: the glyph in the text only advances the
    /// pen, and `pdf::color_glyph` paints the layers.
    pub(crate) color: bool,
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
            data: self.data.clone(),
            index: self.index,
            color: self.color,
            hb_font: self.hb_font.clone(),
            postscript_name: self.postscript_name.clone(),
            fallbacks: RwLock::new(self.fallbacks.read().clone()),
        }
    }
}

// `BUNDLED_FONTS`: the fonts built into Umbruch, deflated and described by `build.rs`: the
// TeX Gyre fonts (free, metric-compatible stand-ins for the Helvetica, Times and Courier families
// of the PDF standard fonts, so that the generic families and the standard font names resolve
// without any installed fonts), the OpenType math fonts (Latin Modern Math and STIX Two Math,
// used for MathML when `font-family` does not name a font with a `MATH` table), the Noto fonts
// for the scripts that DejaVu Sans lacks and DejaVu Sans, which is used when no installed font
// matches the style, so that there always is a font that can be embedded.
use super::embedded::{EmbeddedFaceInfo, EmbeddedFont};
include!(concat!(env!("OUT_DIR"), "/fonts/bundled_fonts.rs"));

/// The families of the math fonts in order of preference.
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
        let color = ttf_parser::Face::parse(&data, index).is_ok_and(|face| face.tables().colr.is_some());
        Some(Font {
            color,
            upem,
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

    /// The font built into Umbruch (DejaVu Sans) in the given weight.
    pub fn bundled(weight: &FontWeight) -> Self {
        // the closer of the two weights, a tie goes to the heavier one like in font matching
        let font = if weight.0 >= 550.0 { DEJAVU_BOLD } else { DEJAVU_REGULAR };
        Font::load(BUNDLED_FONTS[font].data().clone(), 0).expect("the bundled fonts are valid")
    }

    /// Whether the font has colour glyphs painted from the `COLR` table.
    pub fn has_color_glyphs(&self) -> bool {
        self.color
    }

    /// Whether `glyph` is painted from the `COLR` table.
    pub fn is_color_glyph(&self, glyph: GlyphIndex) -> bool {
        self.color
            && ttf_parser::Face::parse(&self.data, self.index)
                .is_ok_and(|face| face.is_color_glyph(ttf_parser::GlyphId(glyph.0 as u16)))
    }

    /// Whether `other` is the same face of the same font file.
    pub(crate) fn is_same_file(&self, other: &Font) -> bool {
        self.index == other.index
            && (Arc::ptr_eq(&self.data, &other.data) || self.data == other.data)
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
        // most fonts have no fallbacks for most keys: share one empty set instead of allocating
        // one for every piece of text
        static EMPTY: OnceLock<Arc<FallbackSet>> = OnceLock::new();
        self.fallbacks
            .read()
            .get(key)
            .cloned()
            .unwrap_or_else(|| EMPTY.get_or_init(Default::default).clone())
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

    /// Whether the outlines of the font are in a `glyf` table (TrueType) and not in a CFF table.
    pub fn is_truetype(&self) -> bool {
        self.hb_font.face().table_with_tag(b"glyf").is_some()
    }

    /// The horizontal advance of `glyph`.
    pub fn advance(&self, glyph: GlyphIndex) -> FontUnit {
        scale_ext(self.upem, self.hb_font.get_glyph_h_advance(glyph.0))
    }

    /// Whether the font has an OpenType `MATH` table.
    pub fn has_math_table(&self) -> bool {
        // SAFETY: the font is alive; `hb_font_get_face` returns its face.
        unsafe { sys::hb_ot_math_has_data(sys::hb_font_get_face(self.hb_font.as_raw())) != 0 }
    }

    pub fn scale_to_px(&self, val: FontUnit, font_size: PixelLength) -> PixelLength {
        val.0 as f32 * font_size / 1000.0f32
    }

    /// The x-height at the font size: half of the size if the font does not have it.
    pub fn x_height_px(&self, font_size: PixelLength) -> PixelLength {
        match self.ot_metric(sys::HB_OT_METRICS_TAG_X_HEIGHT) {
            Some(height) if height > 0 => {
                PixelLength(height as f32 * font_size.0 / self.upem as f32)
            }
            _ => PixelLength(font_size.0 * 0.5),
        }
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

/// Whether the fonts installed on the system are used; if not, only the fonts built into Umbruch
/// are. Unit tests use only the built-in fonts by default, so
/// that their results do not depend on the machine.
static USE_SYSTEM_FONTS: AtomicBool = AtomicBool::new(!cfg!(test));

/// Sets whether the fonts installed on the system are used (see `USE_SYSTEM_FONTS`). Must be
/// called before the first font is looked up: the installed fonts are scanned only once.
pub fn set_use_system_fonts(enabled: bool) {
    USE_SYSTEM_FONTS.store(enabled, Ordering::Relaxed);
}

static EXTRA_FONT_DIRS: OnceLock<Vec<std::path::PathBuf>> = OnceLock::new();

/// Sets directories whose fonts are used in addition to the others. Must be called before the
/// first font is looked up, like `set_use_system_fonts`.
pub fn set_font_dirs(dirs: Vec<std::path::PathBuf>) {
    let _ = EXTRA_FONT_DIRS.set(dirs);
}

/// Whether the fonts installed on the system are used.
pub(crate) fn use_system_fonts() -> bool {
    USE_SYSTEM_FONTS.load(Ordering::Relaxed)
}

/// The built-in fonts in the font database, by ID.
static BUNDLED_IDS: OnceLock<HashMap<fontdb::ID, &'static EmbeddedFont>> = OnceLock::new();

/// The data and the face index of a face of the font database. The built-in fonts share their
/// inflated data, the data of other fonts is copied.
fn face_data(db: &fontdb::Database, id: fontdb::ID) -> Option<(Arc<Vec<u8>>, u32)> {
    if let Some(font) = BUNDLED_IDS.get().and_then(|ids| ids.get(&id)) {
        return Some((font.data().clone(), 0));
    }
    db.with_face_data(id, |data, index| (Arc::new(data.to_vec()), index))
}

/// The fonts installed on the system (unless disabled, see `use_system_fonts`) and the fonts
/// built into Umbruch, scanned once.
fn system_fonts() -> &'static fontdb::Database {
    system_database()
}

/// The database of `system_fonts`, shared by the font contexts of the documents.
fn system_database() -> &'static Arc<fontdb::Database> {
    static DATABASE: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut db = fontdb::Database::new();
        if use_system_fonts() {
            db.load_system_fonts();
        }
        for dir in EXTRA_FONT_DIRS.get().into_iter().flatten() {
            db.load_fonts_dir(dir);
        }
        // Loaded after the system fonts so that installed fonts win ties; the bundled
        // families are only used when asked for by name or when nothing else matches.
        let mut ids = HashMap::new();
        for font in &BUNDLED_FONTS {
            ids.insert(db.push_face_info(font.face_info()), font);
        }
        let _ = BUNDLED_IDS.set(ids);
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
        Arc::new(db)
    })
}

/// The families of the colour emoji fonts that are painted from a `COLR` table, in the order of
/// preference. Bitmap emoji fonts are not drawn (see `pdf::color_glyph`).
const EMOJI_FAMILIES: &[&str] = &[
    "Noto Color Emoji",
    "Twemoji Mozilla",
    "Twitter Color Emoji",
    "Segoe UI Emoji",
    "EmojiOne Color",
    "Fluent Emoji Color",
];

/// A colour emoji font with `COLR` glyphs: an installed one, or else Noto Color Emoji, which is
/// built into Umbruch (the fonts of the system come first in the database). A system can have the
/// same family as a bitmap font too (the `CBDT` build of Noto Color Emoji), which has to be
/// skipped.
pub(crate) fn color_emoji_font() -> Option<Font> {
    static FOUND: OnceLock<Option<(Arc<Vec<u8>>, u32)>> = OnceLock::new();
    let (data, index) = FOUND
        .get_or_init(|| {
            let db = system_fonts();
            EMOJI_FAMILIES.iter().find_map(|family| {
                db.faces()
                    .filter(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)))
                    .find_map(|face| {
                        let (data, index) = face_data(db, face.id)?;
                        let colr = ttf_parser::Face::parse(&data, index).is_ok_and(|face| face.tables().colr.is_some());
                        colr.then_some((data, index))
                    })
            })
        })
        .clone()?;
    Font::load(data, index)
}

/// The faces of a family that a document defines with `@font-face`.
#[derive(Debug)]
struct AuthorFace {
    id: fontdb::ID,
    /// The weights that the face is for.
    weight: (f32, f32),
    style: FontStyle,
}

/// The fonts that a document can use: the installed fonts and the ones built into Umbruch, and
/// the faces of its `@font-face` rules. The faces of a family that the document defines hide the
/// installed fonts of the same name.
///
/// There is one for every document, made from its style sheets (see `Styles::font_context`);
/// documents without `@font-face` rules share the one of the system.
pub struct FontContext {
    /// The faces of the installed fonts, of the built-in ones and of the `@font-face` rules. The
    /// same database is used for the text of SVG images.
    db: Arc<fontdb::Database>,
    /// The faces of the families defined by `@font-face`, by lowercase family name. A family
    /// whose sources cannot all be loaded is in here too, without faces: it does not fall back to
    /// the installed fonts of that name.
    author: HashMap<String, Vec<AuthorFace>>,
    /// The face of the database for a font, by PostScript name and size of the font file.
    face_ids: Mutex<BTreeMap<(String, usize), Option<fontdb::ID>>>,
    /// The shaper of the text of SVG images, shared by all of them so that it keeps the fonts it
    /// has loaded.
    shaper: OnceLock<Arc<crate::svg_text::UmbruchShaper>>,
}

impl std::fmt::Debug for FontContext {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("FontContext").field("author_families", &self.author.keys()).finish()
    }
}

impl FontContext {
    /// The fonts of the system (and the built-in ones) alone.
    pub fn system() -> Arc<FontContext> {
        static SYSTEM: OnceLock<Arc<FontContext>> = OnceLock::new();
        SYSTEM
            .get_or_init(|| Arc::new(FontContext::with_database(system_database().clone(), HashMap::new())))
            .clone()
    }

    fn with_database(db: Arc<fontdb::Database>, author: HashMap<String, Vec<AuthorFace>>) -> Self {
        FontContext { db, author, face_ids: Default::default(), shaper: Default::default() }
    }

    /// The fonts of a document with the given `@font-face` rules.
    pub fn new(rules: &[FontFaceRule]) -> Arc<FontContext> {
        if rules.is_empty() {
            return FontContext::system();
        }
        let system = system_database();
        // the faces in order of the rules; a family is defined even if none of its faces loads
        let mut loaded: HashMap<String, Vec<(fontdb::FaceInfo, (f32, f32), FontStyle)>> = HashMap::new();
        for rule in rules {
            let faces = loaded.entry(rule.family.to_lowercase()).or_default();
            match load_face(system, rule) {
                Some(info) => faces.push((info, rule.weight, rule.style.clone())),
                None => log::warn!(
                    "@font-face \"{}\": none of the sources could be used",
                    rule.family
                ),
            }
        }

        let mut db = (**system).clone();
        let hidden: Vec<fontdb::ID> = db
            .faces()
            .filter(|face| {
                face.families.iter().any(|(name, _)| loaded.contains_key(&name.to_lowercase()))
            })
            .map(|face| face.id)
            .collect();
        for id in hidden {
            db.remove_face(id);
        }
        let author = loaded
            .into_iter()
            .map(|(family, faces)| {
                let faces = faces
                    .into_iter()
                    .map(|(info, weight, style)| AuthorFace { id: db.push_face_info(info), weight, style })
                    .collect();
                (family, faces)
            })
            .collect();
        Arc::new(FontContext::with_database(Arc::new(db), author))
    }

    /// The font database of the document, which the text of SVG images uses too.
    pub(crate) fn database(&self) -> Arc<fontdb::Database> {
        self.db.clone()
    }

    /// The shaper of the text of SVG images.
    pub(crate) fn shaper(self: &Arc<Self>) -> Arc<dyn usvg::TextShaper> {
        self.shaper
            .get_or_init(|| Arc::new(crate::svg_text::UmbruchShaper::new(Arc::downgrade(self))))
            .clone()
    }

    /// The face of the font database that best matches the first available of `families`.
    pub(crate) fn font_id(
        &self,
        families: &[FontFamily],
        weight: &FontWeight,
        style: &FontStyle,
    ) -> Option<fontdb::ID> {
        families.iter().find_map(|family| {
            if let FontFamily::Named(name) = family {
                if let Some(faces) = self.author.get(&name.to_lowercase()) {
                    // the family is the document's: the installed fonts of that name are hidden
                    return best_author_face(faces, weight.0, style);
                }
            }
            query_family(&self.db, family, weight, style)
        })
    }

    /// The font that best matches the first available of `families`.
    pub(crate) fn font(
        &self,
        families: &[FontFamily],
        weight: &FontWeight,
        style: &FontStyle,
    ) -> Option<Font> {
        let id = self.font_id(families, weight, style)?;
        let (data, index) = face_data(&self.db, id)?;
        Font::load(data, index)
    }

    /// The font for `style`: the first available family of `font-family`, else the sans-serif
    /// font, else the font built into Umbruch.
    pub fn match_font(&self, style: &ComputedValues) -> Font {
        let families = style
            .font
            .font_family
            .iter()
            .cloned()
            .chain([FontFamily::SansSerif])
            .collect::<Vec<_>>();
        self.font(&families, &style.font.font_weight, &style.font.font_style)
            .unwrap_or_else(|| Font::bundled(&style.font.font_weight))
    }

    /// The font for MathML text: the font of `font-family` if it has a `MATH` table, else Latin
    /// Modern Math, else STIX Two Math (both are built in), else the font of `match_font`.
    pub fn match_math_font(&self, style: &ComputedValues) -> Font {
        let font = self.match_font(style);
        if font.has_math_table() {
            return font;
        }
        let bundled = MATH_FAMILIES
            .iter()
            .map(|name| [FontFamily::Named((*name).to_owned())])
            .find_map(|family| self.font(&family, &style.font.font_weight, &style.font.font_style))
            .filter(Font::has_math_table);
        bundled.unwrap_or(font)
    }

    /// The face of the font database that is `font`, if the font is in it: fonts that were found
    /// by other means (fontconfig) are not.
    pub(crate) fn svg_face_id(&self, font: &Font) -> Option<fontdb::ID> {
        let key = (font.postscript_name.clone(), font.data.len());
        let mut cache = self.face_ids.lock();
        if let Some(id) = cache.get(&key) {
            return *id;
        }
        let db = &*self.db;
        // fonts of the same name (the bitmap and the vector build of one family) are told apart
        // by their data
        let id = db
            .faces()
            .filter(|face| face.post_script_name == font.postscript_name)
            .find(|face| {
                db.with_face_data(face.id, |data, index| {
                    index == font.index && data == font.data.as_slice()
                })
                .unwrap_or(false)
            })
            .map(|face| face.id);
        cache.insert(key, id);
        id
    }
}

/// The face of the document's family whose descriptors match the request best (CSS Fonts 4,
/// § 5.2): first the style, then the weight. Of equal faces the one of the later rule is taken.
fn best_author_face(faces: &[AuthorFace], weight: f32, style: &FontStyle) -> Option<fontdb::ID> {
    // the preferred styles in order
    let styles: [FontStyle; 3] = match style {
        FontStyle::Normal => [FontStyle::Normal, FontStyle::Oblique, FontStyle::Italic],
        FontStyle::Italic => [FontStyle::Italic, FontStyle::Oblique, FontStyle::Normal],
        FontStyle::Oblique => [FontStyle::Oblique, FontStyle::Italic, FontStyle::Normal],
    };
    let candidates: Vec<&AuthorFace> = styles
        .iter()
        .find_map(|wanted| {
            let found: Vec<&AuthorFace> = faces.iter().filter(|face| &face.style == wanted).collect();
            (!found.is_empty()).then_some(found)
        })?;
    // a face that is for the weight, or else the nearest one in the direction that the weight
    // asks for: heavier first for weights above 500 and lighter first for those below 400, and
    // for the weights in between up to 500, then lighter ones, then heavier ones
    let covers = |face: &AuthorFace| face.weight.0 <= weight && weight <= face.weight.1;
    let heavier = |face: &AuthorFace| face.weight.0 > weight;
    let rank = |face: &AuthorFace| -> (u8, f32) {
        if covers(face) {
            return (0, 0.0);
        }
        // the distance from the requested weight to the nearest weight of the face
        let distance = if heavier(face) { face.weight.0 - weight } else { weight - face.weight.1 };
        let group = if weight < 400.0 {
            if heavier(face) { 2 } else { 1 }
        } else if weight > 500.0 {
            if heavier(face) { 1 } else { 2 }
        } else if heavier(face) {
            // 400 to 500: heavier faces up to 500 come first
            if face.weight.0 <= 500.0 { 1 } else { 3 }
        } else {
            2
        };
        (group, distance)
    };
    candidates
        .into_iter()
        .enumerate()
        // later faces win ties: compare the rank, then prefer the higher index
        .min_by(|(i, a), (j, b)| {
            let (ra, rb) = (rank(a), rank(b));
            ra.0.cmp(&rb.0)
                .then(ra.1.partial_cmp(&rb.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(j.cmp(i))
        })
        .map(|(_, face)| face.id)
}

/// The face of `db` that best matches `family`, with the bundled stand-in of a standard font
/// name, which is only used when the name itself is not installed.
fn query_family(
    db: &fontdb::Database,
    family: &FontFamily,
    weight: &FontWeight,
    style: &FontStyle,
) -> Option<fontdb::ID> {
    let families: Vec<fontdb::Family> = match family {
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
    };
    db.query(&fontdb::Query {
        families: &families,
        weight: fontdb::Weight(weight.0.round().clamp(1.0, 1000.0) as u16),
        stretch: fontdb::Stretch::Normal,
        style: match style {
            FontStyle::Normal => fontdb::Style::Normal,
            FontStyle::Italic => fontdb::Style::Italic,
            FontStyle::Oblique => fontdb::Style::Oblique,
        },
    })
}

/// The face for the first of the sources of `rule` that can be used, as an entry of a font
/// database under the family name of the rule.
fn load_face(system: &fontdb::Database, rule: &FontFaceRule) -> Option<fontdb::FaceInfo> {
    let style = match rule.style {
        FontStyle::Normal => fontdb::Style::Normal,
        FontStyle::Italic => fontdb::Style::Italic,
        FontStyle::Oblique => fontdb::Style::Oblique,
    };
    let families = vec![(rule.family.clone(), fontdb::Language::English_UnitedStates)];
    for source in &rule.sources {
        match source {
            FontSource::Local(name) => {
                let found = system
                    .faces()
                    .find(|face| face.post_script_name.eq_ignore_ascii_case(name))
                    .map(|face| face.id)
                    .or_else(|| {
                        query_family(
                            system,
                            &FontFamily::Named(name.clone()),
                            &FontWeight(rule.weight.0),
                            &rule.style,
                        )
                    })
                    .and_then(|id| system.face(id));
                match found {
                    Some(face) => {
                        let mut info = face.clone();
                        info.families = families;
                        info.style = style;
                        return Some(info);
                    }
                    None => log::info!("@font-face \"{}\": no installed font \"{}\"", rule.family, name),
                }
            }
            FontSource::Url(url) => {
                let data = match crate::resource::read(url, &rule.base_dir, &rule.base_dir) {
                    Ok(data) => data,
                    Err(error) => {
                        log::warn!("@font-face \"{}\": cannot read the font `{}`: {}", rule.family, url, error);
                        continue;
                    }
                };
                if data.starts_with(b"wOFF") || data.starts_with(b"wOF2") {
                    log::warn!("@font-face \"{}\": `{}` is a WOFF file, which is not supported", rule.family, url);
                    continue;
                }
                let data = Arc::new(data);
                let Some(font) = Font::load(data.clone(), 0) else {
                    log::warn!("@font-face \"{}\": `{}` is not a valid font", rule.family, url);
                    continue;
                };
                let monospaced = ttf_parser::Face::parse(&data, 0).is_ok_and(|face| face.is_monospaced());
                return Some(fontdb::FaceInfo {
                    id: fontdb::ID::dummy(),
                    source: fontdb::Source::Binary(data),
                    index: 0,
                    families,
                    post_script_name: font.postscript_name,
                    style,
                    weight: fontdb::Weight(rule.weight.0.round().clamp(1.0, 1000.0) as u16),
                    stretch: fontdb::Stretch::Normal,
                    monospaced,
                });
            }
        }
    }
    None
}

/// The installed font that best matches the first available of `families`.
#[cfg(test)]
pub(crate) fn system_font(
    families: &[FontFamily],
    weight: &FontWeight,
    style: &FontStyle,
) -> Option<Font> {
    FontContext::system().font(families, weight, style)
}

/// The font for `style` among the installed fonts (see `FontContext::match_font`).
pub fn match_font(style: &ComputedValues) -> Font {
    FontContext::system().match_font(style)
}

/// The font for MathML text among the installed fonts (see `FontContext::match_math_font`).
pub fn match_math_font(style: &ComputedValues) -> Font {
    FontContext::system().match_math_font(style)
}
#[cfg(test)]
mod tests {
    use super::{AuthorFace, ComputedValues, Font, FontStyle, FontWeight, best_author_face, match_font};
    use crate::style::properties::specified::FontFamily;

    #[test]
    fn missing_system_font_falls_back_to_an_embeddable_font() {
        let mut style = ComputedValues::default();
        std::sync::Arc::make_mut(&mut style.font).font_family =
            vec![FontFamily::Named("Umbruch Missing Font".to_owned())];

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
        let fonts: Vec<Font> = super::BUNDLED_FONTS
            .iter()
            .map(|font| Font::load(font.data().clone(), 0).expect("embedded fonts are valid"))
            .collect();
        for ch in "中漢字あカ한ع پ ٹ ی ת क ব த ไ ລ ក မ ა ա አ ∑ ♥".chars().filter(|c| !c.is_whitespace()) {
            assert!(fonts.iter().any(|font| font.covers(ch)), "no embedded font covers {ch}");
        }
    }

    #[test]
    fn the_embedded_emoji_font_paints_colour_glyphs() {
        let font = super::color_emoji_font().expect("an emoji font is built in");
        assert!(font.has_color_glyphs());
        for emoji in ["😀", "🎉", "🇩🇪", "👨\u{200D}👩\u{200D}👧", "👍🏽"] {
            assert!(font.covers_cluster(emoji), "{emoji}");
        }
        let glyph = font.hb_font.get_nominal_glyph('😀').unwrap();
        assert!(font.is_color_glyph(super::GlyphIndex(glyph)));
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

    fn faces(descriptors: &[(f32, f32, FontStyle)]) -> Vec<AuthorFace> {
        let mut db = fontdb::Database::new();
        descriptors
            .iter()
            .map(|(lo, hi, style)| AuthorFace {
                id: db.push_face_info(fontdb::FaceInfo {
                    id: fontdb::ID::dummy(),
                    source: fontdb::Source::Binary(std::sync::Arc::new(Vec::<u8>::new())),
                    index: 0,
                    families: vec![],
                    post_script_name: String::new(),
                    style: fontdb::Style::Normal,
                    weight: fontdb::Weight::NORMAL,
                    stretch: fontdb::Stretch::Normal,
                    monospaced: false,
                }),
                weight: (*lo, *hi),
                style: style.clone(),
            })
            .collect()
    }

    /// The index of the face that is chosen for `weight` and `style`.
    fn chosen(faces: &[AuthorFace], weight: f32, style: FontStyle) -> Option<usize> {
        let id = best_author_face(faces, weight, &style)?;
        faces.iter().position(|face| face.id == id)
    }

    #[test]
    fn author_faces_are_matched_like_css_fonts_4() {
        use FontStyle::{Italic, Normal, Oblique};
        let f = faces(&[(300.0, 300.0, Normal), (400.0, 400.0, Normal), (600.0, 600.0, Normal), (800.0, 800.0, Normal)]);
        // a face for the weight
        assert_eq!(chosen(&f, 400.0, Normal), Some(1));
        assert_eq!(chosen(&f, 800.0, Normal), Some(3));
        // above 500: heavier first, then lighter
        assert_eq!(chosen(&f, 700.0, Normal), Some(3));
        assert_eq!(chosen(&f, 900.0, Normal), Some(3));
        // below 400: lighter first, then heavier
        assert_eq!(chosen(&f, 200.0, Normal), Some(0));
        assert_eq!(chosen(&f, 350.0, Normal), Some(0));
        // 500: 400 if there is no 500, before the heavier faces
        assert_eq!(chosen(&f, 500.0, Normal), Some(1));
        // 400 to 500: a face between the weight and 500 comes first
        let g = faces(&[(300.0, 300.0, Normal), (450.0, 450.0, Normal), (700.0, 700.0, Normal)]);
        assert_eq!(chosen(&g, 400.0, Normal), Some(1));
        assert_eq!(chosen(&g, 500.0, Normal), Some(1));
        // ranges contain their weights
        let r = faces(&[(100.0, 300.0, Normal), (500.0, 900.0, Normal)]);
        assert_eq!(chosen(&r, 250.0, Normal), Some(0));
        assert_eq!(chosen(&r, 700.0, Normal), Some(1));
        // 400 looks at the weights up to 500 first, and the second face starts at 500
        assert_eq!(chosen(&r, 400.0, Normal), Some(1));
        assert_eq!(chosen(&r, 450.0, Normal), Some(1));
        assert_eq!(chosen(&r, 350.0, Normal), Some(0));
        // styles: italic prefers oblique, normal prefers oblique to italic; the style wins
        let s = faces(&[(400.0, 400.0, Normal), (700.0, 700.0, Italic), (400.0, 400.0, Oblique)]);
        assert_eq!(chosen(&s, 700.0, Normal), Some(0));
        assert_eq!(chosen(&s, 400.0, Italic), Some(1));
        assert_eq!(chosen(&s, 700.0, Oblique), Some(2));
        let o = faces(&[(400.0, 400.0, Normal), (400.0, 400.0, Oblique)]);
        assert_eq!(chosen(&o, 400.0, Italic), Some(1));
        // the later of equal faces wins
        let twice = faces(&[(400.0, 400.0, Normal), (400.0, 400.0, Normal)]);
        assert_eq!(chosen(&twice, 400.0, Normal), Some(1));
        assert_eq!(chosen(&[], 400.0, Normal), None);
    }
}
#[cfg(test)]
pub(super) fn bundled_font_data() -> Vec<Vec<u8>> {
    BUNDLED_FONTS.iter().map(|font| font.data().to_vec()).collect()
}

/// The x-height and the advance of `0` (`ex` and `ch`) of the font that `font` selects, at its
/// font size. A font without them has half of the font size for both.
pub(crate) fn unit_metrics(font: &crate::style::properties::computed::Font) -> (PixelLength, PixelLength) {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<HashMap<String, (f32, f32)>> = RefCell::new(HashMap::new());
    }
    let key = format!("{:?}|{:?}|{:?}", font.font_family, font.font_style, font.font_weight);
    let (ex, ch) = CACHE.with(|cache| {
        *cache.borrow_mut().entry(key).or_insert_with(|| {
            let mut style = ComputedValues::default();
            let face = Arc::make_mut(&mut style.font);
            face.font_family = font.font_family.clone();
            face.font_style = font.font_style.clone();
            face.font_weight = font.font_weight.clone();
            let matched = match_font(&style);
            let one = PixelLength(1.0);
            let ch = matched
                .hb_font
                .get_nominal_glyph('0')
                .map_or(0.5, |glyph| matched.scale_to_px(matched.advance(GlyphIndex(glyph)), one).0);
            (matched.x_height_px(one).0, ch)
        })
    });
    (PixelLength(ex * font.font_size.0), PixelLength(ch * font.font_size.0))
}
