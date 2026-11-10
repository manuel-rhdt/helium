use crate::display_list::{self, DisplayItem, PrintPages, Rect};
use crate::layout::components::TextOperator;
use crate::style::properties::computed::PixelLength;
use crate::tagging::TagRecorder;
use std::path::PathBuf;

use lemon_pdf::pagetree::MediaBox;
use lemon_pdf::object::{ByteString, IndirectReference, Name, TextString};
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

/// Tracks the marked content of a text object while it is written.
///
/// In a tagged PDF all content is either tagged or an artifact (PDF/UA). Content that no tag
/// covers (text hidden from the structure with `aria-hidden`, for instance) is wrapped in an
/// implicit `/Artifact` marked-content sequence.
#[derive(Default)]
struct MarkedContent {
    /// For every open marked-content sequence, whether it was written (tags are dropped when
    /// the PDF is not tagged).
    stack: Vec<bool>,
    implicit_artifact: bool,
}

impl MarkedContent {
    /// Called before content is painted: opens an implicit artifact if no marked content
    /// covers the content.
    fn before_content(&mut self, page: &mut PageContext, tagged: bool) -> Result<(), Error> {
        if tagged && self.stack.is_empty() && !self.implicit_artifact {
            page.push_operand(RawPdf("/Artifact".into()))?;
            page.apply_operator("BMC")?;
            self.implicit_artifact = true;
        }
        Ok(())
    }

    /// Called before an explicit marked-content sequence starts at the top level.
    fn close_implicit_artifact(&mut self, page: &mut PageContext) -> Result<(), Error> {
        if self.stack.is_empty() && self.implicit_artifact {
            page.apply_operator("EMC")?;
            self.implicit_artifact = false;
        }
        Ok(())
    }

    fn end(&mut self, page: &mut PageContext) -> Result<(), Error> {
        if self.stack.pop() == Some(true) {
            page.apply_operator("EMC")?;
        }
        Ok(())
    }

    /// Closes everything that is still open at the end of the text object.
    fn finish(mut self, page: &mut PageContext) -> Result<(), Error> {
        while !self.stack.is_empty() {
            self.end(page)?;
        }
        self.close_implicit_artifact(page)
    }
}

type PageContext<'p, 'c, 'b> = lemon_pdf::content::PageContext<'p, 'c, 'b>;

