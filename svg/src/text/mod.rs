// Copyright 2024 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use fontdb::{Database, ID};
use svgtypes::FontFamily;

use crate::{Cache, Font, FontStretch, FontStyle, Text};

pub(crate) mod cache;
pub(crate) mod flatten;

/// Provides access to the layout of a text node.
pub mod layout;

/// The ID of a glyph within a font
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(pub u32);

/// A glyph that the [`TextShaper`] made, in logical order of the text it shaped.
///
/// All values are in font units of the font of the glyph.
#[derive(Copy, Clone, Debug)]
pub struct ShapedGlyph {
    /// The byte offset in the shaped text of the cluster that the glyph belongs to.
    pub cluster: usize,
    /// The glyph in the font.
    pub glyph_id: u32,
    /// The face of the font database that the glyph is from. It is the requested face, unless the
    /// shaper chooses fallback fonts (see [`TextShaper::resolves_fallback`]).
    pub font: ID,
    /// How far the glyph is moved horizontally from the pen.
    pub x_offset: i32,
    /// How far the glyph is moved vertically from the pen.
    pub y_offset: i32,
    /// How far the pen advances.
    pub x_advance: i32,
}

/// What a [`TextShaper`] shapes: a text, written in one direction, and the font for it.
#[derive(Debug)]
pub struct ShapeRequest<'a> {
    /// The face of the font database that was chosen for the text, which is the face `index` of
    /// the font file `data`.
    pub id: ID,
    pub data: &'a [u8],
    pub index: u32,
    /// The font that the text asked for: the families, weight and style, which a shaper that
    /// chooses fallback fonts needs.
    pub font: &'a Font,
    pub text: &'a str,
    pub rtl: bool,
    /// Turns the OpenType feature `smcp` on.
    pub small_caps: bool,
    /// Whether the OpenType feature `kern` is on.
    pub kerning: bool,
}

/// Shapes text: turns a string and a font into positioned glyphs.
///
/// This fork does not shape text itself; the caller provides the shaper, so that the text of
/// SVG images is shaped by the same code and with the same fonts as the text around it.
pub trait TextShaper: Send + Sync + std::fmt::Debug {
    /// Shapes the text of the request. The glyphs are returned in the visual order of the text
    /// (left to right), each with the byte offset of its cluster in the text and its font.
    fn shape(&self, request: &ShapeRequest<'_>) -> Option<Vec<ShapedGlyph>>;

    /// Whether the shaper chooses the fonts for the characters that the requested font has no
    /// glyphs for, and returns glyphs of other fonts. If not, the fork looks for fallback fonts
    /// in the font database itself, and a glyph with the id 0 is a missing one.
    fn resolves_fallback(&self) -> bool {
        false
    }
}

/// A shorthand for [FontResolver]'s font selection function.
///
/// This function receives a font specification (families + a style, weight,
/// stretch triple) and a font database and should return the ID of the font
/// that shall be used (if any).
///
/// In the basic case, the function will search the existing fonts in the
/// database to find a good match, e.g. via
/// [`Database::query`](fontdb::Database::query). This is what the [default
/// implementation](FontResolver::default_font_selector) does.
///
/// Users with more complex requirements can mutate the database to load
/// additional fonts dynamically. To perform mutation, it is recommended to call
/// `Arc::make_mut` on the provided database. (This call is not done outside of
/// the callback to not needless clone an underlying shared database if no
/// mutation will be performed.) It is important that the database is only
/// mutated additively. Removing fonts or replacing the entire database will
/// break things.
pub type FontSelectionFn<'a> =
    Box<dyn Fn(&Font, &mut Arc<Database>) -> Option<ID> + Send + Sync + 'a>;

/// A shorthand for [FontResolver]'s fallback selection function.
///
/// This function receives a specific character, a list of already used fonts,
/// and a font database. It should return the ID of a font that
/// - is not any of the already used fonts
/// - is as close as possible to the first already used font (if any)
/// - supports the given character
///
/// The function can search the existing database, but can also load additional
/// fonts dynamically. See the documentation of [`FontSelectionFn`] for more
/// details.
pub type FallbackSelectionFn<'a> =
    Box<dyn Fn(char, &[ID], &mut Arc<Database>) -> Option<ID> + Send + Sync + 'a>;

/// A font resolver for `<text>` elements.
///
/// This type can be useful if you want to have an alternative font handling to
/// the default one. By default, only fonts specified upfront in
/// [`Options::fontdb`](crate::Options::fontdb) will be used. This type allows
/// you to load additional fonts on-demand and customize the font selection
/// process.
pub struct FontResolver<'a> {
    /// Resolver function that will be used when selecting a specific font
    /// for a generic [`Font`] specification.
    pub select_font: FontSelectionFn<'a>,

    /// Resolver function that will be used when selecting a fallback font for a
    /// character.
    pub select_fallback: FallbackSelectionFn<'a>,
}

