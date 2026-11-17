mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str, args: &[&str]) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("pdfa_{name}.html"));
    let output = dir.join(format!("pdfa_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::helium().unwrap().args(args).arg("--no-compression").arg(&input).arg(&output).assert().success();
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

const DOCUMENT: &str = "<!doctype html><html lang=en><title>T</title><p>text <a href=\"https://example.com\">link</a></p>";

#[test]
fn a_pdfa_file_has_an_output_intent_and_its_identification() {
    let pdf = render("level", DOCUMENT, &["--pdfa", "2u"]);
    assert_eq!(pdf.matches("/OutputIntents").count(), 1);
    assert!(pdf.contains("/S /GTS_PDFA1"));
    assert!(pdf.contains("/OutputConditionIdentifier (sRGB IEC61966-2.1)"));
    assert!(pdf.contains("<pdfaid:part>2</pdfaid:part>"));
    assert!(pdf.contains("<pdfaid:conformance>U</pdfaid:conformance>"));
    // the tagged file says that it follows PDF/UA, which PDF/A wants declared as a schema
    assert!(pdf.contains("<pdfuaid:part>1</pdfuaid:part>"));
    assert!(pdf.contains("<pdfaSchema:prefix>pdfuaid</pdfaSchema:prefix>"));
}

#[test]
fn the_levels_are_written() {
    for (level, part, conformance) in [("2a", 2, 'A'), ("2b", 2, 'B'), ("3a", 3, 'A'), ("3b", 3, 'B'), ("3U", 3, 'U')] {
        let pdf = render("levels", DOCUMENT, &["--pdfa", level]);
        assert!(pdf.contains(&format!("<pdfaid:part>{part}</pdfaid:part>")), "{level}");
        assert!(pdf.contains(&format!("<pdfaid:conformance>{conformance}</pdfaid:conformance>")), "{level}");
    }
}

#[test]
fn an_untagged_file_does_not_declare_pdfua() {
    let pdf = render("untagged", DOCUMENT, &["--pdfa", "2b", "--no-tags"]);
    assert!(pdf.contains("<pdfaid:part>2</pdfaid:part>"));
    assert!(!pdf.contains("pdfuaid:part"));
    assert!(!pdf.contains("<pdfaExtension:schemas>"));
}

#[test]
fn a_file_that_is_not_pdfa_has_none_of_it() {
    let pdf = render("plain", DOCUMENT, &[]);
    assert_eq!(pdf.matches("/OutputIntents").count(), 0);
    assert!(!pdf.contains("pdfaid"));
    assert!(!pdf.contains("pdfaExtension"));
}

#[test]
fn links_are_printed() {
    // PDF/A: every annotation has `/F`, with the print flag (4)
    let pdf = render("flags", DOCUMENT, &[]);
    assert!(pdf.contains("/Subtype /Link"));
    assert!(pdf.contains("/F 4"));
}

#[test]
fn bad_levels_are_refused() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("pdfa_input.html");
    std::fs::write(&input, DOCUMENT).unwrap();
    let output = dir.join("pdfa_refused.pdf");
    for level in ["1b", "4b", "2c", "2", "b"] {
        common::helium().unwrap().args(["--pdfa", level]).arg(&input).arg(&output).assert().failure();
    }
    // level `a` is about the structure tree
    common::helium().unwrap().args(["--pdfa", "2a", "--no-tags"]).arg(&input).arg(&output).assert().failure();
}