/// Writes the pages as a PDF file. If `tagged`, it is a tagged PDF whose structure tree is the
/// `structure` of the pages.
pub fn write_pdf(
    output_path: PathBuf,
    mut print_pages: PrintPages,
    no_filter: bool,
    crop: bool,
    tagged: bool,
) -> Result<(), Error> {
    let file = BufWriter::new(File::create(output_path)?);
    let mut context = DocumentContext::with_writer(file, Version::Pdf1_7)?;

    let structure = std::mem::take(&mut print_pages.structure);
    let mut recorder = tagged.then(|| TagRecorder::new(&structure, print_pages.pages.len()));
    if let Some(title) = &structure.title {
        context
            .document_info
            .insert("Title".to_owned(), TextString(title.clone()));
        context.set_display_doc_title(true);
    }
    if let Some(lang) = &structure.lang {
        context.set_lang(lang);
    }
    if tagged {
        // PDF/UA (ISO 14289-1 7.2, 7.1) needs both
        if structure.lang.is_none() {
            log::warn!("The document has no `lang`: the PDF does not conform to PDF/UA.");
        }
        if structure.title.is_none() {
            log::warn!("The document has no `<title>`: the PDF does not conform to PDF/UA.");
        }
    }
    context.set_metadata(&xmp_metadata(structure.title.as_deref(), structure.lang.as_deref(), tagged))?;
    // All pages get their references up front so that links can point to later pages.
    let page_references: Vec<IndirectReference<Page>> = (0..print_pages.pages.len())
        .map(|_| context.create_reference())
        .collect();

    let mut font_map = HashMap::new();
    for (font_name, font) in &print_pages.font_map {
        if !font.glyph_advances.lock().is_empty() {
            log::info!("Embedding font {} into PDF file.", font_name);
            let font = font.embed_into_pdf(&mut context)?;
            let font: IndirectReference<lemon_pdf::font::Font> = context.write_object(font)?;
            font_map.insert(font_name.clone(), font);
        }
    }

    log::info!("Write {} pages.", print_pages.pages.len());

    for (page_index, mut print_page) in print_pages.pages.into_iter().enumerate() {
        let mut page = Page::new();
        let page_height = print_page.height;
        if tagged {
            // the key of the page in the parent tree; annotations follow the order of the
            // structure tree
            page.struct_parents = Some(page_index as i64);
            page.tabs = Some(Name::from("S"));
        }
        let recorder = &mut recorder;

        if crop {
            page.set_media_box(MediaBox::new(
                print_page.content_rect.x.into(),
                (page_height - print_page.content_rect.y).into(),
                print_page.content_rect.xmax().into(),
                (page_height - print_page.content_rect.ymax()).into(),
            ));
        } else {
            page.set_media_box(MediaBox::new(
                Pt(0.0),
                Pt(0.0),
                print_page.width.into(),
                print_page.height.into(),
            ));
        }

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
                        let floatize = |val: u8| f64::from(val) / f64::from(u8::MAX);
                        let red = floatize(color.red);
                        let green = floatize(color.green);
                        let blue = floatize(color.blue);
                        page_context.device_rgb_fill_color(red, green, blue)?;
                    }
                    &DisplayItem::StrokeColor(color) => {
                        let floatize = |val: u8| f64::from(val) / f64::from(u8::MAX);
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
                            page_context.push_operand(RawPdf("/Artifact".into()))?;
                            page_context.apply_operator("BMC")?;
                            page_context.move_to(x, y)?
                        } else {
                            continue;
                        }

                        for (x, y) in point_iter {
                            page_context.line_to(x, y)?
                        }

                        page_context.line_width(line_width.into())?;
                        page_context.stroke_path()?;
                        page_context.apply_operator("EMC")?;
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

                        page_context.push_operand(RawPdf("/Artifact".into()))?;
                        page_context.apply_operator("BMC")?;
                        page_context.rect(x, y, width, height)?;
                        page_context.fill_path()?;
                        page_context.apply_operator("EMC")?;
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
                        let mut marked = MarkedContent::default();
                        let font_ref = font_map[font_name];
                        let key = page_context.add_font(font_ref);
                        page_context.set_font(&key, font_size.into())?;
                        page_context.set_position(x.into(), y.into())?;
                        marked.before_content(page_context, tagged)?;
                        page_context.push_operand(glyphs)?;
                        page_context.apply_operator("Tj")?;
                        marked.finish(page_context)?;
                        page_context.end_text()?;
                    }
                    DisplayItem::Paragraph(par) => {
                        let x = par.start_pos.0;
                        let y = page_height - par.start_pos.1;

                        page_context.begin_text()?;
                        page_context.set_position(x.into(), y.into())?;
                        let mut marked = MarkedContent::default();

                        for op in &par.ops {
                            match op {
                                TextOperator::SetFont(font_name, size) => {
                                    // A font that draws no glyph anywhere is not embedded (e.g.
                                    // the font of text that is drawn entirely with fallback
                                    // fonts). Selecting it would change nothing.
                                    if let Some(&font_ref) = font_map.get(font_name) {
                                        let key = page_context.add_font(font_ref);
                                        page_context.set_font(&key, (*size).into())?;
                                    }
                                }
                                &TextOperator::SetColor(color) => {
                                    let floatize =
                                        |val: u8| f64::from(val) / f64::from(u8::MAX);
                                    page_context.device_rgb_fill_color(
                                        floatize(color.red),
                                        floatize(color.green),
                                        floatize(color.blue),
                                    )?;
                                }
                                TextOperator::Glyphs(glyphs) => {
                                    marked.before_content(page_context, tagged)?;
                                    page_context.push_operand(glyphs)?;
                                    page_context.apply_operator("Tj")?;
                                }
                                TextOperator::KernedGlyphs(glyphs) => {
                                    marked.before_content(page_context, tagged)?;
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
                                TextOperator::BeginActualText(text) => {
                                    marked.before_content(page_context, tagged)?;
                                    marked.stack.push(true);
                                    page_context.push_operand(RawPdf("/Span".into()))?;
                                    let mut hex = String::from("FEFF");
                                    for unit in text.encode_utf16() {
                                        hex.push_str(&format!("{unit:04X}"));
                                    }
                                    page_context
                                        .push_operand(RawPdf(format!("<< /ActualText <{hex}> >>")))?;
                                    page_context.apply_operator("BDC")?;
                                }
                                TextOperator::BeginArtifact => {
                                    // page numbers, running headers and footers
                                    marked.close_implicit_artifact(page_context)?;
                                    marked.stack.push(true);
                                    page_context.push_operand(RawPdf("/Artifact".into()))?;
                                    page_context
                                        .push_operand(RawPdf("<< /Type /Pagination >>".into()))?;
                                    page_context.apply_operator("BDC")?;
                                }
                                TextOperator::BeginTag(tag) => match recorder.as_mut() {
                                    Some(recorder) => {
                                        marked.close_implicit_artifact(page_context)?;
                                        marked.stack.push(true);
                                        let (role, mcid) = recorder.mark(page_index, *tag);
                                        page_context.push_operand(RawPdf(format!("/{role}")))?;
                                        page_context.push_operand(RawPdf(format!(
                                            "<< /MCID {mcid} >>"
                                        )))?;
                                        page_context.apply_operator("BDC")?;
                                    }
                                    None => marked.stack.push(false),
                                },
                                TextOperator::EndMarkedContent => {
                                    marked.end(page_context)?;
                                }
                                &TextOperator::SetRise(rise) => {
                                    page_context.push_operand(Pt::from(rise))?;
                                    page_context.apply_operator("Ts")?;
                                }
                                &TextOperator::Embed { x, y, .. } => {
                                    // resolved by the display list; only the position is left
                                    page_context.push_operand(1.0f32)?;
                                    page_context.push_operand(0.0f32)?;
                                    page_context.push_operand(0.0f32)?;
                                    page_context.push_operand(1.0f32)?;
                                    page_context.push_operand(Pt::from(x).0 as f32)?;
                                    page_context.push_operand(Pt::from(page_height - y).0 as f32)?;
                                    page_context.apply_operator("Tm")?;
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

                        marked.finish(page_context)?;
                        page_context.end_text()?;
                    }
                }
            }

            Ok(())
        })?;

        // Write the annotations of this page (currently only hyperlinks). The media box of a
        // cropped page starts at the content rect, so link coordinates must be offset by it.
        for hyperlink in print_pages
            .hyperlinks
            .iter()
            .filter(|link| link.page == page_index)
        {
            let (offset_x, offset_y) = if crop {
                (print_page.content_rect.x, print_page.content_rect.y)
            } else {
                (
                    crate::style::properties::computed::PixelLength(0.0),
                    crate::style::properties::computed::PixelLength(0.0),
                )
            };
            let xmin = hyperlink.rect.x - offset_x;
            let ymin = hyperlink.rect.y - offset_y;
            let xmax = hyperlink.rect.xmax() - offset_x;
            let ymax = hyperlink.rect.ymax() - offset_y;

            use lemon_pdf::annotation::{GoToAction, LinkAction, UriAction};
            let action = if let Some(destination) = &hyperlink.destination {
                // a cross-reference: jump to the position of the target element
                let x = Pt::from(destination.x - offset_x);
                // the destination y is the top of the target element; keep it in the same
                // coordinate space as the link rect (bottom-left origin)
                let y = Pt::from(page_height - (destination.y - offset_y));
                LinkAction::GoTo(GoToAction {
                    destination: lemon_pdf::annotation::Destination {
                        page: page_references[destination.page],
                        point: (x, y),
                    },
                })
            } else {
                LinkAction::Uri(UriAction {
                    uri: ByteString::from(hyperlink.href.as_str()),
                })
            };

            // the annotation is the content of the `Link` element of the structure tree, next to
            // the text of the link
            let link_node = hyperlink.node;
            let struct_parent = match (recorder.as_mut(), link_node) {
                (Some(recorder), Some(_)) => Some(recorder.annotation_key()),
                _ => None,
            };
            let contents = link_node
                .and_then(|node| structure.nodes[node as usize].link_text.clone())
                .or_else(|| Some(hyperlink.href.clone()))
                .map(TextString);

            let mut link = lemon_pdf::annotation::LinkAnnotation::new(
                [
                    Pt::from(xmin),
                    Pt::from(page_height - ymin - (ymax - ymin)),
                    Pt::from(xmax),
                    Pt::from(page_height - ymin),
                ],
                action,
            );
            link.contents = contents;
            link.struct_parent = struct_parent;
            let annotation = lemon_pdf::annotation::Annotation::Link(link);
            let annotation = page.add_annotation(&mut context, annotation)?;
            if let (Some(recorder), Some(node), Some(key)) =
                (recorder.as_mut(), link_node, struct_parent)
            {
                recorder.add_annotation(node, page_index, annotation, key);
            }
        }

        context.add_reserved_page(page_references[page_index], page)?;
    }
    if let Some(recorder) = recorder {
        recorder.finish(&mut context, &page_references)?;
    }
    context.finish()?;

    Ok(())
}

