use super::font::{Font, FontData};

use super::font::FontUnit;
use lemon_pdf::font::Font as PdfFont;
use lemon_pdf::font::{
    descriptor::FontDescriptor,
    type0::{CIDFont, CIDFontType, CompositeFont, MetricsEntry},
    FontType,
};
use lemon_pdf::object::{IndirectReference, Object};
use lemon_pdf::stream::Stream;
use lemon_pdf::DocumentContext;

use font_parse::{write_font, FontKitFont, OpentypeTableAccess, Tag};

use std::io::{Result, Write};

impl Font {
    pub fn is_truetype(&self) -> bool {
        match &self.font_data {
            FontData::External(font) => {
                font.load_font_table(u32::from_be_bytes(*b"glyf")).is_some()
            }
            _ => false,
        }
    }

    pub fn pdf_font_stream(&self) -> Result<Option<Stream>> {
        match &self.font_data {
            FontData::External(font) => {
                let wrapped_font = FontKitFont::new(font);
                let mut font_bytes = vec![];

                if wrapped_font.has_table(Tag(*b"glyf")) {
                    let tables = vec![
                        Tag(*b"head"),
                        Tag(*b"hhea"),
                        Tag(*b"loca"),
                        Tag(*b"maxp"),
                        Tag(*b"cvt "),
                        Tag(*b"prep"),
                        Tag(*b"glyf"),
                        Tag(*b"hmtx"),
                        Tag(*b"fpgm"),
                    ];
                    write_font(&wrapped_font, Tag([0, 1, 0, 0]), &tables, &mut font_bytes)?;
                } else {
                    let tables = &[
                        Tag(*b"head"),
                        Tag(*b"CFF "),
                        Tag(*b"hhea"),
                        Tag(*b"hmtx"),
                        Tag(*b"maxp"),
                    ];
                    write_font(&wrapped_font, Tag([0, 1, 0, 0]), tables, &mut font_bytes)?;
                }

                let mut stream = Stream::with_bytes(font_bytes, None);
                stream.add_key_value("Subtype".to_owned(), Box::new("OpenType"));

                Ok(Some(stream))
            }
            _ => Ok(None),
        }
    }

    pub fn to_font_unit(&self, val: f32) -> FontUnit {
        FontUnit((val * 1000.0 / self.upem() as f32) as f64)
    }

    pub fn pdf_font_descriptor(
        &self,
        font_data: Option<IndirectReference<Stream>>,
    ) -> FontDescriptor {
        match &self.font_data {
            FontData::External(font) => {
                let bbox = font.metrics().bounding_box;
                let bbox = [
                    self.to_font_unit(bbox.min_x()),
                    self.to_font_unit(bbox.min_y()),
                    self.to_font_unit(bbox.max_x()),
                    self.to_font_unit(bbox.max_y()),
                ];

                FontDescriptor {
                    font_name: self.postscript_name().clone(),
                    font_family: font.family_name().as_bytes().to_owned(),
                    font_b_box: bbox,
                    cap_height: self.to_font_unit(font.metrics().cap_height),
                    x_height: self.to_font_unit(font.metrics().x_height),
                    ascent: self.to_font_unit(font.metrics().ascent),
                    descent: self.to_font_unit(font.metrics().descent),
                    font_file3: font_data,
                    ..Default::default()
                }
            }
            _ => todo!(),
        }
    }

    pub fn upem(&self) -> u32 {
        match &self.font_data {
            FontData::BuiltIn(_) => 1000,
            FontData::External(font) => font.metrics().units_per_em,
        }
    }

    pub fn embed_into_pdf(&self, pdf_context: &mut DocumentContext) -> Result<PdfFont> {
        let font_data = self
            .pdf_font_stream()?
            .map(|stream| pdf_context.write_object(stream))
            .transpose()?;
        let fd = pdf_context.write_object(self.pdf_font_descriptor(font_data))?;

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
            base_font: self.postscript_name(),
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
            self.postscript_name()
        )
        .into_bytes();

        let to_unicode = self.to_unicode.lock();
        let to_unicode_len = to_unicode.len();

        writeln!(to_unicode_stream, "{} beginbfchar", to_unicode_len)?;
        for (glyph, uni) in to_unicode.iter() {
            write!(to_unicode_stream, "<{:04X}>\t<", glyph.0 as u16)?;
            for chr in uni.chars() {
                write!(to_unicode_stream, "{:04X}", chr as u32)?;
            }
            writeln!(to_unicode_stream, ">")?;
        }
        writeln!(to_unicode_stream, "endbfchar")?;
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
            self.postscript_name()
        } else {
            format!("{}-Identity-H", &self.postscript_name())
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
