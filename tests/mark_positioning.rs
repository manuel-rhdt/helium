//! Tests that the glyph offsets from shaping (`x_offset` and `y_offset` of HarfBuzz, used by
//! fonts to attach marks and by cursive scripts like Nastaliq) end up in the painted text. They
//! use the built-in DejaVu Sans, whose mark positioning (GPOS) covers Latin and Arabic marks.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

/// The text operators of the painted paragraphs of a document with `body`.
fn ops(name: &str, body: &str) -> Vec<Value> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("marks_{}.html", name));
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
        .arg(dir.join(format!("marks_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|text| text["ops"].as_array().unwrap().clone())
        .collect()
}

/// The glyphs and kerns of all `TJ` operators, in drawing order.
fn kerned_items(ops: &[Value]) -> Vec<Value> {
    ops.iter()
        .filter(|op| op["op"] == "kerned")
        .flat_map(|op| op["items"].as_array().unwrap().clone())
        .collect()
}

fn rises(ops: &[Value]) -> Vec<f64> {
    ops.iter()
        .filter(|op| op["op"] == "rise")
        .map(|op| op["value"].as_f64().unwrap())
        .collect()
}

#[test]
fn marks_are_moved_horizontally_onto_their_base() {
    // q has no precomposed form with an acute accent, so the accent is a mark glyph that the
    // font moves to the left, over the q
    let items = kerned_items(&ops("acute", "<p>q&#x0301;</p>"));
    let kerns: Vec<f64> = items.iter().filter_map(Value::as_f64).collect();
    let mark_index = items.iter().rposition(Value::is_array).unwrap();
    let kern_before_mark = items[mark_index - 1].as_f64().expect("a kern before the mark");
    assert!(
        kern_before_mark > 10.0,
        "the mark is moved to the left: {:?}",
        items
    );
    // the pen is moved back after the mark, so that the following text is not shifted
    assert!(kerns.iter().sum::<f64>().abs() < 0.2, "{:?}", items);
}

#[test]
fn marks_are_raised_or_lowered_and_the_baseline_is_restored() {
    let ops = ops("arabic", "<p dir=\"rtl\">شَّ</p><p>after</p>");
    let rises = rises(&ops);
    assert!(
        rises.iter().any(|rise| rise.abs() > 0.5),
        "a mark is moved vertically: {:?}",
        ops
    );
    assert_eq!(rises.last(), Some(&0.0), "{:?}", ops);
}

#[test]
fn text_without_offsets_has_no_rise() {
    let ops = ops("plain", "<p>Plain text, no marks.</p>");
    assert!(rises(&ops).is_empty(), "{:?}", ops);
}
