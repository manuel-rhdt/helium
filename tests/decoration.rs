//! Tests for `text-decoration`, expressed against the layout dump.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn lines(name: &str, html: &str) -> Vec<Value> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(
            dir.join("wpt-pdfs")
                .join(format!("decoration__{}.pdf", name)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["lines"].as_array().unwrap().clone()
}

fn width(line: &Value) -> f64 {
    line["to"][0].as_f64().unwrap() - line["from"][0].as_f64().unwrap()
}

fn y(line: &Value) -> f64 {
    line["from"][1].as_f64().unwrap()
}

#[test]
fn no_decoration_by_default() {
    assert!(lines("none", "<p>plain text</p>").is_empty());
}

#[test]
fn underline_is_drawn_below_the_text_and_has_its_width() {
    let underlined = lines(
        "underline",
        r#"<p style="text-decoration: underline">some text</p>"#,
    );
    assert_eq!(underlined.len(), 1);
    // the text is 16px high, the baseline is somewhere in the box
    let overlined = lines(
        "overline",
        r#"<p style="text-decoration: overline">some text</p>"#,
    );
    let struck = lines(
        "strike",
        r#"<p style="text-decoration: line-through">some text</p>"#,
    );
    assert!(y(&overlined[0]) < y(&struck[0]));
    assert!(y(&struck[0]) < y(&underlined[0]));
    assert!(width(&underlined[0]) > 40.0);
    assert!((width(&underlined[0]) - width(&struck[0])).abs() < 0.01);
}

#[test]
fn multiple_lines_can_be_combined() {
    let both = lines(
        "both",
        r#"<p style="text-decoration: underline line-through">x</p>"#,
    );
    assert_eq!(both.len(), 2);
}

#[test]
fn decoration_is_inherited_by_inline_children_and_can_be_removed() {
    let inherited = lines(
        "inherited",
        r#"<p style="text-decoration: underline">a <b>b</b> c</p>"#,
    );
    // one line per text run
    assert_eq!(inherited.len(), 3);
    let removed = lines(
        "removed",
        r#"<p style="text-decoration: underline">a <b style="text-decoration: none">b</b> c</p>"#,
    );
    assert_eq!(removed.len(), 2);
}

#[test]
fn decoration_has_the_color_of_the_text() {
    let l = lines(
        "color",
        r#"<p style="text-decoration: underline; color: #ff0000">x</p>"#,
    );
    assert_eq!(l[0]["color"], serde_json::json!([255, 0, 0, 255]));
}

#[test]
fn links_are_underlined_and_blue() {
    let l = lines("link", r#"<p><a href="https://example.com">link</a></p>"#);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["color"], serde_json::json!([0, 0, 238, 255]));
}

#[test]
fn decoration_wraps_with_the_text() {
    let words = "word ".repeat(80);
    let l = lines(
        "wrap",
        &format!(r#"<p style="text-decoration: underline">{}</p>"#, words),
    );
    assert!(
        l.len() > 1,
        "a paragraph of 80 words should take more than one line"
    );
    let mut ys: Vec<i64> = l.iter().map(|line| y(line).round() as i64).collect();
    ys.dedup();
    assert_eq!(ys.len(), l.len());
}
