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

/// The border box of the element with the `id`, as `(x, width)`.
fn box_of(dump: &Value, id: &str) -> (f64, f64) {
    fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
        if node["id"] == id {
            return Some(node);
        }
        node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|child| find(child, id))
    }
    let node = find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{id}"));
    let border_box = &node["border_box"];
    (
        border_box["x"].as_f64().unwrap(),
        border_box["width"].as_f64().unwrap(),
    )
}

fn right(dump: &Value, id: &str) -> f64 {
    let (x, width) = box_of(dump, id);
    x + width
}

#[test]
fn grid_columns_go_from_right_to_left() {
    let dump = dump(
        "rtl_grid",
        r#"<div id="g" dir="rtl" style="display: grid; grid-template-columns: 1fr 2fr; padding-right: 30px">
             <div id="a">a</div><div id="b">b</div></div>"#,
    );
    let (a, a_width) = box_of(&dump, "a");
    let (b, b_width) = box_of(&dump, "b");
    assert!(a > b, "the first column is at the right: a {a}, b {b}");
    assert!((b_width - 2.0 * a_width).abs() < 0.5);
    // the padding stays at the right
    assert!((right(&dump, "g") - 30.0 - right(&dump, "a")).abs() < 0.5);
}

#[test]
fn flex_items_go_from_right_to_left_and_keep_their_physical_margins() {
    let dump = dump(
        "rtl_flex",
        r#"<div id="f" dir="rtl" style="display: flex">
             <div id="a" style="width: 50px">a</div>
             <div id="b" style="width: 50px; margin-left: 40px">b</div>
             <div id="c" style="width: 50px">c</div></div>"#,
    );
    assert!((right(&dump, "f") - right(&dump, "a")).abs() < 0.5);
    let (a, _) = box_of(&dump, "a");
    let (b, _) = box_of(&dump, "b");
    let (_, c_width) = box_of(&dump, "c");
    assert!((a - right(&dump, "b")).abs() < 0.5, "b is next to a");
    // the margin is on the left of b, between b and c
    assert!((b - 40.0 - right(&dump, "c")).abs() < 0.5);
    assert!((c_width - 50.0).abs() < 0.5);
}

#[test]
fn table_columns_go_from_right_to_left() {
    let dump = dump(
        "rtl_table",
        r#"<table dir="rtl"><tr><td id="a">first</td><td id="b">second</td></tr></table>"#,
    );
    let (a, _) = box_of(&dump, "a");
    let (b, _) = box_of(&dump, "b");
    assert!(a > b, "the first column is at the right: a {a}, b {b}");
}

#[test]
fn narrow_blocks_are_at_the_right_of_right_to_left_containers() {
    let dump = dump(
        "rtl_block",
        r#"<div id="c" dir="rtl">
             <div id="narrow" style="width: 100px"></div>
             <div id="margin" style="margin-left: 50px; margin-right: 20px"></div>
             <table id="t"><tr><td>a</td></tr></table></div>"#,
    );
    assert!((right(&dump, "c") - right(&dump, "narrow")).abs() < 0.5);
    assert!((right(&dump, "c") - right(&dump, "t")).abs() < 0.5);
    // margins stay physical
    let (container, _) = box_of(&dump, "c");
    let (margin, _) = box_of(&dump, "margin");
    assert!((margin - container - 50.0).abs() < 0.5);
    assert!((right(&dump, "c") - 20.0 - right(&dump, "margin")).abs() < 0.5);
}
