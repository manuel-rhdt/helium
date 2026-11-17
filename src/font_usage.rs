//! Which glyphs of which fonts a document draws, and what text they stand for.
//!
//! The usage is worked out from the finished display list, not noted down while the text is laid
//! out: the layout also measures and sets text that ends up elsewhere or nowhere, and what a
//! backend embeds has to be what is drawn. It is the same for every backend: PDF subsets the
//! fonts to the glyphs and maps them to Unicode, HTML would do the same with web fonts.

use std::collections::{BTreeMap, BTreeSet};

use crate::display_list::{DisplayItem, PrintPages};
use crate::layout::components::TextOps;
use crate::layout::text::clusters::PieceClusters;
use crate::layout::text::font::GlyphIndex;
use crate::svg_text::SvgFontNames;

/// What is known about the use of the glyphs of one font.
#[derive(Debug, Default)]
pub struct GlyphUsage {
    glyphs: BTreeSet<GlyphIndex>,
    text: BTreeMap<GlyphIndex, String>,
    fallback_text: BTreeMap<GlyphIndex, String>,
}

impl GlyphUsage {
    /// The glyphs that are drawn.
    pub fn glyphs(&self) -> impl ExactSizeIterator<Item = GlyphIndex> + '_ {
        self.glyphs.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The text of a glyph that is drawn for a cluster of its own. A glyph has one text, the
    /// text of its first use: a glyph that is used for different text, or together with other
    /// glyphs for a cluster, is not described by a map from glyph to text, and the uses that
    /// differ have to say their text themselves.
    pub fn text(&self, glyph: GlyphIndex) -> Option<&str> {
        self.text.get(&glyph).map(String::as_str)
    }

    /// The text to give a glyph that [`text`](Self::text) has nothing for: the text of the first
    /// cluster that it is drawn in, where it is not alone or not the same. PDF/UA wants every
    /// glyph mapped to Unicode (ISO 14289-1 7.21.7), and this is better than nothing.
    pub fn fallback_text(&self, glyph: GlyphIndex) -> Option<&str> {
        self.fallback_text.get(&glyph).map(String::as_str)
    }
}

/// The usage of all fonts of a document, by the name of the font in the font map.
#[derive(Debug, Default)]
pub struct FontUsage {
    fonts: BTreeMap<String, GlyphUsage>,
}

impl FontUsage {
    /// The usage of the font with that name, if it draws a glyph.
    pub fn font(&self, name: &str) -> Option<&GlyphUsage> {
        self.fonts.get(name)
    }

    /// The fonts that draw a glyph, in the order of their names.
    pub fn fonts(&self) -> impl Iterator<Item = (&str, &GlyphUsage)> {
        self.fonts.iter().map(|(name, usage)| (name.as_str(), usage))
    }

    fn entry(&mut self, name: &str) -> &mut GlyphUsage {
        if !self.fonts.contains_key(name) {
            self.fonts.insert(name.to_owned(), GlyphUsage::default());
        }
        self.fonts.get_mut(name).expect("the font was just inserted")
    }

    /// Records the glyphs that `ops` draws, and the text of the pieces that they were shaped
    /// from: a cluster of one glyph gives it that text, unless the glyph was drawn for other text
    /// before; the text of every other cluster is only kept as a fallback.
    pub fn record_text(&mut self, ops: &TextOps) {
        let contexts = ops.glyph_contexts();
        for (glyph, context) in ops.glyphs.iter().zip(&contexts) {
            if !context.font.is_empty() {
                self.entry(context.font).glyphs.insert(glyph.glyph);
            }
        }
        let mut clusters = PieceClusters::default();
        for piece in &ops.pieces {
            clusters.set(&ops.glyphs, piece);
            for cluster in clusters.iter(ops.piece_text(piece)) {
                if cluster.text.is_empty() {
                    continue;
                }
                let alone = match cluster.glyphs {
                    &[only] => Some(only as usize),
                    _ => None,
                };
                for &index in cluster.glyphs {
                    let glyph = ops.glyphs[index as usize].glyph;
                    let font = contexts[index as usize].font;
                    if font.is_empty() {
                        continue;
                    }
                    let usage = self.entry(font);
                    if alone == Some(index as usize) && !usage.text.contains_key(&glyph) {
                        usage.text.insert(glyph, cluster.text.to_owned());
                    } else if usage.text.get(&glyph).map(String::as_str) != Some(cluster.text) || alone.is_none() {
                        usage.fallback_text.entry(glyph).or_insert_with(|| cluster.text.to_owned());
                    }
                }
            }
        }
    }

    /// Records a glyph that is drawn for `text` (the text of SVG images, whose glyphs are shown
    /// by the PDF writer). A glyph that is used for different text, or one that stands for more
    /// than a character, gets the text of its first use as its fallback text.
    pub fn record_glyph(&mut self, font: &str, glyph: GlyphIndex, text: &str) {
        let usage = self.entry(font);
        usage.glyphs.insert(glyph);
        if text.is_empty() {
            return;
        }
        if text.chars().count() == 1 && usage.text.get(&glyph).is_none_or(|existing| existing == text) {
            usage.text.insert(glyph, text.to_owned());
            return;
        }
        usage.fallback_text.entry(glyph).or_insert_with(|| text.to_owned());
    }
}

/// The usage of the fonts by the display lists of `print_pages`, and the fonts of the text of
/// the SVG images in them (see [`crate::svg_text::register_glyphs`]), which are added to the
/// font map of `print_pages` if they are not part of it yet.
pub fn collect(print_pages: &mut PrintPages) -> (FontUsage, SvgFontNames) {
    let mut usage = FontUsage::default();
    let mut svg_font_names = SvgFontNames::new();
    let mut seen = std::collections::HashSet::new();
    let display_lists = print_pages
        .islands
        .iter_mut()
        .map(|island| &mut island.display_list)
        .chain(print_pages.pages.iter_mut().map(|page| &mut page.display_list));
    for display_list in display_lists {
        for item in display_list.items() {
            match item {
                DisplayItem::Paragraph(paragraph) => usage.record_text(&paragraph.ops),
                DisplayItem::Image { image, .. } => {
                    if let crate::image::LoadedImage::Svg(svg) = &**image {
                        crate::svg_text::register_glyphs(
                            &svg.tree,
                            &mut print_pages.font_map,
                            &mut svg_font_names,
                            &mut usage,
                            &mut seen,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    (usage, svg_font_names)
}
