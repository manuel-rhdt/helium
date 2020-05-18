use crate::display_list::{self, DisplayItem, PrintPages, Rect};
use crate::layout::components::TextOperator;
use crate::style::properties::computed::PixelLength;

use lemon_pdf::pagetree::MediaBox;
use lemon_pdf::stream::StreamFilter;
use lemon_pdf::{DocumentContext, Page, Pt, Version};

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Error};

impl From<PixelLength> for Pt {
    fn from(px: PixelLength) -> Pt {
        Pt(f64::from(px.0) * 0.75)
    }
}

pub fn write_pdf(print_pages: PrintPages, no_filter: bool) -> Result<(), Error> {
    let file = BufWriter::new(File::create("test_working.pdf")?);
    let mut context = DocumentContext::with_writer(file, Version::Pdf1_7)?;

    let mut font_map = HashMap::new();
    for (font_name, font) in &print_pages.font_map {
        // if !font.glyph_advances.lock().is_empty() {
            log::info!("Embedding font {} into PDF file.", font_name);
            let font = font.embed_into_pdf(&mut context)?;
            let font = context.write_object(font)?;
            font_map.insert(font_name.clone(), font);
        // }
    }

    log::info!("Write {} pages.", print_pages.pages.len());

    for mut print_page in print_pages.pages {
        let mut page = Page::new();
        page.set_media_box(MediaBox::new(
            Pt(0.0),
            Pt(0.0),
            print_page.width.into(),
            print_page.height.into(),
        ));

        let page_height = print_page.height;

        let filter = if no_filter {
            None
        } else {
            Some(StreamFilter::Deflate)
        };

        page.add_content(&mut context, filter, |page_context| {
            for item in print_page.display_list.items() {
                match item {
                    DisplayItem::Push => page_context.save_graphics_state()?,
                    DisplayItem::Pop => page_context.restore_graphics_state()?,
                    DisplayItem::Translate(x, y) => {
                        let x = *x;
                        let y = -*y;
                        page_context.concatenate_matrix(1.0, 0.0, 0.0, 1.0, x.into(), y.into())?;
                    }
                    &DisplayItem::Color(color) => {
                        let floatize = |val: u8| f64::from(val) / f64::from(u8::max_value());
                        let red = floatize(color.red);
                        let green = floatize(color.green);
                        let blue = floatize(color.blue);
                        page_context.device_rgb_fill_color(red, green, blue)?;
                    }
                    &DisplayItem::StrokeColor(color) => {
                        let floatize = |val: u8| f64::from(val) / f64::from(u8::max_value());
                        let red = floatize(color.red);
                        let green = floatize(color.green);
                        let blue = floatize(color.blue);
                        page_context.device_rgb_stroke_color(red, green, blue)?;
                    }
                    &DisplayItem::LineString {
                        ref points,
                        line_width,
                    } => {
                        let mut point_iter =
                            points.iter().map(|&crate::display_list::Point { x, y }| {
                                let x = Pt::from(x);
                                let y = Pt::from(page_height - y);
                                (x, y)
                            });

                        if let Some((x, y)) = point_iter.next() {
                            page_context.move_to(x, y)?
                        } else {
                            continue;
                        }

                        for (x, y) in point_iter {
                            page_context.line_to(x, y)?
                        }

                        page_context.line_width(line_width.into())?;
                        page_context.stroke_path()?;
                    }
                    &DisplayItem::FilledRect(Rect {
                        x,
                        y,
                        width,
                        height,
                    }) => {
                        let x = Pt::from(x);
                        let y = Pt::from(page_height - y - height);
                        let width = Pt::from(width);
                        let height = Pt::from(height);

                        page_context.rect(x, y, width, height)?;
                        page_context.fill_path()?;
                    }
                    &DisplayItem::Text(
                        display_list::Point { x, y },
                        font_size,
                        ref font_name,
                        ref glyphs,
                    ) => {
                        let x = x;
                        let y = page_height - y;
                        page_context.begin_text()?;
                        let font_ref = font_map[font_name];
                        let key = page_context.add_font(font_ref);
                        page_context.set_font(&key, font_size.into())?;
                        page_context.set_position(x.into(), y.into())?;
                        page_context.push_operand(glyphs)?;
                        page_context.apply_operator("Tj")?;
                        page_context.end_text()?;
                    }
                    DisplayItem::Paragraph(par) => {
                        let x = par.start_pos.0;
                        let y = page_height - par.start_pos.1;

                        page_context.begin_text()?;
                        page_context.set_position(x.into(), y.into())?;

                        for op in &par.ops {
                            match op {
                                TextOperator::SetFont(font_name, size) => {
                                    let font_ref = font_map[font_name];
                                    let key = page_context.add_font(font_ref);
                                    page_context.set_font(&key, (*size).into())?;
                                }
                                TextOperator::Glyphs(glyphs) => {
                                    page_context.push_operand(glyphs)?;
                                    page_context.apply_operator("Tj")?;
                                }
                                TextOperator::KernedGlyphs(glyphs) => {
                                    page_context.push_operand(glyphs)?;
                                    page_context.apply_operator("TJ")?;
                                }
                                &TextOperator::NewLine { x_offset, leading } => {
                                    page_context
                                        .set_position(x_offset.into(), (-leading).into())?;
                                }
                                &TextOperator::SetWordSpacing(spacing) => {
                                    page_context.push_operand(Pt::from(spacing))?;
                                    page_context.apply_operator("Tw")?;
                                }
                                &TextOperator::SetTransform(matrix) => {
                                    let matrix: [f32; 6] = [
                                        matrix.a,
                                        matrix.b,
                                        matrix.c,
                                        matrix.d,
                                        Pt::from(matrix.e).0 as f32,
                                        Pt::from(page_height - matrix.f).0 as f32,
                                    ];
                                    for elem in &matrix {
                                        page_context.push_operand(*elem)?;
                                    }
                                    page_context.apply_operator("Tm")?;
                                }
                            }
                        }

                        page_context.end_text()?;
                    }
                }
            }

            Ok(())
        })?;

        context.add_page(page);
    }
    context.finish()?;

    Ok(())
}

// fn add_font(
//     context: &mut PageContext<impl std::io::Write>,
//     font: &HFont,
// ) -> Result<(String, Font), std::io::Error> {
//     let mut pdf_font = match font {
//         HFont::BuiltIn(font) => font.font(&mut context.pdf_context)?,
//     };
//     pdf_font.encoding = Some(EncodingEntry::Predefined(
//         PredefinedEncoding::WinAnsiEncoding,
//     ));

//     pdf_font.widths = WIN_ANSI_ENCODING
//         .iter()
//         .filter(|entry| !entry.is_empty())
//         .map(|&entry| font);

//     let font_ref = context
//         .pdf_context
//         .write_object(Font::Simple(pdf_font.clone()))?;

//     Ok((context.add_font(font_ref), Font::Simple(pdf_font)))
// }

// fn encode_glyphs_simple(glyphs: &[GlyphIndex], font: &Font) -> Vec<u8> {
//     let Font::Simple(font) = font;
//     let encoding = font.encoding.as_ref().unwrap();
//     let metrics = BuiltInFont::Helvetica.metrics();
//     glyphs
//         .iter()
//         .flat_map(|&glyph| {
//             let char_name = metrics.char_metrics[glyph.0 as usize].name;
//             encoding.encode(char_name)
//         })
//         .collect()
// }
