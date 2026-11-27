use crate::font_usage::GlyphUsage;
use crate::pdf::encoding::GlyphEncoding;
use crate::layout::text::font::{Font, FontUnit, GlyphIndex};
use std::collections::BTreeMap;
use lemon_pdf::DocumentContext;
use lemon_pdf::font::Font as PdfFont;
use lemon_pdf::font::{
    FontType, ToUnicodeCMapDict,
    descriptor::{
        FontDescriptor, FontFile3, FontFile3Dict, FontFile3Subtype, TrueTypeFontFile,
        TrueTypeFontFileDict,
    },
    type0::{CIDFont, CIDFontType, CompositeFont, MetricsEntry, Widths},
};
use lemon_pdf::object::{ByteString, IndirectReference, Name, Object};
use lemon_pdf::stream::Stream;

use std::io::{Result, Write};

/// The embedded font program of a CIDFont.
enum FontProgram {
    /// A complete TrueType sfnt (`FontFile2`).
    TrueType(TrueTypeFontFile),
    /// A bare CFF table (`FontFile3` with subtype `CIDFontType0C`).
    Cff(FontFile3),
}

/// A font that is embedded into the document: what the content streams need to show its glyphs.
pub(crate) struct EmbeddedFont {
    pub reference: IndirectReference<PdfFont>,
    pub encoding: GlyphEncoding,
}

/// A reference to the embedded font program of a CIDFont.
enum FontProgramRef {
    TrueType(IndirectReference<TrueTypeFontFile>),
    Cff(IndirectReference<FontFile3>),
}

/// A stream of `bytes`, deflated if `compress`.
fn stream<D>(dict: D, bytes: &[u8], compress: bool) -> std::io::Result<Stream<D>> {
    if compress {
        Stream::deflated(dict, bytes)
    } else {
        Ok(Stream::new(dict, bytes.to_vec()))
    }
}

/// Subsets the font to the glyphs that were used in the document and
/// returns the bytes of the resulting sfnt font file.
///
/// Glyph ids are retained, because the content streams refer to glyphs by
/// their original id (`Identity-H` encoding). Variable fonts are pinned to
/// their default instance since PDF has no notion of variations and the
/// shaper never applies any.
fn subset_font_bytes(font: &Font, usage: &GlyphUsage) -> std::result::Result<Vec<u8>, harfbuzz_rs::subset::SubsetError> {
    use harfbuzz_rs::subset::{SubsetFlags, SubsetInput};

    let face = harfbuzz_rs::Face::from_bytes(&font.data, font.index);
    let mut input = SubsetInput::new()?;
    {
        let mut glyphs = input.glyph_set_mut();
        for glyph in usage.glyphs() {
            glyphs.insert(glyph.0);
        }
    }
    // `Identity-H` text selects glyphs by CID, and a PDF viewer finds the glyph of a CID in a
    // CID-keyed CFF font (the Noto CJK fonts) through the charset of the font. The shaper
    // returns glyph indices, which are the same as the CIDs only if the charset is the
    // identity. A subset of the font (like the one bundled with Umbruch, whose glyphs are
    // numbered anew) keeps the CIDs of the original, so the charset is renumbered:
    // together with the retained glyph indices CID = glyph index. This has no effect on
    // other fonts.
    input.set_flags(
        SubsetFlags::RETAIN_GIDS
            | SubsetFlags::CFF_IDENTITY_CHARSET
            | SubsetFlags::NO_LAYOUT_CLOSURE
            | SubsetFlags::NOTDEF_OUTLINE,
    );
    {
        // Layout tables are not needed in a PDF.
        let mut drop = input.drop_table_set_mut();
        for tag in [b"GSUB", b"GPOS", b"GDEF", b"MATH", b"COLR", b"CPAL", b"SVG "] {
            drop.insert(harfbuzz_rs::Tag::from(tag));
        }
    }
    if face.table_with_tag(b"fvar").is_some() {
        input.pin_all_axes_to_default(&face)?;
    }
    let subset = face.subset(&input)?;
    let bytes = subset.face_data().as_ref().to_vec();
    Ok(bytes)
}