/// An operand that is written to the content stream as it is.
#[derive(Debug)]
struct RawPdf(String);

impl lemon_pdf::object::PdfFormat for RawPdf {
    fn write(&self, output: &mut lemon_pdf::object::Formatter) -> std::io::Result<()> {
        use std::io::Write;
        write!(output, "{}", self.0)
    }
}

/// The XMP packet of the document metadata (ISO 32000-1 14.3.2). A tagged document claims
/// conformance to PDF/UA-1 (`pdfuaid:part`, ISO 14289-1 5); PDF/UA also wants the title in
/// `dc:title` (7.1).
fn xmp_metadata(title: Option<&str>, lang: Option<&str>, tagged: bool) -> String {
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }
    let mut properties = String::new();
    if let Some(title) = title {
        properties += &format!(
            "   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            escape(title)
        );
    }
    if let Some(lang) = lang {
        properties += &format!("   <dc:language><rdf:Bag><rdf:li>{}</rdf:li></rdf:Bag></dc:language>\n", escape(lang));
    }
    properties += &format!(
        "   <pdf:Producer>Helium {}</pdf:Producer>\n",
        env!("CARGO_PKG_VERSION")
    );
    if tagged {
        properties += "   <pdfuaid:part>1</pdfuaid:part>\n";
    }
    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
  <rdf:Description rdf:about=\"\"
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"
    xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\">
{properties}  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end=\"w\"?>"
    )
}
