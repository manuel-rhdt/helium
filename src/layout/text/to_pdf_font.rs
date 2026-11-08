use super::font::Font;

use super::font::FontUnit;
use lemon_pdf::DocumentContext;
use lemon_pdf::font::Font as PdfFont;
use lemon_pdf::font::{
    FontType,
    descriptor::FontDescriptor,
    type0::{CIDFont, CIDFontType, CompositeFont, MetricsEntry},
};
use lemon_pdf::object::{IndirectReference, Object};
use lemon_pdf::stream::Stream;

use std::io::{Result, Write};

impl Font {
    pub fn is_truetype(&self) -> bool {
        self.hb_font.face().table_with_tag(b"glyf").is_some()
    }

    /// Subsets the font to the glyphs that were used in the document and
    /// returns the bytes of the resulting sfnt font file.
    ///
    /// Glyph ids are retained, because the content streams refer to glyphs by
    /// their original id (`Identity-H` encoding). Variable fonts are pinned to
    /// their default instance since PDF has no notion of variations and the
    /// shaper never applies any.
    fn subset_font_bytes(&self) -> std::result::Result<Vec<u8>, harfbuzz_rs::subset::SubsetError> {
        use harfbuzz_rs::subset::{SubsetFlags, SubsetInput};

        let face = self.hb_font.face();
        let mut input = SubsetInput::new()?;
        {
            let mut glyphs = input.glyph_set_mut();
            for glyph in self.glyph_advances.lock().keys() {
                glyphs.insert(glyph.0);
            }
        }
        // `Identity-H` text selects glyphs by CID, and a PDF viewer finds the glyph of a CID in a
        // CID-keyed CFF font (the Noto CJK fonts) through the charset of the font. The shaper
        // returns glyph indices, which are the same as the CIDs only if the charset is the
        // identity. A subset of the font (like the one bundled with Helium, whose glyphs are
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
            for tag in [b"GSUB", b"GPOS", b"GDEF", b"MATH"] {
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
    fn embedded_font_bytes(&self) -> Result<Vec<u8>> {
        self.subset_font_bytes().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("could not subset font {} for embedding: {}", self.postscript_name, err),
            )
        })
    }

    /// The six-letter tag that PDF requires in front of the name of a subset font
    /// (`ABCDEF+Name`). It is derived from the font name and the used glyphs, so
    /// different subsets of the same font get different names.
    fn subset_tag(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.postscript_name.hash(&mut hasher);
        for glyph in self.glyph_advances.lock().keys() {
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

    /// Builds the `FontFile2`/`FontFile3` stream from the font file bytes.
    fn pdf_font_stream(&self, font_file: &[u8]) -> Result<Stream> {
        let mut stream = Stream::with_bytes(vec![], None);
        if self.is_truetype() {
            // FontFile2: a complete TrueType sfnt.
            stream.bytes.write_all(font_file)?;
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
                    format!("font {} {}", self.postscript_name, reason),
                )
            })?;
            stream.bytes.write_all(&cff_data)?;
            stream.add_key_value("Subtype".to_owned(), Box::new("CIDFontType0C"));
        }
        Ok(stream)
    }

    pub fn to_font_unit(&self, val: f32) -> FontUnit {
        FontUnit((val * 1000.0 / self.upem as f32) as f64)
    }

    pub fn pdf_font_descriptor(
        &self,
        name: String,
        font_data: IndirectReference<Stream>,
    ) -> Result<FontDescriptor> {
        let face = self.hb_font.face();
        let invalid = |reason: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, reason.to_owned());
        // xMin, yMin, xMax, yMax of the `head` table
        let head = face.table_with_tag(b"head").ok_or_else(|| invalid("font has no head table"))?;
        let head: &[u8] = head.as_ref();
        if head.len() < 44 {
            return Err(invalid("the head table is truncated"));
        }
        let bbox = [36, 38, 40, 42].map(|offset| {
            self.to_font_unit(i16::from_be_bytes([head[offset], head[offset + 1]]) as f32)
        });
        let extents = self.hb_font.get_font_h_extents().unwrap_or_default();

        let mut font_descriptor = FontDescriptor {
            font_name: name.clone(),
            font_family: super::font::name_of(&face, harfbuzz_rs::sys::HB_OT_NAME_ID_FONT_FAMILY)
                .unwrap_or_else(|| self.postscript_name.clone())
                .into_bytes(),
            font_b_box: bbox,
            cap_height: self.to_font_unit(self.ot_metric(harfbuzz_rs::sys::HB_OT_METRICS_TAG_CAP_HEIGHT).unwrap_or(0) as f32),
            x_height: self.to_font_unit(self.ot_metric(harfbuzz_rs::sys::HB_OT_METRICS_TAG_X_HEIGHT).unwrap_or(0) as f32),
            ascent: self.to_font_unit(extents.ascender as f32),
            descent: self.to_font_unit(extents.descender as f32),
            ..Default::default()
        };

        if self.is_truetype() {
            font_descriptor.font_file2 = Some(font_data);
        } else {
            font_descriptor.font_file3 = Some(font_data);
        }

        Ok(font_descriptor)
    }

    pub fn embed_into_pdf(&self, pdf_context: &mut DocumentContext) -> Result<PdfFont> {
        let font_file = self.embedded_font_bytes()?;
        let name = format!("{}+{}", self.subset_tag(), self.postscript_name());
        let font_data = pdf_context.write_object(self.pdf_font_stream(&font_file)?)?;
        let fd = pdf_context.write_object(self.pdf_font_descriptor(name.clone(), font_data)?)?;

        let mut metrics_h = vec![];
        let mut unfinished_entry: Option<MetricsEntry> = None;
        for (&glyph, &advance) in &*self.glyph_advances.lock() {
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
        let metrics_h = pdf_context.write_object(metrics_h)?;

        let cid_font = CIDFont {
            subtype: if self.is_truetype() {
                CIDFontType::CIDFontType2
            } else {
                CIDFontType::CIDFontType0
            },
            base_font: name.clone(),
            font_descriptor: fd,
            dw: None,
            w: Some(Object::Indirect(metrics_h)),
            dw2: None,
            w2: None,
            cid_to_gid_map: if self.is_truetype() {
                "Identity".to_string()
            } else {
                String::new()
            },
            ..Default::default()
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

        let to_unicode = self.to_unicode.lock();

        // A single beginbfchar section may hold at most 100 entries.
        let entries: Vec<_> = to_unicode.iter().collect();
        for chunk in entries.chunks(100) {
            writeln!(to_unicode_stream, "{} beginbfchar", chunk.len())?;
            for (glyph, uni) in chunk {
                write!(to_unicode_stream, "<{:04X}>\t<", glyph.0 as u16)?;
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
        let to_unicode_stream = Stream::with_bytes(to_unicode_stream, None);
        let to_unicode_stream = pdf_context.write_object(to_unicode_stream)?;

        let base_name = if self.is_truetype() {
            name.clone()
        } else {
            format!("{}-Identity-H", &name)
        };

        let font = CompositeFont {
            subtype: FontType::Type0,
            base_font: base_name,
            encoding: "Identity-H".to_string(),
            descendant_fonts: [cid_font],
            to_unicode: Some(to_unicode_stream),
        };
        Ok(PdfFont::Composite(font))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::text::font::{GlyphIndex, system_font};
    use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};

    fn load(family: &str) -> Font {
        system_font(&[FontFamily::Named(family.into())], &FontWeight::Normal, &FontStyle::Normal)
            .unwrap()
    }

    /// Records that `text` is drawn with `font`, like the layout does.
    fn use_glyphs(font: &Font, text: &str) -> Vec<u16> {
        text.chars()
            .map(|ch| {
                let glyph = font.hb_font.get_nominal_glyph(ch).expect("the font has the glyph");
                font.glyph_advances.lock().insert(GlyphIndex(glyph), FontUnit(1000.0));
                glyph as u16
            })
            .collect()
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
        let glyphs = use_glyphs(&font, "道可名非常");
        // the premise: the CIDs of the bundled font are not its glyph indices
        let original = cids(&font.data, &glyphs);
        assert!(
            original.iter().zip(&glyphs).any(|(cid, glyph)| *cid != Some(*glyph)),
            "{original:?} {glyphs:?}"
        );
        // the embedded font has identity CIDs
        let embedded = font.subset_font_bytes().unwrap();
        let expected: Vec<_> = glyphs.iter().map(|g| Some(*g)).collect();
        assert_eq!(cids(&embedded, &glyphs), expected);
    }

    /// Fonts that are not CID-keyed are not changed in how they are numbered.
    #[test]
    fn other_fonts_keep_their_glyph_indices() {
        let font = load("TeX Gyre Termes");
        let glyphs = use_glyphs(&font, "abc");
        let embedded = font.subset_font_bytes().unwrap();
        let face = harfbuzz_rs::Face::from_bytes(&embedded, 0);
        let font = harfbuzz_rs::Font::new(face);
        for (ch, glyph) in "abc".chars().zip(glyphs) {
            assert_eq!(font.get_nominal_glyph(ch), Some(glyph as u32));
        }
    }
}
