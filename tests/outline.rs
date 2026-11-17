mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("outline_{name}.html"));
    let output = dir.join(format!("outline_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::helium().unwrap().arg(&input).arg(&output).assert().success();
    common::expand_object_streams(&std::fs::read(output).unwrap())
}

fn count(pdf: &[u8], needle: &str) -> usize {
    pdf.windows(needle.len()).filter(|window| *window == needle.as_bytes()).count()
}

#[test]
fn headings_become_the_outline() {
    let pdf = render(
        "headings",
        "<h1>One</h1><h2>Two</h2><h3 style=\"display:none\">Hidden</h3><h2>Three</h2><h1>Four</h1><h3>Five</h3>\
         <p>no heading</p><svg width=\"10\" height=\"10\"><text>h1</text></svg>",
    );
    assert_eq!(count(&pdf, "/Type /Outlines"), 1);
    // five visible headings, each with a destination; the hidden one is left out
    assert_eq!(count(&pdf, "/Title (One)") + count(&pdf, "/Title (Two)") + count(&pdf, "/Title (Three)")
        + count(&pdf, "/Title (Four)") + count(&pdf, "/Title (Five)"), 5);
    assert_eq!(count(&pdf, "/Title (Hidden)"), 0);
    assert_eq!(count(&pdf, "/Dest ["), 5);
    // the root counts every item: all are open
    assert!(count(&pdf, "/Count 5") >= 1);
}

#[test]
fn a_document_without_headings_has_no_outline() {
    let pdf = render("none", "<p>text</p>");
    assert_eq!(count(&pdf, "/Outlines"), 0);
}