impl Default for FontResolver<'_> {
    fn default() -> Self {
        FontResolver {
            select_font: FontResolver::default_font_selector(),
            select_fallback: FontResolver::default_fallback_selector(),
        }
    }
}

impl FontResolver<'_> {
    /// Creates a default font selection resolver.
    ///
    /// The default implementation forwards to
    /// [`query`](fontdb::Database::query) on the font database specified in the
    /// [`Options`](crate::Options).
    pub fn default_font_selector() -> FontSelectionFn<'static> {
        Box::new(move |font, fontdb| {
            let mut name_list = Vec::new();
            for family in &font.families {
                name_list.push(match family {
                    FontFamily::Serif => fontdb::Family::Serif,
                    FontFamily::SansSerif => fontdb::Family::SansSerif,
                    FontFamily::Cursive => fontdb::Family::Cursive,
                    FontFamily::Fantasy => fontdb::Family::Fantasy,
                    FontFamily::Monospace => fontdb::Family::Monospace,
                    FontFamily::Named(s) => fontdb::Family::Name(s),
                });
            }

            // Use the default font as fallback.
            name_list.push(fontdb::Family::Serif);

            let stretch = match font.stretch {
                FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
                FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
                FontStretch::Condensed => fontdb::Stretch::Condensed,
                FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
                FontStretch::Normal => fontdb::Stretch::Normal,
                FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
                FontStretch::Expanded => fontdb::Stretch::Expanded,
                FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
                FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
            };

            let style = match font.style {
                FontStyle::Normal => fontdb::Style::Normal,
                FontStyle::Italic => fontdb::Style::Italic,
                FontStyle::Oblique => fontdb::Style::Oblique,
            };

            let query = fontdb::Query {
                families: &name_list,
                weight: fontdb::Weight(font.weight),
                stretch,
                style,
            };

            let id = fontdb.query(&query);
            if id.is_none() {
                log::warn!(
                    "No match for '{}' font-family.",
                    font.families
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            id
        })
    }

    /// Creates a default font fallback selection resolver.
    ///
    /// The default implementation searches through the entire `fontdb`
    /// to find a font that has the correct style and supports the character.
    pub fn default_fallback_selector() -> FallbackSelectionFn<'static> {
        Box::new(|c, exclude_fonts, fontdb| {
            let base_font_id = exclude_fonts[0];
            let base_face = fontdb.face(base_font_id)?;

            // Iterate over fonts and check if any of them support the specified char.
            for face in fontdb.faces() {
                // Ignore fonts, that were used for shaping already.
                if exclude_fonts.contains(&face.id) {
                    continue;
                }

                // Check that the new face has the same style.
                if base_face.style != face.style
                    && base_face.weight != face.weight
                    && base_face.stretch != face.stretch
                {
                    continue;
                }

                if !has_char(fontdb, face.id, c) {
                    continue;
                }

                let base_family = base_face
                    .families
                    .iter()
                    .find(|f| f.1 == fontdb::Language::English_UnitedStates)
                    .unwrap_or(&base_face.families[0]);

                let new_family = face
                    .families
                    .iter()
                    .find(|f| f.1 == fontdb::Language::English_UnitedStates)
                    .unwrap_or(&face.families[0]);

                log::warn!("Fallback from {} to {}.", base_family.0, new_family.0);
                return Some(face.id);
            }

            None
        })
    }
}

impl std::fmt::Debug for FontResolver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FontResolver { .. }")
    }
}

/// Convert a text into its paths. This is done in two steps:
/// 1. We convert the text into glyphs and position them according to the rules specified
///    in the SVG specification. While doing so, we also calculate the text bbox (which
///    is not based on the outlines of a glyph, but instead the glyph metrics as well
///    as decoration spans).
/// 2. We convert all of the positioned glyphs into outlines.
pub(crate) fn convert(text: &mut Text, resolver: &FontResolver, cache: &mut Cache) -> Option<()> {
    let (text_fragments, bbox) = layout::layout_text(text, resolver, &mut cache.fonts)?;
    text.layouted = text_fragments;
    text.bounding_box = bbox.to_rect();
    text.abs_bounding_box = bbox.transform(text.abs_transform)?.to_rect();

    let (group, stroke_bbox) = flatten::flatten(text, &mut cache.fonts)?;
    text.flattened = Box::new(group);
    text.stroke_bounding_box = stroke_bbox.to_rect();
    text.abs_stroke_bounding_box = stroke_bbox.transform(text.abs_transform)?.to_rect();

    Some(())
}

#[inline(never)]
fn has_char(fontdb: &Database, id: ID, c: char) -> bool {
    // Unlike for the main fonts, which are kept, here we just call `with_face_data` repeatedly so we don't need to call `to_vec`
    // for all fallback fonts.
    // TODO: Cache the charmaps of fallback candidates, building one for every lookup is expensive.
    fontdb
        .with_face_data(id, |data, face_index| {
            let face = ttf_parser::Face::parse(data, face_index).ok()?;
            face.glyph_index(c)
        })
        .flatten()
        .is_some()
}
