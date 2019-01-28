use printpdf::{Error, Mm, PdfDocument};

use font_kit::family_name::FamilyName;
use font_kit::handle::Handle;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

use std::fs::File;
use std::io::BufWriter;

pub fn write_pdf(_: crate::FlowTree) -> Result<(), Error> {
    let (doc, page1, layer1) =
        PdfDocument::new("PDF_Document_title", Mm(210.0), Mm(297.0), "Layer 1");
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

    doc.save(&mut BufWriter::new(File::create("test_working.pdf")?))?;

    Ok(())
}
