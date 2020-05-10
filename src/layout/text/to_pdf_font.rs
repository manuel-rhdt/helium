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

use font_parse::Font as ParsedFont;

use std::io::{Read, Result};

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
                // lot of copying here :(
                let handle = font.handle().unwrap();
                let mut font_bytes = vec![];
                match handle {
                    font_kit::handle::Handle::Path { path, font_index } => {
                        let mut font_data = vec![];
                        std::fs::File::open(path)?.read_to_end(&mut font_data)?;
                        let pf = ParsedFont::from_bytes(&font_data, font_index).unwrap();
                        pf.write_to(&mut font_bytes)?;
                    }
                    font_kit::handle::Handle::Memory { bytes, font_index } => {
                        let pf = ParsedFont::from_bytes(&bytes, font_index).unwrap();
                        pf.write_to(&mut font_bytes)?;
                    }
                }

                let mut stream = Stream::with_bytes(Vec::clone(&font_bytes), None);

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

                let mut fd = FontDescriptor {
                    font_name: self.postscript_name().clone(),
                    font_family: font.family_name().as_bytes().to_owned(),
                    font_b_box: bbox,
                    cap_height: self.to_font_unit(font.metrics().cap_height),
                    x_height: self.to_font_unit(font.metrics().x_height),
                    ascent: self.to_font_unit(font.metrics().ascent),
                    descent: self.to_font_unit(font.metrics().descent),
                    ..Default::default()
                };

                fd.font_file3 = font_data;

                fd
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
        for (&glyph, &advance) in &*self.glyph_advances.lock().unwrap() {
            let advance = self.to_font_unit(advance);
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
            base_font: self.postscript_name().clone(),
            font_descriptor: fd,
            dw: None,
            w: Some(Object::Indirect(metrics_h)),
            dw2: None,
            w2: None,
            cid_to_gid_map: "Identity".to_string(),
            ..Default::default()
        };
        let cid_font = pdf_context.write_object(cid_font)?;
        let font = CompositeFont {
            subtype: FontType::Type0,
            base_font: self.postscript_name().clone(),
            encoding: "Identity-H".to_string(),
            descendant_fonts: [cid_font],
            to_unicode: None,
        };
        Ok(PdfFont::Composite(font))
    }
}