/// The font file to embed: the subset of the font to the glyphs that are used.
///
/// Failing to subset is an error rather than a reason to embed the complete font. The
/// complete font would work for most fonts, but not for a CID-keyed CFF font whose charset is
/// not the identity (the glyph indices in the text would select the wrong glyphs, see
/// `subset_font_bytes`), and a font that cannot be subsetted is not in a state to trust.
fn embedded_font_bytes(font: &Font, usage: &GlyphUsage) -> Result<Vec<u8>> {
    subset_font_bytes(font, usage).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not subset font {} for embedding: {}", font.postscript_name, err),
        )
    })
}

/// The six-letter tag that PDF requires in front of the name of a subset font
/// (`ABCDEF+Name`). It is derived from the font name and the used glyphs, so
/// different subsets of the same font get different names.
fn subset_tag(font: &Font, usage: &GlyphUsage) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    font.postscript_name.hash(&mut hasher);
    for glyph in usage.glyphs() {
        glyph.hash(&mut hasher);
    }
    let mut hash = hasher.finish();
    (0..6)
        .map(|_| {
            let c = (b'A' + (hash % 26) as u8) as char;
            hash /= 26;
            c
        })
        .collect()
}

/// Builds the `FontFile2`/`FontFile3` stream from the font file bytes, deflated if `compress`.
fn pdf_font_stream(font: &Font, font_file: &[u8], compress: bool) -> Result<FontProgram> {
    if font.is_truetype() {
        // FontFile2: a complete TrueType sfnt.
        let dict = TrueTypeFontFileDict {
            length1: font_file.len(),
        };
        Ok(FontProgram::TrueType(stream(dict, font_file, compress)?))
    } else {
        // FontFile3 / CIDFontType0C: the bare CFF table.
        let face = harfbuzz_rs::Face::from_bytes(font_file, 0);
        let cff_data = face.table_with_tag(b"CFF ").ok_or_else(|| {
            let reason = if face.table_with_tag(b"CFF2").is_some() {
                "uses CFF2 outlines, which PDF cannot embed"
            } else {
                "has neither a glyf nor a CFF table"
            };
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("font {} {}", font.postscript_name, reason),
            )
        })?;
        let dict = FontFile3Dict {
            subtype: FontFile3Subtype::CIDFontType0C,
        };
        Ok(FontProgram::Cff(stream(dict, &cff_data, compress)?))
    }
}

fn to_font_unit(font: &Font, val: f32) -> FontUnit {
    FontUnit((val * 1000.0 / font.upem as f32) as f64)
}

fn pdf_font_descriptor(font: &Font, name: Name, font_data: FontProgramRef) -> Result<FontDescriptor> {
    let invalid = |reason: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, reason.to_owned());
    // xMin, yMin, xMax, yMax of the `head` table
    let head = font.table_data(b"head").ok_or_else(|| invalid("font has no head table"))?;
    if head.len() < 44 {
        return Err(invalid("the head table is truncated"));
    }
    let bbox = [36, 38, 40, 42].map(|offset| {
        to_font_unit(font, i16::from_be_bytes([head[offset], head[offset + 1]]) as f32)
    });
    let (ascender, descender) = font.line_extents();

    let mut font_descriptor = FontDescriptor {
        font_family: Some(ByteString::from(
            crate::layout::text::font::name_of(&font.face, read_fonts::types::NameId::FAMILY_NAME)
                .unwrap_or_else(|| font.postscript_name.clone())
                .into_bytes(),
        )),
        font_b_box: bbox,
        cap_height: to_font_unit(font, font.cap_height().unwrap_or(0) as f32),
        x_height: to_font_unit(font, font.x_height().unwrap_or(0) as f32),
        ascent: to_font_unit(font, ascender as f32),
        descent: to_font_unit(font, descender as f32),
        ..FontDescriptor::new(name)
    };

    match font_data {
        FontProgramRef::TrueType(font_data) => font_descriptor.font_file2 = Some(font_data),
        FontProgramRef::Cff(font_data) => font_descriptor.font_file3 = Some(font_data),
    }

    Ok(font_descriptor)
}

