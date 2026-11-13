//! `white-space: nowrap`, expressed against the layout dump.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("white_space__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("white_space__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The number of lines of text: one more than the number of line feeds in the text operators.
fn lines(dump: &Value) -> usize {
    let line_feeds: usize = dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|text| {
            text["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|op| op["op"] == "newline" && op["leading"].as_f64().unwrap_or(0.0) > 0.0)
                .count()
        })
        .sum();
    line_feeds + 1
}

fn rects_with(dump: &Value, color: [u8; 3]) -> Vec<Value> {
    let color = json!([color[0], color[1], color[2], 255]);
    dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["color"] == color)
        .cloned()
        .collect()
}

const TEXT: &str = "a fairly long sentence that needs more than one line";

#[test]
fn normal_text_wraps_and_nowrap_text_does_not() {
    let wrapping = dump(
        "wraps",
        &format!(r#"<div style="width: 100px">{}</div>"#, TEXT),
    );
    assert!(lines(&wrapping) >= 3, "{}", wrapping["paint"]);
    let nowrap = dump(
        "nowrap",
        &format!(
            r#"<div style="width: 100px; white-space: nowrap">{}</div>"#,
            TEXT
        ),
    );
    assert_eq!(lines(&nowrap), 1, "{}", nowrap["paint"]);
}

#[test]
fn nowrap_is_inherited_and_can_be_reset() {
    let inherited = dump(
        "inherited",
        &format!(
            r#"<div style="width: 100px; white-space: nowrap"><p>{}</p></div>"#,
            TEXT
        ),
    );
    assert_eq!(lines(&inherited), 1);
    let reset = dump(
        "reset",
        &format!(
            r#"<div style="width: 100px; white-space: nowrap"><p style="white-space: normal">{}</p></div>"#,
            TEXT
        ),
    );
    assert!(lines(&reset) >= 3);
}

#[test]
fn white_space_is_still_collapsed() {
    let collapsed = dump(
        "collapsed",
        r#"<div style="white-space: nowrap; display: inline-block; background-color: #ff0000">a      b
        c</div>"#,
    );
    let spaced = dump(
        "single_spaces",
        r#"<div style="white-space: nowrap; display: inline-block; background-color: #ff0000">a b c</div>"#,
    );
    assert_eq!(
        rects_with(&collapsed, [255, 0, 0])[0]["width"],
        rects_with(&spaced, [255, 0, 0])[0]["width"]
    );
}

#[test]
fn line_breaks_are_still_honored() {
    let dump = dump(
        "br",
        r#"<div style="width: 100px; white-space: nowrap">first line<br>second line that is too long</div>"#,
    );
    assert_eq!(lines(&dump), 2);
}

#[test]
fn a_nowrap_span_is_kept_together_in_wrapping_text() {
    let wide = dump(
        "span_wide",
        r#"<div style="width: 140px; background-color: #ff0000">one <span style="white-space: nowrap; background-color: #0000ff">three four five six</span></div>"#,
    );
    // the span does not fit on the first line, so the paragraph is two lines tall
    let container = &rects_with(&wide, [255, 0, 0])[0];
    assert!(container["height"].as_f64().unwrap() > 30.0, "{}", container);
}

/// Text that cannot wrap has a min-content width as wide as its max-content width.
#[test]
fn nowrap_text_does_not_shrink_below_its_width() {
    let width_of = |name: &str, style: &str| {
        let dump = dump(
            name,
            &format!(
                r#"<div style="width: 100px"><div style="display: inline-block; background-color: #ff0000; {}">{}</div></div>"#,
                style, TEXT
            ),
        );
        rects_with(&dump, [255, 0, 0])[0]["width"].as_f64().unwrap()
    };
    assert!(width_of("shrunk", "") <= 100.0);
    assert!(width_of("not_shrunk", "white-space: nowrap") > 200.0);
}

#[test]
fn inline_blocks_do_not_wrap_either() {
    let boxes = r#"<span style="display: inline-block; width: 50px; height: 10px; background-color: #ff0000"></span>"#.repeat(4);
    let wrapping = dump(
        "boxes_wrap",
        &format!(r#"<div style="width: 140px">{}</div>"#, boxes),
    );
    let nowrap = dump(
        "boxes_nowrap",
        &format!(r#"<div style="width: 140px; white-space: nowrap">{}</div>"#, boxes),
    );
    let ys = |dump: &Value| {
        let mut ys: Vec<f64> = rects_with(dump, [255, 0, 0])
            .iter()
            .map(|r| r["y"].as_f64().unwrap())
            .collect();
        ys.dedup();
        ys
    };
    assert!(ys(&wrapping).len() >= 2);
    assert_eq!(ys(&nowrap).len(), 1);
}
