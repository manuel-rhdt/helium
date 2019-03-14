use crate::display_list::{DisplayItem, Rect};
use crate::style::device::Device;
use crate::style::properties::computed::CssFloat;

use printpdf::{Error, Line, Mm, PdfDocument, Point};

use font_kit::family_name::FamilyName;
use font_kit::handle::Handle;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

use std::fs::File;
use std::io::BufWriter;

const MM_PER_IN: f64 = 25.4;
const PT_PER_IN: f64 = 72.0;

fn px_to_mm(px: CssFloat) -> Mm {
    Mm(f64::from(px) * 0.75 * MM_PER_IN / PT_PER_IN)
}

pub fn write_pdf(display_list: crate::display_list::DisplayList, device: Device) -> Result<(), Error> {
    let (doc, page1, layer1) =
        PdfDocument::new("PDF_Document_title", px_to_mm(device.width), px_to_mm(device.height), "Layer 1");
    let current_layer = doc.get_page(page1).get_layer(layer1);

    let font = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .unwrap();

    let font = match font {
        Handle::Path { path, .. } => doc.add_external_font(File::open(path)?)?,
        _ => unimplemented!(),
    };

    current_layer.begin_text_section();

    current_layer.set_font(&font, 33);
    current_layer.set_text_cursor(Mm(10.0), Mm(10.0));
    current_layer.set_line_height(33);

    current_layer.write_text("Hello Wordl!".clone(), &font);

    current_layer.end_text_section();

    for item in display_list.items() {
        match *item {
            DisplayItem::FilledRect(Rect {
                x,
                y,
                width,
                height,
            }) => {
                let p1 = Point::new(px_to_mm(x), px_to_mm(y));
                let p2 = Point::new(px_to_mm(x + width), px_to_mm(y));
                let p3 = Point::new(px_to_mm(x + width), px_to_mm(y + height));
                let p4 = Point::new(px_to_mm(x), px_to_mm(y + height));

                let mut line: Line = [p1, p2, p3, p4].into_iter().map(|&pt| (pt, false)).collect();
                line.set_fill(false);
                line.set_closed(true);
                line.set_stroke(true);
                current_layer.add_shape(line);
            },
            _ => {}
        }
    }

    doc.save(&mut BufWriter::new(File::create("test_working.pdf")?))?;

    Ok(())
}
