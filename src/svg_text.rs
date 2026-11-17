//! The text of SVG images is shaped by the code that shapes the text of the document, with the
//! fonts of the document: `usvg` lays the text out (anchors, `tspan`, paths), and asks the
//! [`HeliumShaper`] for the glyphs of every run. The fonts of the characters that the font of the
//! text has no glyphs for are chosen by the font fallback of the document
//! (`layout::text::fallback`), like for any other text. (The SVG parser does not shape text or
//! look for fallback fonts itself.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use usvg::{ShapeRequest, ShapedGlyph, TextShaper};

use crate::layout::FontMap;
use crate::layout::text::fallback::{FallbackKey, FontResolver, register_font};
use crate::layout::text::font::{Font, FontContext};
use crate::layout::text::shape_directed;
use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight, Language};

/// What the shaper keeps between the runs of the text of all SVG images: the fonts that it
/// loaded, and the fallback fonts that the resolver found for them.
#[derive(Default)]
struct State {
    resolver: FontResolver,
    /// Every font once, by PostScript name (see `register_font`).
    font_map: FontMap,
    /// The font of a face of the font database of the images.
    primary: HashMap<usvg::fontdb::ID, Arc<Font>>,
}

pub struct HeliumShaper {
    state: Mutex<State>,
    /// The fonts of the document that the images are in.
    fonts: std::sync::Weak<FontContext>,
}

impl HeliumShaper {
    pub fn new(fonts: std::sync::Weak<FontContext>) -> Self {
        let mut state = State::default();
        if let Some(fonts) = fonts.upgrade() {
            state.resolver.set_fonts(fonts);
        }
        HeliumShaper { state: Mutex::new(state), fonts }
    }
}

impl std::fmt::Debug for HeliumShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeliumShaper").finish()
    }
}

/// The font families of `font` as the families of the document.
pub fn helium_families(font: &usvg::Font) -> Vec<FontFamily> {
    font.families()
        .iter()
        .map(|family| match family {
            usvg::FontFamily::Serif => FontFamily::Serif,
            usvg::FontFamily::SansSerif => FontFamily::SansSerif,
            usvg::FontFamily::Monospace => FontFamily::Monospace,
            // the document has no families for these: the closest generic ones
            usvg::FontFamily::Cursive | usvg::FontFamily::Fantasy => FontFamily::SansSerif,
            // the family for text without a `font-family` is a name (`Options::font_family`), which
            // can be a generic family
            usvg::FontFamily::Named(name) => match name.to_ascii_lowercase().as_str() {
                "serif" => FontFamily::Serif,
                "sans-serif" | "cursive" | "fantasy" => FontFamily::SansSerif,
                "monospace" => FontFamily::Monospace,
                _ => FontFamily::Named(name.clone()),
            },
        })
        .collect()
}

pub fn helium_style(font: &usvg::Font) -> FontStyle {
    match font.style() {
        usvg::FontStyle::Normal => FontStyle::Normal,
        usvg::FontStyle::Italic => FontStyle::Italic,
        usvg::FontStyle::Oblique => FontStyle::Oblique,
    }
}

impl TextShaper for HeliumShaper {
    fn shape(&self, request: &ShapeRequest<'_>) -> Option<Vec<ShapedGlyph>> {
        let context = self.fonts.upgrade()?;
        let mut state = self.state.lock().ok()?;
        let State { resolver, font_map, primary } = &mut *state;

        let font = match primary.get(&request.id) {
            Some(font) => font.clone(),
            None => {
                let font = Font::load(Arc::new(request.data.to_vec()), request.index)?;
                let font = register_font(font_map, font);
                primary.insert(request.id, font.clone());
                font
            }
        };

        let mut families = helium_families(request.font);
        // like the text of the document, which ends with the generic family of its design
        families.push(FontFamily::SansSerif);
        let key = FallbackKey::from_parts(
            Language::default(),
            families,
            FontWeight(f32::from(request.font.weight())),
            helium_style(request.font),
        );
        let mut fonts = resolver.ensure_coverage_for_key(key.clone(), &font, request.text, font_map);
        let mut features = Vec::new();
        if request.small_caps {
            features.push((*b"smcp", 1));
        }
        if !request.kerning {
            features.push((*b"kern", 0));
        }
        fonts.features = features.into();

        let fallbacks = font.fallbacks(&key);
        let mut shaped = shape_directed(request.text, &font, &fonts, None, request.rtl);
        if request.rtl {
            // right-to-left text is in logical order, and the glyphs of the shaper are in visual
            // order (left to right)
            shaped.reverse();
        }
        Some(
            shaped
                .into_iter()
                .map(|(position, info)| {
                    let glyph_font = match info.font {
                        0 => Some(&font),
                        n => fallbacks.get(n as usize - 1),
                    };
                    // a glyph of a font that the images do not have is a missing glyph
                    let face = glyph_font.and_then(|font| Some((context.svg_face_id(font)?, font.upem)));
                    let (id, upem) = face.unwrap_or((request.id, font.upem));
                    let missing = face.is_none();
                    // from thousandths of an em to the units of the font
                    let units = |value: crate::layout::text::font::FontUnit| {
                        (value.0 * f64::from(upem) / 1000.0).round() as i32
                    };
                    ShapedGlyph {
                        cluster: info.cluster as usize,
                        glyph_id: if missing { 0 } else { info.codepoint },
                        font: id,
                        x_offset: units(position.x_offset),
                        y_offset: units(position.y_offset),
                        x_advance: units(position.x_advance),
                    }
                })
                .collect(),
        )
    }

    fn resolves_fallback(&self) -> bool {
        true
    }
}

/// The fonts that the glyphs of the SVG text use, by their ID in the font database of the SVG
/// images, as the PostScript names of fonts in the font map of the document.
pub type SvgFontNames = HashMap<usvg::fontdb::ID, String>;

/// Records the glyphs of the text in `tree` (and in the SVG images inside of it) in `usage`, so
/// that the PDF writer can show them as text with an embedded font. A font that the document
/// does not have yet is added to `font_map`. `names` tells which font of the map is meant by an
/// ID of the font database. `seen` holds the trees that were recorded already.
pub fn register_glyphs(
    tree: &usvg::Tree,
    font_map: &mut crate::layout::FontMap,
    names: &mut SvgFontNames,
    usage: &mut crate::font_usage::FontUsage,
    seen: &mut std::collections::HashSet<*const usvg::Tree>,
) {
    if !seen.insert(tree as *const usvg::Tree) {
        return;
    }
    register_group(tree.root(), tree.fontdb(), font_map, names, usage, seen);
}

fn register_group(
    group: &usvg::Group,
    db: &usvg::fontdb::Database,
    font_map: &mut crate::layout::FontMap,
    names: &mut SvgFontNames,
    usage: &mut crate::font_usage::FontUsage,
    seen: &mut std::collections::HashSet<*const usvg::Tree>,
) {
    use crate::layout::text::font::{Font, GlyphIndex};
    for node in group.children() {
        match node {
            usvg::Node::Group(inner) => register_group(inner, db, font_map, names, usage, seen),
            usvg::Node::Image(image) => {
                if let usvg::ImageKind::SVG(tree) = image.kind() {
                    register_glyphs(tree, font_map, names, usage, seen);
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
                        if font_map.contains_key(&name) {
                            usage.record_glyph(&name, GlyphIndex(u32::from(glyph.id.0)), &glyph.text);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
