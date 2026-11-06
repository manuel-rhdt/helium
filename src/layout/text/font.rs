pub use super::HarfbuzzFont;
pub use lemon_pdf::font::FontUnit;

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, Language, PixelLength,
};

use harfbuzz_rs::{Blob, Face as HbFace, Font as HbFont, HarfbuzzObject, sys};
use parking_lot::{Mutex, RwLock};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

fn scale_ext(upem: u32, metric: i32) -> FontUnit {
    let upem = upem as f64;
    FontUnit(metric as f64 * 1000f64 / upem)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct GlyphIndex(pub u32);

/// A font file (a system font or one of the fonts built into Helium). Every font that is used
/// is embedded into the PDF.
#[derive(Debug)]
pub struct Font {
    pub(crate) upem: u32,
    pub(crate) glyph_advances: Mutex<BTreeMap<GlyphIndex, FontUnit>>,
    pub(crate) to_unicode: Mutex<BTreeMap<GlyphIndex, String>>,
    pub(crate) hb_font: HarfbuzzFont,
    pub(crate) postscript_name: String,
    /// The bytes of the font file and the index of the face in it.
    pub(crate) data: Arc<Vec<u8>>,
    pub(crate) index: u32,
    /// Fonts that provide the characters this font has no glyph for, in order of preference, for
    /// text in each language: the preferred fallback fonts depend on the language (e.g. the
    /// Chinese or the Japanese forms of Han characters).
    ///
    /// They are filled in during box construction (see `fallback::FontResolver`), when the text
    /// that is rendered with this font is known, and used by `shape`. Every font in here is also
    /// an entry of the `FontMap`, so that it is embedded into the PDF.
    pub(crate) fallbacks: RwLock<BTreeMap<Language, Vec<Arc<Font>>>>,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Font {
            upem: self.upem,
            glyph_advances: Mutex::new(self.glyph_advances.lock().clone()),
            to_unicode: Mutex::new(self.to_unicode.lock().clone()),
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
        let bytes = match weight {
            FontWeight::Normal => REGULAR.get_or_init(|| Arc::new(BUNDLED_REGULAR.to_vec())),
            FontWeight::Bold => BOLD.get_or_init(|| Arc::new(BUNDLED_BOLD.to_vec())),
        };
        Font::load(bytes.clone(), 0).expect("the bundled fonts are valid")
    }

    /// Whether this font has a glyph for `ch`.
    pub fn covers(&self, ch: char) -> bool {
        self.hb_font.get_nominal_glyph(ch).is_some()
    }

    /// A snapshot of the fallback fonts of this font for text in `lang`.
    pub fn fallbacks(&self, lang: &Language) -> Vec<Arc<Font>> {
        self.fallbacks
            .read()
            .get(lang)
            .cloned()
            .unwrap_or_default()
    }

    /// Appends `fallback` to the fallback fonts for `lang` unless it is this font or already one
    /// of them.
    pub(crate) fn add_fallback(&self, lang: &Language, fallback: Arc<Font>) {
        if fallback.postscript_name == self.postscript_name {
            return;
        }
        let mut fallbacks = self.fallbacks.write();
        let fallbacks = fallbacks.entry(lang.clone()).or_default();
        if !fallbacks
            .iter()
            .any(|font| font.postscript_name == fallback.postscript_name)
        {
            fallbacks.push(fallback);
        }
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

/// The fonts installed on the system, scanned once.
fn system_fonts() -> &'static fontdb::Database {
    static DATABASE: OnceLock<fontdb::Database> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // `fontdb` names fixed families for the generic ones, which are rarely installed on
        // Linux; use what the system prefers, or else the first installed common family.
        let has = |db: &fontdb::Database, name: &str| {
            db.faces()
                .any(|face| face.families.iter().any(|(f, _)| f.eq_ignore_ascii_case(name)))
        };
        let generics: [(&str, &[&str]); 3] = [
            ("sans-serif", &["DejaVu Sans", "Noto Sans", "Liberation Sans", "Arial", "Helvetica"]),
            ("serif", &["DejaVu Serif", "Noto Serif", "Liberation Serif", "Times New Roman", "Times"]),
            ("monospace", &["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono", "Courier New", "Menlo"]),
        ];
        for (generic, candidates) in generics {
            let family = super::fallback::default_family(generic)
                .filter(|family| has(&db, family))
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
    let families = families
        .iter()
        .map(|family| match family {
            FontFamily::Monospace => fontdb::Family::Monospace,
            FontFamily::SansSerif => fontdb::Family::SansSerif,
            FontFamily::Serif => fontdb::Family::Serif,
            FontFamily::Named(name) => fontdb::Family::Name(name),
        })
        .collect::<Vec<_>>();
    let id = db.query(&fontdb::Query {
        families: &families,
        weight: match weight {
            FontWeight::Normal => fontdb::Weight::NORMAL,
            FontWeight::Bold => fontdb::Weight::BOLD,
        },
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
    fn bundled_fonts_load() {
        let regular = Font::bundled(&FontWeight::Normal);
        let bold = Font::bundled(&FontWeight::Bold);
        assert_eq!(regular.postscript_name(), "DejaVuSans");
        assert_eq!(bold.postscript_name(), "DejaVuSans-Bold");
        assert!(regular.covers('→'));
        assert!(regular.is_truetype());
    }
}
