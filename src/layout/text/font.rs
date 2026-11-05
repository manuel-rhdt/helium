pub use super::HarfbuzzFont;
pub use lemon_pdf::font::FontUnit;

use crate::style::properties::computed::{
    ComputedValues, FontFamily, FontStyle, FontWeight, PixelLength,
};

use font_kit;
use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::{handle::Handle, source::SystemSource};

use harfbuzz_rs::{Blob, Face as HbFace, Font as HbFont};
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
    pub(crate) handle: Handle,
    /// Fonts that provide the characters this font has no glyph for, in order of preference.
    ///
    /// They are filled in during box construction (see `fallback::FontResolver`), when the text
    /// that is rendered with this font is known, and used by `shape`. Every font in here is also
    /// an entry of the `FontMap`, so that it is embedded into the PDF.
    pub(crate) fallbacks: RwLock<Vec<Arc<Font>>>,
}

impl Clone for Font {
    fn clone(&self) -> Self {
        Font {
            upem: self.upem,
            glyph_advances: Mutex::new(self.glyph_advances.lock().clone()),
            to_unicode: Mutex::new(self.to_unicode.lock().clone()),
            handle: self.handle.clone(),
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
    /// Loads a font, or returns `None` if the font file cannot be read.
    pub(crate) fn load(handle: Handle) -> Option<Self> {
        let loaded = handle.load().ok()?;

        let upem = loaded.metrics().units_per_em;
        let font_bytes = loaded.copy_font_data()?;
        let index = match handle {
            Handle::Memory { font_index, .. } => font_index,
            Handle::Path { font_index, .. } => font_index,
        };
        let name = loaded
            .postscript_name()
            .unwrap_or_else(|| loaded.full_name());
        let blob = Blob::with_bytes_owned(font_bytes, |b| &*b);
        let face = HbFace::new(blob, index);
        let hb_font = HbFont::new(face).into();
        Some(Font {
            upem,
            glyph_advances: Default::default(),
            to_unicode: Default::default(),
            hb_font,
            postscript_name: name,
            handle,
            fallbacks: Default::default(),
        })
    }

    /// The font built into Helium (DejaVu Sans) in the given weight.
    pub fn bundled(weight: &FontWeight) -> Self {
        static REGULAR: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
        static BOLD: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
        let bytes = match weight {
            FontWeight::Normal => REGULAR.get_or_init(|| Arc::new(BUNDLED_REGULAR.to_vec())),
            FontWeight::Bold => BOLD.get_or_init(|| Arc::new(BUNDLED_BOLD.to_vec())),
        };
        Font::load(Handle::from_memory(bytes.clone(), 0)).expect("the bundled fonts are valid")
    }

    /// Whether this font has a glyph for `ch`.
    pub fn covers(&self, ch: char) -> bool {
        self.hb_font.get_nominal_glyph(ch).is_some()
    }

    /// A snapshot of the fallback fonts of this font.
    pub fn fallbacks(&self) -> Vec<Arc<Font>> {
        self.fallbacks.read().clone()
    }

    /// Appends `fallback` to the fallback fonts unless it is this font or already one of them.
    pub(crate) fn add_fallback(&self, fallback: Arc<Font>) {
        if fallback.postscript_name == self.postscript_name {
            return;
        }
        let mut fallbacks = self.fallbacks.write();
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

impl<'a> From<&'a ComputedValues> for Properties {
    fn from(computed_values: &'a ComputedValues) -> Properties {
        let mut props = Properties::new();
        props
            .style(computed_values.font.font_style.clone().into())
            .weight(computed_values.font.font_weight.clone().into());
        props
    }
}

/// The font for `style`: the first installed family of `font-family`, else the system's
/// sans-serif font, else the font built into Helium.
pub fn match_font(style: &ComputedValues) -> Font {
    let families = style
        .font
        .font_family
        .iter()
        .map(|fam| FamilyName::from(fam.clone()))
        .chain([FamilyName::SansSerif])
        .collect::<Vec<_>>();
    SystemSource::new()
        .select_best_match(&families, &style.into())
        .ok()
        .and_then(Font::load)
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

impl From<FontFamily> for FamilyName {
    fn from(family: FontFamily) -> FamilyName {
        match family {
            FontFamily::Monospace => FamilyName::Monospace,
            FontFamily::SansSerif => FamilyName::SansSerif,
            FontFamily::Serif => FamilyName::Serif,
            FontFamily::Named(name) => FamilyName::Title(name),
        }
    }
}

impl From<FontStyle> for font_kit::properties::Style {
    fn from(style: FontStyle) -> font_kit::properties::Style {
        match style {
            FontStyle::Normal => font_kit::properties::Style::Normal,
            FontStyle::Italic => font_kit::properties::Style::Italic,
            FontStyle::Oblique => font_kit::properties::Style::Oblique,
        }
    }
}

impl From<FontWeight> for font_kit::properties::Weight {
    fn from(weight: FontWeight) -> font_kit::properties::Weight {
        match weight {
            FontWeight::Normal => font_kit::properties::Weight::NORMAL,
            FontWeight::Bold => font_kit::properties::Weight::BOLD,
        }
    }
}
