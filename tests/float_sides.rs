//! `float: left` and `float: right` beside the text, laid out by Taffy: a minimal first step. The
//! block that follows a float is a box of its own beside the float (it does not flow around the
//! float). Expressed against the layout dump.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, body: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 200px; margin: 0 }}
        body, p, h2, div {{ margin: 0; padding: 0 }}
        body {{ font: 16px/20px sans-serif }}
        </style></head><body>{}</body></html>"#,
        body
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("float_sides__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("float_sides__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// `(page, x, y, width, height)` of the pieces of the background of a colour.
fn rects(dump: &Value, color: [u8; 3]) -> Vec<(u64, f64, f64, f64, f64)> {
    let color = json!([color[0], color[1], color[2], 255]);
    dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["color"] == color)
        .map(|rect| {
            (
                rect["page"].as_u64().unwrap(),
                rect["x"].as_f64().unwrap(),
                rect["y"].as_f64().unwrap(),
                rect["width"].as_f64().unwrap(),
                rect["height"].as_f64().unwrap(),
            )
        })
        .collect()
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];

#[test]
fn a_float_at_the_left_has_the_text_beside_it() {
    let body = r#"<div style="float: left; width: 100px; height: 60px; background-color: #f00"></div>
        <p style="background-color: #0f0">line<br>line<br>line</p>"#;
    let dump = dump("left", body);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 100.0, 60.0)]);
    assert_eq!(rects(&dump, GREEN), [(0, 100.0, 0.0, 200.0, 60.0)]);
}

#[test]
fn a_float_at_the_right_has_the_text_beside_it() {
    let body = r#"<div style="float: right; width: 100px; height: 60px; background-color: #f00"></div>
        <p style="background-color: #0f0">line<br>line<br>line</p>"#;
    let dump = dump("right", body);
    assert_eq!(rects(&dump, RED), [(0, 200.0, 0.0, 100.0, 60.0)]);
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 200.0, 60.0)]);
}

#[test]
fn a_block_that_is_taller_than_the_float_stays_next_to_it() {
    // (the known limit of this step: the text does not flow under the float)
    let body = r#"<div style="float: left; width: 100px; height: 20px; background-color: #f00"></div>
        <p style="background-color: #0f0">line<br>line<br>line</p>"#;
    let dump = dump("taller", body);
    assert_eq!(rects(&dump, GREEN), [(0, 100.0, 0.0, 200.0, 60.0)]);
}