/// The text of every glyph that is used, for the `ToUnicode` map: PDF/UA wants all of them
/// mapped (ISO 14289-1 7.21.7). A glyph that no text was recorded for gets the character
/// that the font maps to it, else the text of a cluster it was drawn in, else U+FFFD.
fn complete_to_unicode(font: &Font, usage: &GlyphUsage) -> BTreeMap<GlyphIndex, String> {
    let mut map: BTreeMap<GlyphIndex, String> =
        usage.glyphs().filter_map(|glyph| Some((glyph, usage.text(glyph)?.to_owned()))).collect();
    let mut missing: std::collections::BTreeSet<GlyphIndex> =
        usage.glyphs().filter(|glyph| !map.contains_key(glyph)).collect();
    if missing.is_empty() {
        return map;
    }
    if let Ok(face) = ttf_parser::Face::parse(&font.data, font.index) {
        if let Some(cmap) = face.tables().cmap {
            for subtable in cmap.subtables.into_iter().filter(|s| s.is_unicode()) {
                subtable.codepoints(|codepoint| {
                    let Some(glyph) = subtable.glyph_index(codepoint) else { return };
                    let glyph = GlyphIndex(glyph.0 as u32);
                    if missing.contains(&glyph) {
                        // the lowest code point wins, so the result does not depend on
                        // the order of the subtables
                        if let Some(ch) = char::from_u32(codepoint) {
                            let entry = map.entry(glyph).or_insert_with(|| ch.to_string());
                            if entry.chars().next().is_some_and(|old| ch < old) {
                                *entry = ch.to_string();
                            }
                        }
                    }
                });
            }
        }
    }
    missing.retain(|glyph| !map.contains_key(glyph));
    for glyph in missing {
        let text = usage.fallback_text(glyph).unwrap_or("\u{FFFD}").to_owned();
        map.insert(glyph, text);
    }
    map
}

