use crate::display_list::{self, DisplayItem, Rect};
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;

use lemon_pdf::font::builtin::BuiltInFont;
use lemon_pdf::pagetree::MediaBox;
use lemon_pdf::stream::StreamFilter;
use lemon_pdf::{Context, Page, Pt, Version};

use std::fs::File;
use std::io::{BufWriter, Error};

impl From<PixelLength> for Pt {
    fn from(px: PixelLength) -> Pt {
        Pt(px.0 as f64 * 0.75)
    }
}

pub fn write_pdf(
    display_list: crate::display_list::DisplayList,
    device: Device,
) -> Result<(), Error> {
    let file = BufWriter::new(File::create("test_working.pdf")?);
    let mut context = Context::with_writer(file, Version::Pdf1_7)?;

    let mut page = Page::new();
    page.set_media_box(MediaBox::new(
        Pt(0.0),
        Pt(0.0),
        device.width.into(),
        device.height.into(),
    ));
    page.add_content(&mut context, Some(StreamFilter::Deflate), |page_context| {
        let font = BuiltInFont::Helvetica.font(&mut page_context.pdf_context)?;
        let font_ref = page_context.pdf_context.write_object(font)?;
        let font_key = page_context.add_font(font_ref);

        for item in display_list.items() {
            match item {
                DisplayItem::Push => page_context.save_graphics_state()?,
                DisplayItem::Pop => page_context.restore_graphics_state()?,
                DisplayItem::Translate(x, y) => {
                    let x = *x;
                    let y = -*y;
                    page_context.concatenate_matrix(1.0, 0.0, 0.0, 1.0, x.into(), y.into())?;
                }
                &DisplayItem::Color(color) => {
                    let red = color.red as f64 / u8::max_value() as f64;
                    let green = color.green as f64 / u8::max_value() as f64;
                    let blue = color.blue as f64 / u8::max_value() as f64;
                    page_context.device_rgb_fill_color(red, green, blue)?;
                }
                &DisplayItem::LineString {
                    ref points,
                    line_width,
                } => {
                    let mut point_iter =
                        points
                            .into_iter()
                            .map(|&crate::display_list::Point { x, y }| {
                                let x = Pt::from(x);
                                let y = Pt::from(device.height - y);
                                (x, y)
                            });

                    if let Some(pt) = point_iter.next() {
                        page_context.move_to(pt.0, pt.1)?
                    } else {
                        continue;
                    }

                    for pt in point_iter {
                        page_context.line_to(pt.0, pt.1)?
                    }

                    page_context.line_width(line_width.into())?;
                    page_context.stroke_path()?;
                }
                DisplayItem::FilledRect(Rect {
                    x,
                    y,
                    width,
                    height,
                }) => {
                    let x = Pt::from(*x);
                    let y = Pt::from(device.height - *y - *height);
                    let width = Pt::from(*width);
                    let height = Pt::from(*height);

                    page_context.rect(x, y, width, height)?;
                    page_context.fill_path()?;
                }
                &DisplayItem::Text(display_list::Point { x, y }, font_size, ref text) => {
                    let x = x;
                    let y = device.height - y;
                    page_context.begin_text()?;
                    page_context.set_font(&font_key, font_size.into())?;
                    page_context.set_position(x.into(), y.into())?;
                    page_context.draw_simple_glyphs(&text)?;
                    page_context.end_text()?;
                }
            }
        }

        Ok(())
    })?;

    context.add_page(page);
    context.finish()?;

    Ok(())
}
