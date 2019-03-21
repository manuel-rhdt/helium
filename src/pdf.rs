use crate::display_list::{self, DisplayItem, Rect};
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;

use printpdf::{BuiltinFont, Error, Line, Mm, PdfDocument, Point, CurTransMat, Color, Rgb};

use font_kit::family_name::FamilyName;
use font_kit::handle::Handle;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

use std::fs::File;
use std::io::BufWriter;

const MM_PER_IN: f64 = 25.4;
const PT_PER_IN: f64 = 72.0;

impl From<PixelLength> for Mm {
    fn from(px: PixelLength) -> Mm {
        Mm(f64::from(px.0) * (0.75 * MM_PER_IN / PT_PER_IN))
    }
}

pub fn write_pdf(
    display_list: crate::display_list::DisplayList,
    device: Device,
) -> Result<(), Error> {
    let (doc, page1, layer1) = PdfDocument::new(
        "PDF_Document_title",
        device.width.into(),
        device.height.into(),
        "Layer 1",
    );
    let current_layer = doc.get_page(page1).get_layer(layer1);

    let font = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .unwrap();

    let _font = match font {
        Handle::Path { path, .. } => doc.add_external_font(File::open(path)?)?,
        _ => unimplemented!(),
    };

    let builtin_font = doc.add_builtin_font(BuiltinFont::Helvetica)?;

    current_layer.set_fill_color(Color::Rgb(Rgb::new(1.0, 0.0, 0.0, None)));

    for item in display_list.items() {
        match item {
            DisplayItem::PushTransform => current_layer.save_graphics_state(),
            DisplayItem::RestoreTransform => current_layer.restore_graphics_state(),
            DisplayItem::Translate(x, y) => {
                let x = *x;
                let y = -*y;
                current_layer.set_ctm(CurTransMat::Translate(x.into(), y.into()));
            },
            DisplayItem::FilledRect(Rect {
                x,
                y,
                width,
                height,
            }) => {
                let x = *x;
                let y = device.height - *y - *height;
                let width = *width;
                let height = *height;
                let p1 = Point::new(x.into(), y.into());
                let p2 = Point::new((x + width).into(), y.into());
                let p3 = Point::new((x + width).into(), (y + height).into());
                let p4 = Point::new(x.into(), (y + height).into());

                let mut line: Line = [p1, p2, p3, p4]
                    .into_iter()
                    .map(|&pt| (pt, false))
                    .collect();
                line.set_fill(true);
                line.set_closed(true);
                line.set_stroke(false);
                current_layer.add_shape(line);
            }
            DisplayItem::Text(display_list::Point { x, y }, _, text) => {
                let x = *x;
                let y = device.height - *y;
                let p = Point::new(x.into(), y.into());
                current_layer.begin_text_section();
                current_layer.set_font(&builtin_font, 12);
                current_layer.set_text_cursor(p.x.into(), p.y.into());
                current_layer.write_text(text.clone(), &builtin_font);
                current_layer.end_text_section();
            }
            _ => {}
        }
    }

    doc.save(&mut BufWriter::new(File::create("test_working.pdf")?))?;

    Ok(())
}