/// Writes the font into the document. The font program and the `ToUnicode` map are deflated
/// if `compress`: every viewer reads Flate, and the font programs are most of a document
/// that has text in many scripts.
pub(crate) fn embed_into_pdf(
    font: &Font,
    usage: &GlyphUsage,
    pdf_context: &mut DocumentContext,
    compress: bool,
) -> Result<EmbeddedFont> {
    let encoding = GlyphEncoding::choose(font, usage);
    let font_file = embedded_font_bytes(font, usage)?;
    let name = format!("{}+{}", subset_tag(font, usage), font.postscript_name());
    let font_data = match pdf_font_stream(font, &font_file, compress)? {
        FontProgram::TrueType(stream) => FontProgramRef::TrueType(pdf_context.write_object(stream)?),
        FontProgram::Cff(stream) => FontProgramRef::Cff(pdf_context.write_object(stream)?),
    };
    let mut descriptor = pdf_font_descriptor(font, Name::from(name.clone()), font_data)?;
    if super::pdfa1_active() {
        // ISO 19005-1, 6.3.5: the CIDs of a font subset, which are the glyphs that are drawn (and the
        // glyph 0, `.notdef`, that every font program has)
        let cids = std::iter::once(0).chain(usage.glyphs().map(|glyph| encoding.code(glyph)));
        let bytes = lemon_pdf::font::descriptor::cid_set(cids);
        descriptor.cid_set = Some(pdf_context.write_object(stream(lemon_pdf::font::descriptor::CidSetDict {}, &bytes, compress)?)?);
    }
    let fd = pdf_context.write_object(descriptor)?;

    let mut metrics_h = vec![];
    let mut unfinished_entry: Option<MetricsEntry> = None;
    for glyph in usage.glyphs() {
        let advance = font.advance(glyph);
        unfinished_entry = match unfinished_entry.take() {
            Some(MetricsEntry::Range {
                from,
                to,
                advance: previous_advance,
            }) => {
                if advance == previous_advance {
                    Some(MetricsEntry::Range {
                        from,
                        to: glyph.0,
                        advance,
                    })
                } else {
                    metrics_h.push(MetricsEntry::Range {
                        from,
                        to,
                        advance: previous_advance,
                    });
                    Some(MetricsEntry::Consecutive {
                        start: glyph.0,
                        advances: vec![advance],
                    })
                }
            }
            Some(MetricsEntry::Consecutive {
                start,
                mut advances,
            }) => {
                // the glyph indices are not consecutive anymore
                if start as usize + advances.len() != glyph.0 as usize {
                    metrics_h.push(MetricsEntry::Consecutive { start, advances });
                    Some(MetricsEntry::Consecutive {
                        start: glyph.0,
                        advances: vec![advance],
                    })
                } else {
                    // check if last 2 elements are the same as the current advance
                    if advances.len() >= 2 && advances[advances.len() - 2..] == [advance; 2] {
                        advances.truncate(advances.len() - 2);
                        if !advances.is_empty() {
                            metrics_h.push(MetricsEntry::Consecutive { start, advances })
                        }
                        // The run described here is the two dropped entries plus the
                        // current glyph, so it starts at `glyph.0 - 2`. Starting at
                        // `glyph.0 - 3` swallowed the glyph before the run, whose
                        // advance is already in the entry pushed above: the range wins,
                        // and every glyph after it is drawn at the wrong x.
                        Some(MetricsEntry::Range {
                            from: glyph.0 - 2,
                            to: glyph.0,
                            advance,
                        })
                    } else {
                        advances.push(advance);
                        Some(MetricsEntry::Consecutive { start, advances })
                    }
                }
            }
            None => Some(MetricsEntry::Consecutive {
                start: glyph.0,
                advances: vec![advance],
            }),
        }
    }
    if let Some(entry) = unfinished_entry {
        metrics_h.push(entry);
    }
    let metrics_h = pdf_context.write_object(Widths(metrics_h))?;

    let cid_font = CIDFont {
        subtype: if font.is_truetype() {
            CIDFontType::CIDFontType2
        } else {
            CIDFontType::CIDFontType0
        },
        base_font: Name::from(name.clone()),
        cid_system_info: Default::default(),
        font_descriptor: fd,
        dw: None,
        w: Some(Object::Indirect(metrics_h)),
        dw2: None,
        w2: None,
        cid_to_gid_map: if font.is_truetype() {
            Some(Name::from("Identity"))
        } else {
            None
        },
    };
    let cid_font = pdf_context.write_object(cid_font)?;

    let mut to_unicode_stream = format!(
        r"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (FontSpecific) /Ordering ({0}) /Supplement 0 >> def
/CMapName /FontSpecific-{0} def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
",
        name
    )
    .into_bytes();

    let to_unicode = complete_to_unicode(font, usage);

    // A single beginbfchar section may hold at most 100 entries.
    let entries: Vec<_> = to_unicode.iter().collect();
    for chunk in entries.chunks(100) {
        writeln!(to_unicode_stream, "{} beginbfchar", chunk.len())?;
        for (glyph, uni) in chunk {
            write!(to_unicode_stream, "<{:04X}>\t<", encoding.code(**glyph))?;
            for chr in uni.encode_utf16() {
                write!(to_unicode_stream, "{:04X}", chr)?;
            }
            writeln!(to_unicode_stream, ">")?;
        }
        writeln!(to_unicode_stream, "endbfchar")?;
    }
    writeln!(
        to_unicode_stream,
        r"endcmap
CMapName currentdict /CMap defineresource pop
end
end"
    )?;
    let to_unicode_stream = stream(ToUnicodeCMapDict {}, &to_unicode_stream, compress)?;
    let to_unicode_stream = pdf_context.write_object(to_unicode_stream)?;

    let base_name = if font.is_truetype() {
        name.clone()
    } else {
        format!("{}-{}", &name, encoding.cmap_name())
    };

    let font = CompositeFont {
        subtype: FontType::Type0,
        base_font: Name::from(base_name),
        encoding: Name::from(encoding.cmap_name()),
        descendant_fonts: [cid_font],
        to_unicode: Some(to_unicode_stream),
    };
    let reference = pdf_context.write_object(PdfFont::Composite(font))?;
    Ok(EmbeddedFont { reference, encoding })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_usage::FontUsage;
    use crate::layout::text::font::{GlyphIndex, system_font};
    use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};

    fn load(family: &str) -> Font {
        system_font(&[FontFamily::Named(family.into())], &FontWeight::NORMAL, &FontStyle::Normal)
            .unwrap()
    }

    /// The usage of `font` that draws `text`, and the glyphs of the text.
    fn use_glyphs(font: &Font, text: &str) -> (FontUsage, Vec<u16>) {
        let mut usage = FontUsage::default();
        let glyphs = text
            .chars()
            .map(|ch| {
                let glyph = font.nominal_glyph(ch).expect("the font has the glyph");
                usage.record_glyph(&font.postscript_name, GlyphIndex(glyph), &ch.to_string());
                glyph as u16
            })
            .collect();
        (usage, glyphs)
    }

    fn cids(font_bytes: &[u8], glyphs: &[u16]) -> Vec<Option<u16>> {
        let face = ttf_parser::Face::parse(font_bytes, 0).unwrap();
        let cff = face.tables().cff.as_ref().expect("a CFF font");
        glyphs.iter().map(|g| cff.glyph_cid(ttf_parser::GlyphId(*g))).collect()
    }

    /// The text of the PDF contains glyph indices, so in the embedded font, whose glyphs are
    /// found by CID, the CID of every glyph must be its glyph index. The bundled CJK font is a
    /// CID-keyed CFF whose glyphs were renumbered by the subsetting in `build.rs`, so
    /// the CIDs differ from the glyph indices in the font that is embedded.
    #[test]
    fn cid_keyed_cff_fonts_are_embedded_with_cids_equal_to_glyph_indices() {
        let font = load("Noto Sans CJK JP");
        let (usage, glyphs) = use_glyphs(&font, "道可名非常");
        // the premise: the CIDs of the bundled font are not its glyph indices
        let original = cids(&font.data, &glyphs);
        assert!(
            original.iter().zip(&glyphs).any(|(cid, glyph)| *cid != Some(*glyph)),
            "{original:?} {glyphs:?}"
        );
        // the embedded font has identity CIDs
        let embedded = subset_font_bytes(&font, usage.font(&font.postscript_name).unwrap()).unwrap();
        let expected: Vec<_> = glyphs.iter().map(|g| Some(*g)).collect();
        assert_eq!(cids(&embedded, &glyphs), expected);
    }

    /// Fonts that are not CID-keyed are not changed in how they are numbered.
    #[test]
    fn other_fonts_keep_their_glyph_indices() {
        let font = load("TeX Gyre Termes");
        let (usage, glyphs) = use_glyphs(&font, "abc");
        let embedded = subset_font_bytes(&font, usage.font(&font.postscript_name).unwrap()).unwrap();
        let face = harfbuzz_rs::Face::from_bytes(&embedded, 0);
        let font = harfbuzz_rs::Font::new(face);
        for (ch, glyph) in "abc".chars().zip(glyphs) {
            assert_eq!(font.get_nominal_glyph(ch), Some(glyph as u32));
        }
    }
}
