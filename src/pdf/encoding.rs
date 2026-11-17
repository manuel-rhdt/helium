//! How the glyphs of a font are written in the strings of the content streams.
//!
//! The display list shows glyphs by their index in the font. The strings of a content stream
//! are made of the codes of a font, which the `/Encoding` of the font turns into CIDs, and the
//! CIDs of the embedded font program select its glyphs. The encoding of a font is chosen once,
//! from the glyphs that the document uses, and everything that writes or reads codes asks it:
//! the strings of text, the text of SVG images, the `ToUnicode` map.

use crate::font_usage::GlyphUsage;
use crate::layout::text::font::{Font, GlyphIndex};

/// How the glyphs of one font are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlyphEncoding {
    /// `Identity-H`: a code is two bytes, big endian, and is the CID. The embedded font program
    /// keeps the glyph indices of the font (see `pdf::font`), so a glyph's code is its index.
    Identity,
}

impl GlyphEncoding {
    /// The encoding of `font`, which draws the glyphs of `usage`. This is where a font gets an
    /// encoding of its own, like one whose codes are the characters of the text; every font
    /// has the identity encoding for now.
    pub fn choose(_font: &Font, _usage: &GlyphUsage) -> Self {
        GlyphEncoding::Identity
    }

    /// The code that shows `glyph`, whose value is the one of a two byte code.
    pub fn code(self, glyph: GlyphIndex) -> u16 {
        match self {
            GlyphEncoding::Identity => glyph.0 as u16,
        }
    }

    /// The name of the predefined CMap that is the `/Encoding` of the font.
    pub fn cmap_name(self) -> &'static str {
        match self {
            GlyphEncoding::Identity => "Identity-H",
        }
    }
}
