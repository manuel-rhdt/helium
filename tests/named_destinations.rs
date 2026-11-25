mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str, args: &[&str]) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("named_{name}.html"));
    let output = dir.join(format!("named_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::umbruch().unwrap().args(args).arg("--no-compression").arg(&input).arg(&output).assert().success();
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

fn count(pdf: &str, needle: &str) -> usize {
    pdf.matches(needle).count()
}

#[test]
fn ids_are_named_destinations() {
    let pdf = render(
        "ids",
        "<div id=intro>Intro</div><p id=\"b\">x</p><div style=\"break-before: page\" id=later>y</div>\
         <p id=hidden style=\"display:none\">z</p><p id=intro>second intro</p>",
        &[],
    );
    assert_eq!(count(&pdf, "/Dests"), 1);
    // sorted by name; the hidden element and the second `intro` are not there
    let at = |name: &str| pdf.find(&format!("({name})")).unwrap_or_else(|| panic!("no destination {name}"));
    assert!(at("b") < at("intro") && at("intro") < at("later"));
    assert_eq!(count(&pdf, "(hidden)"), 0);
    assert_eq!(count(&pdf, "(intro)"), 1);
    assert_eq!(count(&pdf, "/XYZ"), 3);
}

#[test]
fn a_document_without_ids_has_no_names() {
    let pdf = render("none", "<p>text</p>", &[]);
    assert_eq!(count(&pdf, "/Dests"), 0);
}

#[test]
fn named_destinations_work_in_cropped_documents() {
    let pdf = render("crop", "<p id=a>text</p>", &["--crop"]);
    assert_eq!(count(&pdf, "(a)"), 1);
}
