//! How the glyphs of a font are written in the strings of the content streams.
//!
//! The display list shows glyphs by their index in the font. The strings of a content stream
//! are made of the codes of a font, which the `/Encoding` of the font turns into CIDs, and the
//! CIDs of the embedded font program select its glyphs. The encoding of a font is chosen once,
//! from the glyphs that the document uses, and everything that writes or reads codes asks it:
//! the strings of text, the text of SVG images, the widths, the `ToUnicode` map.

use std::sync::Arc;

use crate::font_usage::GlyphUsage;
use crate::layout::text::font::{Font, GlyphIndex};
use subsetter::GlyphRemapper;

/// How the glyphs of one font are encoded.
#[derive(Debug, Clone)]
pub(crate) enum GlyphEncoding {
    /// `Identity-H` with the glyph indices of the font as codes. Only what no font is selected
    /// for uses it.
    Identity,
    /// `Identity-H`: a code is two bytes, big endian, and is the CID. The embedded font program
    /// is a subset of the font whose glyphs are numbered anew: the notdef glyph is 0, followed
    /// by the glyphs that are drawn in the order of their indices. A CID is the new glyph index
    /// (`CIDToGIDMap` is the identity, and a CFF font is CID-keyed with an identity charset).
    Subset(Arc<GlyphRemapper>),
}

impl GlyphEncoding {
    /// The encoding of `font`, which draws the glyphs of `usage`.
    pub fn choose(_font: &Font, usage: &GlyphUsage) -> Self {
        let glyphs: Vec<u16> = usage.glyphs().map(|glyph| glyph.0 as u16).collect();
        GlyphEncoding::Subset(Arc::new(GlyphRemapper::new_from_glyphs_sorted(&glyphs)))
    }

    /// The code that shows `glyph`, whose value is the one of a two byte code. A glyph that
    /// the font is not subset to is shown as the notdef glyph.
    pub fn code(&self, glyph: GlyphIndex) -> u16 {
        match self {
            GlyphEncoding::Identity => glyph.0 as u16,
            GlyphEncoding::Subset(remapper) => remapper.get(glyph.0 as u16).unwrap_or(0),
        }
    }

    /// The glyph numbering of the embedded font program, if it is a subset.
    pub fn remapper(&self) -> Option<&GlyphRemapper> {
        match self {
            GlyphEncoding::Identity => None,
            GlyphEncoding::Subset(remapper) => Some(remapper),
        }
    }

    /// The name of the predefined CMap that is the `/Encoding` of the font.
    pub fn cmap_name(&self) -> &'static str {
        "Identity-H"
    }
}

/// The encoding of text in a font that is not embedded (nothing is drawn with it).
pub(crate) static IDENTITY: GlyphEncoding = GlyphEncoding::Identity;
