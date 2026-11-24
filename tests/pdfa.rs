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
    for level in ["4b", "2c", "2", "b"] {
        common::helium().unwrap().args(["--pdfa", level]).arg(&input).arg(&output).assert().failure();
    }
    // level `a` is about the structure tree
    common::helium().unwrap().args(["--pdfa", "2a", "--no-tags"]).arg(&input).arg(&output).assert().failure();
}

fn render_with_image(name: &str, image: &str, args: &[&str]) -> std::process::Output {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("pdfa_{name}.html"));
    let output = dir.join(format!("pdfa_{name}.pdf"));
    let _ = std::fs::remove_file(&output);
    std::fs::copy(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(image), dir.join(image)).unwrap();
    std::fs::write(&input, format!("<!doctype html><html lang=en><title>T</title><img src=\"{image}\">")).unwrap();
    common::helium().unwrap().args(args).arg(&input).arg(&output).output().unwrap()
}

#[test]
fn a_cmyk_image_is_an_error_in_pdfa_mode() {
    let output = render_with_image("cmyk", "cmyk.jpg", &["--pdfa", "2b"]);
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("CMYK image"), "{message}");
    // no file is left behind
    let file = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("pdfa_cmyk.pdf");
    assert!(!file.exists());
}

#[test]
fn cmyk_images_are_fine_when_the_file_is_not_pdfa() {
    assert!(render_with_image("cmyk_plain", "cmyk.jpg", &[]).status.success());
    // and an RGB image is fine in PDF/A mode
    assert!(render_with_image("rgb", "rgb.jpg", &["--pdfa", "2b"]).status.success());
}

/// Runs Helium on `html` with `--pdfa 1b`; the output file (if there is one) and what it said on stderr.
fn render_pdfa1(name: &str, html: &str) -> (Option<Vec<u8>>, bool, String) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("pdfa1_{name}.html"));
    let output = dir.join(format!("pdfa1_{name}.pdf"));
    let _ = std::fs::remove_file(&output);
    std::fs::write(&input, html).unwrap();
    let result = common::helium().unwrap().env("RUST_LOG", "warn").args(["--no-system-fonts", "--pdfa", "1b"]).arg(&input).arg(&output).output().unwrap();
    (std::fs::read(&output).ok(), result.status.success(), String::from_utf8_lossy(&result.stderr).into_owned())
}

#[test]
fn a_file_of_pdfa1_has_no_object_streams_and_the_cidset_of_its_fonts() {
    let (file, success, stderr) = render_pdfa1("plain", DOCUMENT);
    assert!(success, "{stderr}");
    let raw = String::from_utf8_lossy(&file.unwrap()).into_owned();
    // ISO 19005-1, 6.1.4: no cross-reference streams, and no objects in streams
    assert!(!raw.contains("/ObjStm") && !raw.contains("/XRef"));
    assert!(raw.contains("\nxref\n"));
    assert!(raw.contains("/CIDSet "));
    assert!(raw.contains("<pdfaid:part>1</pdfaid:part>") && raw.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
    assert!(!raw.contains("/Transparency") && !raw.contains("/SMask"));
}

#[test]
fn only_the_level_b_of_part_1_is_supported() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("pdfa_part1_input.html");
    std::fs::write(&input, DOCUMENT).unwrap();
    for level in ["1a", "1u"] {
        let output = common::helium().unwrap().args(["--pdfa", level]).arg(&input).arg(dir.join("pdfa_part1_refused.pdf")).output().unwrap();
        assert!(!output.status.success(), "{level}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("only the level 1b is"), "{level}");
    }
}

#[test]
fn what_pdfa1_does_not_allow_is_an_error_that_leaves_no_file() {
    let svg = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill-opacity='.5'/%3E%3C/svg%3E";
    for (name, body, found) in [
        ("opacity", "<p style=\"opacity: .5\">x</p>".to_owned(), "`opacity` on an element"),
        ("alpha", "<p style=\"background: rgb(0 0 0 / 50%)\">x</p>".to_owned(), "a colour with an alpha channel"),
        ("gradient", "<div style=\"height: 20px; background: linear-gradient(red, transparent)\"></div>".to_owned(), "a gradient with transparent colours"),
        ("blend", "<div style=\"height: 20px; background: linear-gradient(red, blue), red; background-blend-mode: multiply\"></div>".to_owned(), "`background-blend-mode`"),
        ("svg", format!("<img src=\"{svg}\">"), "an SVG with opacity"),
    ] {
        let (file, success, stderr) = render_pdfa1(name, &format!("<!doctype html><html lang=en><title>T</title>{body}"));
        assert!(!success, "{name}");
        assert!(file.is_none(), "{name}: the file is kept");
        assert!(stderr.contains("PDF/A-1") && stderr.contains(found), "{name}: {stderr}");
    }
}

#[test]
fn colors_of_a_wide_gamut_are_cielab_in_pdfa1() {
    // the ICC spaces of color() are profiles of version 4, which PDF/A-1 does not allow
    let html = "<!doctype html><html lang=en><title>T</title>\
                <p style=\"color: color(display-p3 1 0 0); background: color(a98-rgb 0 1 0)\">x</p>\
                <div style=\"height: 20px; background: linear-gradient(color(display-p3 1 0 0), color(display-p3 0 1 0))\"></div>";
    let (file, success, stderr) = render_pdfa1("wide", html);
    assert!(success, "{stderr}");
    assert!(stderr.contains("clipped to sRGB"), "{stderr}");
    let raw = String::from_utf8_lossy(&file.unwrap()).into_owned();
    assert!(raw.contains("/Lab"), "{raw}");
}
