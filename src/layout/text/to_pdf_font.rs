use super::font::{Font, FontData};

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
        input.set_flags(
            SubsetFlags::RETAIN_GIDS | SubsetFlags::NO_LAYOUT_CLOSURE | SubsetFlags::NOTDEF_OUTLINE,
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

    /// The font file to embed, and whether it is a subset.
    ///
    /// If subsetting fails the complete font is embedded instead.
    fn embedded_font_bytes(&self) -> (Vec<u8>, bool) {
        match self.subset_font_bytes() {
            Ok(bytes) => (bytes, true),
            Err(err) => {
                log::warn!(
                    "Could not subset font {}: {}. Embedding the complete font.",
                    self.postscript_name,
                    err
                );
                (self.hb_font.face().face_data().as_ref().to_vec(), false)
            }
        }
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
    fn pdf_font_stream(&self, font_file: &[u8]) -> Result<Option<Stream>> {
        match &self.font_data {
            FontData::External(_) => {
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
                Ok(Some(stream))
            }
            _ => Ok(None),
        }
    }

    pub fn to_font_unit(&self, val: f32) -> FontUnit {
        FontUnit((val * 1000.0 / self.upem as f32) as f64)
    }

    pub fn pdf_font_descriptor(
        &self,
        name: String,
        font_data: Option<IndirectReference<Stream>>,
    ) -> FontDescriptor {
        match &self.font_data {
            FontData::External(font) => {
                let font = font.load().unwrap();
                let bbox = font.metrics().bounding_box;
                let bbox = [
                    self.to_font_unit(bbox.min_x()),
                    self.to_font_unit(bbox.min_y()),
                    self.to_font_unit(bbox.max_x()),
                    self.to_font_unit(bbox.max_y()),
                ];

                let mut font_descriptor = FontDescriptor {
                    font_name: name.clone(),
                    font_family: font.family_name().as_bytes().to_owned(),
                    font_b_box: bbox,
                    cap_height: self.to_font_unit(font.metrics().cap_height),
                    x_height: self.to_font_unit(font.metrics().x_height),
                    ascent: self.to_font_unit(font.metrics().ascent),
                    descent: self.to_font_unit(font.metrics().descent),
                    ..Default::default()
                };

                if self.is_truetype() {
                    font_descriptor.font_file2 = font_data;
                } else {
                    font_descriptor.font_file3 = font_data;
                }

                font_descriptor
            }
            _ => todo!(),
        }
    }

    pub fn embed_into_pdf(&self, pdf_context: &mut DocumentContext) -> Result<PdfFont> {
        let (font_file, subsetted) = match &self.font_data {
            FontData::External(_) => self.embedded_font_bytes(),
            _ => (vec![], false),
        };
        let name = if subsetted {
            format!("{}+{}", self.subset_tag(), self.postscript_name())
        } else {
            self.postscript_name()
        };
        let font_data = self
            .pdf_font_stream(&font_file)?
            .map(|stream| pdf_context.write_object(stream))
            .transpose()?;
        let fd = pdf_context.write_object(self.pdf_font_descriptor(name.clone(), font_data))?;

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
                            Some(MetricsEntry::Range {
                                from: glyph.0 - 3,
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
