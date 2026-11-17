mod common;

use assert_cmd::prelude::*;

fn render_with(name: &str, html: &str, source_date_epoch: Option<&str>) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("metadata_{name}.html"));
    let output = dir.join(format!("metadata_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    let mut command = common::helium().unwrap();
    command.env_remove("SOURCE_DATE_EPOCH");
    if let Some(epoch) = source_date_epoch {
        command.env("SOURCE_DATE_EPOCH", epoch);
    }
    command.arg("--no-compression").arg(&input).arg(&output).assert().success();
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

fn render(name: &str, html: &str) -> String {
    render_with(name, html, None)
}

const DOCUMENT: &str = "<!doctype html><html lang=en><head><title>T</title>\
    <meta name=Author content=\"Ada  Lovelace\"><meta name=author content=\"Charles Babbage\">\
    <meta name=description content=\"A &amp; B\"><meta name=keywords content=\"engines, notes\">\
    <meta name=generator content=\"Hand\"><meta name=date content=\"2024-03-05T14:30:07Z\">\
    </head><body><p>text</p></body></html>";

#[test]
fn meta_elements_go_to_the_information_dictionary() {
    let pdf = render("info", DOCUMENT);
    assert!(pdf.contains("/Author (Ada Lovelace, Charles Babbage)"), "{pdf}");
    assert!(pdf.contains("/Subject (A & B)"));
    assert!(pdf.contains("/Keywords (engines, notes)"));
    assert!(pdf.contains("/Creator (Hand)"));
    assert!(pdf.contains("/CreationDate (D:20240305143007Z)"));
    assert!(pdf.contains("/ModDate (D:20240305143007Z)"));
    assert!(pdf.contains("/Producer (Helium "));
}

#[test]
fn meta_elements_go_to_the_xmp_packet() {
    let pdf = render("xmp", DOCUMENT);
    assert!(pdf.contains("<dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li><rdf:li>Charles Babbage</rdf:li></rdf:Seq></dc:creator>"));
    assert!(pdf.contains("A &amp; B</rdf:li></rdf:Alt></dc:description>"));
    assert!(pdf.contains("<pdf:Keywords>engines, notes</pdf:Keywords>"));
    assert!(pdf.contains("<xmp:CreatorTool>Hand</xmp:CreatorTool>"));
    assert!(pdf.contains("<xmp:CreateDate>2024-03-05T14:30:07Z</xmp:CreateDate>"));
    assert!(pdf.contains("<xmp:ModifyDate>2024-03-05T14:30:07Z</xmp:ModifyDate>"));
}

#[test]
fn there_are_no_dates_unless_the_document_or_the_environment_gives_one() {
    let pdf = render("no_date", "<title>T</title><p>text</p>");
    assert!(!pdf.contains("CreationDate") && !pdf.contains("xmp:CreateDate"));
    assert!(pdf.contains("/Producer (Helium "));
    // `SOURCE_DATE_EPOCH` gives the date, and the document wins over it
    let pdf = render_with("epoch", "<title>T</title><p>text</p>", Some("1709649007"));
    assert!(pdf.contains("/CreationDate (D:20240305143007Z)"));
    let pdf = render_with("epoch_vs_meta", "<meta name=date content=2020-01-02><p>text</p>", Some("1709649007"));
    assert!(pdf.contains("/CreationDate (D:20200102000000)"));
    // a date that cannot be read is no date
    let pdf = render("bad_date", "<meta name=date content=yesterday><p>text</p>");
    assert!(!pdf.contains("CreationDate"));
}

#[test]
fn the_output_does_not_depend_on_the_time() {
    let first = render("same_1", DOCUMENT);
    let second = render("same_2", DOCUMENT);
    assert_eq!(first, second);
}
