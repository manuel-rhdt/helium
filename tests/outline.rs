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

#[test]
fn bookmark_level_none_removes_a_heading() {
    let pdf = render(
        "none_level",
        "<style>.skip { bookmark-level: none }</style><h1>Kept</h1><h1 class=skip>Dropped</h1>",
    );
    assert_eq!(count(&pdf, "/Title (Kept)"), 1);
    assert_eq!(count(&pdf, "/Title (Dropped)"), 0);
}

#[test]
fn any_element_can_have_a_level_and_a_label() {
    let pdf = render(
        "custom",
        "<style>.part { bookmark-level: 1 } .sub { bookmark-level: 2; bookmark-label: \"Sub: \" attr(data-name) }</style>\
         <div class=part>Part I</div><div class=sub data-name=\"x\">ignored text</div>\
         <div class=sub>no attribute</div>",
    );
    assert_eq!(count(&pdf, "/Title (Part I)"), 1);
    assert_eq!(count(&pdf, "/Title (Sub: x)"), 1);
    assert_eq!(count(&pdf, "/Title (ignored text)"), 0);
    // `attr()` of a missing attribute is empty, so the third label is "Sub:" (whitespace is collapsed and trimmed)
    assert_eq!(count(&pdf, "/Title (Sub:)"), 1);
    assert_eq!(count(&pdf, "/Dest ["), 3);
}

#[test]
fn aria_headings_are_in_the_outline() {
    let pdf = render("aria", "<div role=heading aria-level=2>Aria</div>");
    assert_eq!(count(&pdf, "/Title (Aria)"), 1);
}

#[test]
fn closed_state_gives_a_negative_count() {
    let pdf = render(
        "closed",
        "<style>.c { bookmark-state: closed }</style><h1 class=c>Top</h1><h2>Child</h2><h2>Child 2</h2>",
    );
    assert_eq!(count(&pdf, "/Count -2"), 1);
}

#[test]
fn the_outline_panel_is_opened() {
    let with = render("panel", "<h1>One</h1>");
    assert_eq!(count(&with, "/PageMode /UseOutlines"), 1);
    let without = render("no_panel", "<p>text</p>");
    assert_eq!(count(&without, "/PageMode"), 0);
}
