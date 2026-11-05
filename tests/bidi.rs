//! Tests for bidirectional text, expressed against the layout dump. The fonts are chosen so that
//! the built-in DejaVu Sans (which covers Hebrew) is used when no system font matches.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn dump(name: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("bidi_{}.html", name));
    let html = format!(
        r#"<html><head><style>body {{ font-family: "DejaVu Sans"; }}</style></head><body>{}</body></html>"#,
        body
    );
    std::fs::write(&input, html).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("bidi_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The paragraphs of text that are painted.
fn texts(dump: &Value) -> Vec<Value> {
    dump["paint"]["text"].as_array().unwrap().clone()
}

/// The glyphs of a painted paragraph, in the order in which they are drawn (left to right).
fn glyphs(text: &Value) -> Vec<u64> {
    let mut glyphs = vec![];
    for op in text["ops"].as_array().unwrap() {
        let items = match op["op"].as_str() {
            Some("kerned") => op["items"].as_array().unwrap().clone(),
            Some("glyphs") => vec![op["glyphs"].clone()],
            _ => continue,
        };
        for item in items {
            if let Some(ids) = item.as_array() {
                glyphs.extend(ids.iter().map(|id| id.as_u64().unwrap()));
            }
        }
    }
    glyphs
}

fn x(text: &Value) -> f64 {
    text["x"].as_f64().unwrap()
}

/// The `p` elements of the dump (border boxes), in document order.
fn paragraphs(dump: &Value) -> Vec<Value> {
    fn collect(node: &Value, out: &mut Vec<Value>) {
        if node["tag"] == "p" {
            out.push(node["content_box"].clone());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            collect(child, out);
        }
    }
    let mut out = vec![];
    collect(&dump["document"], &mut out);
    out
}

#[test]
fn right_to_left_paragraphs_start_at_the_right() {
    let dump = dump(
        "rtl_align",
        r#"<p>abc</p><p dir="rtl">אבג</p><p style="direction: rtl; text-align: end">אבג</p>"#,
    );
    let texts = texts(&dump);
    let p = paragraphs(&dump);
    let middle = |p: &Value| p["x"].as_f64().unwrap() + p["width"].as_f64().unwrap() / 2.0;
    assert!(
        x(&texts[0]) < middle(&p[0]),
        "LTR text should be on the left"
    );
    assert!(
        x(&texts[1]) > middle(&p[1]),
        "RTL text should be on the right"
    );
    assert!(x(&texts[2]) < middle(&p[2]), "`end` is the left in RTL");
}

#[test]
fn text_align_end_is_right_in_left_to_right_paragraphs() {
    let dump = dump("ltr_end", r#"<p style="text-align: end">abc</p>"#);
    let p = &paragraphs(&dump)[0];
    let middle = p["x"].as_f64().unwrap() + p["width"].as_f64().unwrap() / 2.0;
    assert!(x(&texts(&dump)[0]) > middle);
}

#[test]
fn bdo_reverses_the_characters() {
    let dump = dump(
        "bdo",
        r#"<p>abcd</p><p><bdo dir="rtl">abcd</bdo></p><p><bdo dir="ltr">abcd</bdo></p>"#,
    );
    let texts = texts(&dump);
    let forward = glyphs(&texts[0]);
    let mut backward = glyphs(&texts[1]);
    backward.reverse();
    assert_eq!(forward.len(), 4);
    assert_eq!(forward, backward);
    assert_eq!(forward, glyphs(&texts[2]));
}

#[test]
fn hebrew_inside_left_to_right_text_is_drawn_right_to_left() {
    // `<bdo dir="ltr">` draws the Hebrew letters in logical order
    let dump = dump(
        "mixed",
        r#"<p>x אבג y</p><p><bdo dir="ltr">x אבג y</bdo></p>"#,
    );
    let texts = texts(&dump);
    let bidi = glyphs(&texts[0]);
    let logical = glyphs(&texts[1]);
    assert_eq!(bidi.len(), logical.len());
    // "x", " " and " ", "y" stay where they are, the three letters in between are reversed
    assert_eq!(bidi[..2], logical[..2]);
    assert_eq!(bidi[5..], logical[5..]);
    let mut hebrew = logical[2..5].to_vec();
    hebrew.reverse();
    assert_eq!(bidi[2..5], hebrew[..]);
}

#[test]
fn bidi_controls_are_not_drawn() {
    // the `dir` attribute adds invisible isolate characters around the span
    let dump = dump("controls", r#"<p>a <span dir="rtl">b</span></p>"#);
    assert_eq!(glyphs(&texts(&dump)[0]).len(), 3);
}

#[test]
fn inline_blocks_are_reordered_with_the_text() {
    let dump = dump(
        "inline_block",
        r#"<p dir="rtl">אבגדה <span style="display: inline-block; width: 20px; height: 10px; background: red"></span> וזחטי</p>"#,
    );
    let rect = dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["color"] == serde_json::json!([255, 0, 0, 255]))
        .unwrap()
        .clone();
    let p = &paragraphs(&dump)[0];
    let right = p["x"].as_f64().unwrap() + p["width"].as_f64().unwrap();
    let rect_x = rect["x"].as_f64().unwrap();
    let text_x = x(&texts(&dump)[0]);
    // The box is after the first word, which starts at the right edge, and before the second
    // word, which ends where the text starts.
    assert!(rect_x < right - 20.0, "box at {rect_x}, right edge {right}");
    assert!(
        rect_x > text_x + 20.0,
        "box at {rect_x}, text starts at {text_x}"
    );
}
