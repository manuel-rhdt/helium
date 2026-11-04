//! Tests for `display: inline-block`, expressed against the layout dump.

use assert_cmd::prelude::*;
use serde_json::{Value, json};
use std::process::Command;

fn dump(name: &str, html: &str) -> Value {
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
                .join(format!("inline_block__{}.pdf", name)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn f(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap()
}

/// The rects that are filled with `color`.
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

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn boxed(color: &str, style: &str, content: &str) -> String {
    format!(
        r#"<span style="display: inline-block; background-color: {}; {}">{}</span>"#,
        color, style, content
    )
}

#[test]
fn explicit_width_and_padding() {
    let dump = dump(
        "explicit",
        &format!(
            "<p>{}</p>",
            boxed(
                "#ff0000",
                "width: 100px; padding: 4px 6px; border: 2px solid #00ff00",
                "x"
            )
        ),
    );
    let background = &rects_with(&dump, RED)[0];
    // the dump reports the background over the border box: 100 + 2 * 6 + 2 * 2
    assert_eq!(f(background, "width"), 116.0);
    assert!(f(background, "height") >= 8.0);
    let borders = rects_with(&dump, GREEN);
    assert_eq!(borders.len(), 4);
    assert_eq!(f(&borders[0], "width"), 116.0);
}

#[test]
fn auto_width_shrinks_to_fit() {
    let dump = dump(
        "shrink",
        &format!("<p>{}</p>", boxed("#ff0000", "", "short")),
    );
    let background = &rects_with(&dump, RED)[0];
    assert!(
        f(background, "width") > 10.0 && f(background, "width") < 100.0,
        "{}",
        background
    );
}

#[test]
fn auto_width_is_limited_by_the_available_width_and_wraps() {
    let words = "word ".repeat(100);
    let dump = dump("wrap", &format!("<p>{}</p>", boxed("#ff0000", "", &words)));
    let background = &rects_with(&dump, RED)[0];
    // the page content is 604.7px wide
    assert!(f(background, "width") <= 604.8, "{}", background);
    assert!(f(background, "width") > 500.0, "{}", background);
    assert!(
        f(background, "height") > 40.0,
        "the text should wrap inside of the box"
    );
}

#[test]
fn boxes_share_a_line_and_wrap_when_they_do_not_fit() {
    let two_small = dump(
        "side_by_side",
        &format!(
            "<p>{} {}</p>",
            boxed("#ff0000", "width: 100px; height: 30px", ""),
            boxed("#0000ff", "width: 100px; height: 30px", "")
        ),
    );
    let (a, b) = (
        &rects_with(&two_small, RED)[0],
        &rects_with(&two_small, BLUE)[0],
    );
    assert_eq!(f(a, "y"), f(b, "y"));
    assert!(f(b, "x") >= f(a, "x") + 100.0);

    let two_wide = dump(
        "wrapped",
        &format!(
            "<p>{} {}</p>",
            boxed("#ff0000", "width: 400px; height: 30px", ""),
            boxed("#0000ff", "width: 400px; height: 30px", "")
        ),
    );
    let (a, b) = (
        &rects_with(&two_wide, RED)[0],
        &rects_with(&two_wide, BLUE)[0],
    );
    assert!(f(b, "y") >= f(a, "y") + 30.0);
    assert_eq!(f(a, "x"), f(b, "x"));
}

#[test]
fn box_without_text_sits_on_the_baseline() {
    // the bottom margin edge of an empty box is its baseline: the text and the box end together
    let dump = dump(
        "baseline_empty",
        &format!(
            r#"<style>p {{ margin: 0 }}</style><p>text {}</p>"#,
            boxed("#ff0000", "width: 20px; height: 40px", "")
        ),
    );
    let background = &rects_with(&dump, RED)[0];
    let baseline = f(&dump["paint"]["text"][0], "y");
    assert!((f(background, "y") + f(background, "height") - baseline).abs() < 0.01);
}

#[test]
fn baseline_of_a_box_is_the_baseline_of_its_last_line() {
    let dump = dump(
        "baseline_text",
        &format!(
            r#"<style>p {{ margin: 0 }}</style><p>before {} after</p>"#,
            boxed(
                "#ff0000",
                "width: 60px",
                "one two three four five six seven eight"
            )
        ),
    );
    let runs = dump["paint"]["text"].as_array().unwrap();
    // the surrounding text is one paragraph, the box has its own; their last baselines agree
    let mut baselines: Vec<f64> = runs.iter().map(|run| f(run, "y")).collect();
    let outer = baselines[0];
    baselines.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let box_last_line = {
        // the last line of the box: paragraphs advance by `newline` leading ops
        let inner = runs
            .iter()
            .find(|run| f(run, "y") != outer)
            .expect("the box paints its own text");
        let leadings: f64 = inner["ops"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|op| op["op"] == "newline")
            .map(|op| op["leading"].as_f64().unwrap())
            .sum();
        f(inner, "y") + leadings
    };
    assert!(
        (box_last_line - outer).abs() < 0.01,
        "outer {} box {}",
        outer,
        box_last_line
    );
}

#[test]
fn nested_boxes_are_placed_inside_each_other() {
    let inner = boxed("#0000ff", "width: 50px; height: 20px", "");
    let dump = dump(
        "nested",
        &format!(
            "<p>{}</p>",
            boxed(
                "#ff0000",
                "width: 200px; padding: 10px",
                &format!("a {} b", inner)
            )
        ),
    );
    let outer = &rects_with(&dump, RED)[0];
    let inner = &rects_with(&dump, BLUE)[0];
    assert!(f(inner, "x") >= f(outer, "x") + 10.0);
    assert!(f(inner, "x") + 50.0 <= f(outer, "x") + f(outer, "width"));
    assert!(f(inner, "y") >= f(outer, "y"));
    assert!(f(inner, "y") + 20.0 <= f(outer, "y") + f(outer, "height"));
}

#[test]
fn box_after_a_page_break_is_on_that_page() {
    let lines = "<div style=\"height: 20px\"></div>".repeat(50);
    let dump = dump(
        "second_page",
        &format!(
            "{}<p>{}</p>",
            lines,
            boxed("#ff0000", "width: 50px; height: 20px", "")
        ),
    );
    let background = &rects_with(&dump, RED)[0];
    assert_eq!(background["page"], 1);
    assert!(f(background, "y") < 200.0, "{}", background);
}

#[test]
fn text_in_a_box_is_painted_inside_of_the_box() {
    let dump = dump(
        "text_inside",
        &format!(
            "<p>x {}</p>",
            boxed("#ff0000", "width: 100px; padding: 5px", "hello")
        ),
    );
    let background = &rects_with(&dump, RED)[0];
    let runs = dump["paint"]["text"].as_array().unwrap();
    let inner = runs
        .iter()
        .find(|run| f(run, "x") > f(background, "x"))
        .expect("text in the box");
    assert!(f(inner, "x") >= f(background, "x") + 5.0 - 0.01);
    assert!(
        f(inner, "y") > f(background, "y")
            && f(inner, "y") < f(background, "y") + f(background, "height")
    );
}

#[test]
fn shrink_to_fit_counts_nested_boxes() {
    let inner = boxed("#0000ff", "width: 80px; height: 20px", "");
    let dump = dump(
        "nested_shrink",
        &format!("<p>{}</p>", boxed("#ff0000", "", &inner)),
    );
    let outer = &rects_with(&dump, RED)[0];
    let inner = &rects_with(&dump, BLUE)[0];
    assert_eq!(f(inner, "width"), 80.0);
    assert_eq!(f(outer, "width"), 80.0);
}

#[test]
fn absolute_position_makes_the_box_a_block() {
    let dump = dump(
        "absolute",
        r#"<div style="position: absolute; top: 0; left: 0"></div>
           <span style="display: inline-block; position: absolute; top: 100px; left: 50px; width: 30px; height: 30px; background-color: #ff0000"></span>"#,
    );
    let background = &rects_with(&dump, RED)[0];
    // the initial containing block starts at the page margin
    assert!((f(background, "x") - (94.488 + 50.0)).abs() < 0.01);
    assert_eq!(f(background, "width"), 30.0);
}
