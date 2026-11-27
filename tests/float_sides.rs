//! `float: left` and `float: right` beside the text: Taffy places the float and the first lines
//! of the paragraph after it go around it (as long as the float is, whatever the text is). The
//! text is expressed against the layout dump: the lines of a paragraph are the `newline`s of a
//! run of text, which move the pen by `x_offset`.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, body: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 400px; margin: 0 }}
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
const YELLOW: [u8; 3] = [255, 255, 0];

/// Words that are as long as a line of 100px (a few of them go on a line of 200px).
const WORDS: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho \
     sigma tau upsilon phi chi psi omega alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi";

/// The x positions of the lines of the text of the first paragraph (the first run of text).
fn line_starts(dump: &Value) -> Vec<f64> {
    let run = &dump["paint"]["text"][0];
    let mut x = run["x"].as_f64().unwrap();
    let mut starts = vec![x];
    for op in run["ops"].as_array().unwrap() {
        if op["op"] == "newline" {
            x += op["x_offset"].as_f64().unwrap();
            starts.push(x);
        }
    }
    starts
}

fn float(side: &str, height: u32) -> String {
    format!(r#"<div style="float: {side}; width: 100px; height: {height}px; background-color: #f00"></div>"#)
}

#[test]
fn the_first_lines_go_around_a_float_at_the_left() {
    // (a float of 50px takes the lines that start above 50px: three lines of 20px)
    let body = format!(r#"{}<p style="background-color: #0f0">{WORDS}</p>"#, float("left", 50));
    let dump = dump("left", &body);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 100.0, 50.0)]);
    // (the box of the paragraph is as wide as the page: only its lines are shorter)
    assert_eq!(rects(&dump, GREEN)[0].3, 300.0);
    let starts = line_starts(&dump);
    assert_eq!(starts[..5], [100.0, 100.0, 100.0, 0.0, 0.0]);
}

#[test]
fn the_lines_beside_a_float_at_the_right_are_shorter() {
    let alone = dump("alone", &format!(r#"<p style="background-color: #0f0">{WORDS}</p>"#));
    let body = format!(r#"{}<p style="background-color: #0f0">{WORDS}</p>"#, float("right", 50));
    let dump = dump("right", &body);
    assert_eq!(rects(&dump, RED), [(0, 200.0, 0.0, 100.0, 50.0)]);
    // (they start at the left edge, and the text needs more of them)
    assert!(line_starts(&dump).iter().all(|&x| x == 0.0));
    assert!(rects(&dump, GREEN)[0].4 > rects(&alone, GREEN)[0].4);
}

#[test]
fn a_paragraph_is_as_high_as_the_float_that_it_goes_around() {
    let body = format!(r#"{}<p style="background-color: #0f0">short</p><p style="background-color: #ff0">after</p>"#, float("left", 60));
    let dump = dump("short", &body);
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 300.0, 60.0)]);
    assert_eq!(rects(&dump, YELLOW)[0].2, 60.0);
    // (the first line of the short paragraph is beside the float)
    assert_eq!(line_starts(&dump), [100.0]);
}

#[test]
fn floats_at_both_sides_take_the_lines_between_them() {
    let body = format!(r#"{}{}<p style="background-color: #0f0">{WORDS}</p>"#, float("left", 30), float("right", 50));
    let dump = dump("both", &body);
    // (two lines have a float at each side, one has a float at the right only)
    let starts = line_starts(&dump);
    assert_eq!(starts[..4], [100.0, 100.0, 0.0, 0.0]);
}
