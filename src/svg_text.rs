//! The text of SVG images is shaped by HarfBuzz, the library that shapes the text of the
//! document, with the fonts of the document. (The SVG parser does not shape text itself.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use harfbuzz_rs::{Blob, Direction, Face, Feature, Font, Owned, Shared, Tag, UnicodeBuffer, shape};
use usvg::{ShapedGlyph, TextShaper};

#[derive(Default)]
pub struct HarfbuzzShaper {
    fonts: Mutex<HashMap<usvg::fontdb::ID, Shared<Font<'static>>>>,
}

impl std::fmt::Debug for HarfbuzzShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarfbuzzShaper").finish()
    }
}

impl TextShaper for HarfbuzzShaper {
    fn shape(
        &self,
        id: usvg::fontdb::ID,
        data: &[u8],
        index: u32,
        text: &str,
        rtl: bool,
        small_caps: bool,
        kerning: bool,
    ) -> Option<Vec<ShapedGlyph>> {
        let font = {
            let mut fonts = self.fonts.lock().ok()?;
            fonts
                .entry(id)
                .or_insert_with(|| {
                    let blob = Blob::with_bytes_owned(Arc::new(data.to_vec()), |bytes| &**bytes);
                    let font: Owned<Font<'static>> = Font::new(Face::new(blob, index));
                    font.into()
                })
                .clone()
        };

        let direction = if rtl { Direction::Rtl } else { Direction::Ltr };
        let buffer = UnicodeBuffer::new()
            .add_str(text)
            .set_direction(direction)
            .guess_segment_properties();
        let mut features = Vec::new();
        if small_caps {
            features.push(Feature::new(Tag::new('s', 'm', 'c', 'p'), 1, ..));
        }
        if !kerning {
            features.push(Feature::new(Tag::new('k', 'e', 'r', 'n'), 0, ..));
        }
        let output = shape(&font, buffer, &features);
        Some(
            output
                .get_glyph_infos()
                .iter()
                .zip(output.get_glyph_positions())
                .map(|(info, position)| ShapedGlyph {
                    cluster: info.cluster as usize,
                    glyph_id: info.codepoint,
                    x_offset: position.x_offset,
                    y_offset: position.y_offset,
                    x_advance: position.x_advance,
                })
                .collect(),
        )
    }
}

/// The fonts that the glyphs of the SVG text use, by their ID in the font database of the SVG
/// images, as the PostScript names of fonts in the font map of the document.
pub type SvgFontNames = HashMap<usvg::fontdb::ID, String>;

/// Records the glyphs of the text in `tree` (and in the SVG images inside of it) in the fonts of
/// the document, so that the PDF writer can show them as text with an embedded font. A font that
/// the document does not have yet is added to `font_map`. `names` tells which font of the map is
/// meant by an ID of the font database. `seen` holds the trees that were recorded already.
pub fn register_glyphs(
    tree: &usvg::Tree,
    font_map: &mut crate::layout::FontMap,
    names: &mut SvgFontNames,
    seen: &mut std::collections::HashSet<*const usvg::Tree>,
) {
    if !seen.insert(tree as *const usvg::Tree) {
        return;
    }
    register_group(tree.root(), tree.fontdb(), font_map, names, seen);
}

fn register_group(
    group: &usvg::Group,
    db: &usvg::fontdb::Database,
    font_map: &mut crate::layout::FontMap,
    names: &mut SvgFontNames,
    seen: &mut std::collections::HashSet<*const usvg::Tree>,
) {
    use crate::layout::text::font::{Font, GlyphIndex};
    for node in group.children() {
        match node {
            usvg::Node::Group(inner) => register_group(inner, db, font_map, names, seen),
            usvg::Node::Image(image) => {
                if let usvg::ImageKind::SVG(tree) = image.kind() {
                    register_glyphs(tree, font_map, names, seen);
                }
            }
            usvg::Node::Text(text) => {
                for span in text.layouted() {
                    for glyph in &span.positioned_glyphs {
                        let name = match names.get(&glyph.font) {
                            Some(name) => name.clone(),
                            None => {
                                let Some(font) = db
                                    .with_face_data(glyph.font, |data, index| Font::load(Arc::new(data.to_vec()), index))
                                    .flatten()
                                else {
                                    continue;
                                };
                                let font = crate::layout::text::fallback::register_font(font_map, font);
                                let name = font.postscript_name();
                                names.insert(glyph.font, name.clone());
                                name
                            }
                        };
                        if let Some(font) = font_map.get(&name) {
                            font.record_glyph(GlyphIndex(u32::from(glyph.id.0)), &glyph.text);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
