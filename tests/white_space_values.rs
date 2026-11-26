//! `white-space`: what happens to spaces and new lines, and whether the lines wrap.
//!
//! | value      | new lines | spaces   | wrapping |
//! | ---------- | --------- | -------- | -------- |
//! | `normal`   | collapse  | collapse | yes      |
//! | `nowrap`   | collapse  | collapse | no       |
//! | `pre`      | keep      | keep     | no       |
//! | `pre-wrap` | keep      | keep     | yes      |
//! | `pre-line` | keep      | collapse | yes      |

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("white_space_values__{name}.html"));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("white_space_values__{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"].as_array()?.iter().find_map(|child| find(child, id))
}

fn measure(dump: &Value, id: &str, side: &str) -> f64 {
    find(&dump["document"], id).unwrap()["border_box"][side].as_f64().unwrap()
}

/// The height of a div of `width` px that has `text` with the white space `value`. The lines are
/// 20px high.
fn height(value: &str, width: u32, text: &str) -> f64 {
    let name = format!("height_{}_{}_{}", value, width, text.len());
    let html = format!(
        r#"<!DOCTYPE html><style>body {{ margin: 0; font: 16px/20px sans-serif }}</style><div id="d" style="width: {width}px; white-space: {value}">{text}</div>"#
    );
    measure(&dump(&name, &html), "d", "height")
}

const LONG: &str = "the quick brown fox jumps over the lazy dog";

#[test]
fn new_lines_are_line_breaks_in_the_preserving_values_only() {
    let text = "one\ntwo\nthree";
    assert_eq!(height("normal", 400, text), 20.0);
    assert_eq!(height("nowrap", 400, text), 20.0);
    for value in ["pre", "pre-wrap", "pre-line"] {
        assert_eq!(height(value, 400, text), 60.0, "{value}");
    }
}

#[test]
fn the_wrapping_values_wrap_and_pre_does_not() {
    // 100px is much narrower than the text
    assert!(height("normal", 100, LONG) >= 60.0);
    assert_eq!(height("pre", 100, LONG), 20.0);
    assert_eq!(height("nowrap", 100, LONG), 20.0);
    assert_eq!(height("pre-wrap", 100, LONG), height("normal", 100, LONG));
    assert_eq!(height("pre-line", 100, LONG), height("normal", 100, LONG));
}

#[test]
fn lines_with_new_lines_wrap_too() {
    // two paragraphs, each wrapping to the same number of lines as it does alone
    let one = height("pre-wrap", 100, LONG);
    assert_eq!(height("pre-wrap", 100, &format!("{LONG}\n{LONG}")), 2.0 * one);
    assert_eq!(height("pre-line", 100, &format!("{LONG}\n{LONG}")), 2.0 * one);
}

fn inline_width(value: &str, text: &str) -> f64 {
    let name = format!("width_{}_{}", value, text.len());
    let html = format!(
        r#"<!DOCTYPE html><style>body {{ margin: 0; font: 16px/20px sans-serif }}</style><span id="s" style="white-space: {value}; display: inline-block">{text}</span>"#
    );
    measure(&dump(&name, &html), "s", "width")
}

#[test]
fn spaces_are_kept_by_pre_and_pre_wrap_and_collapsed_by_the_others() {
    let text = "a          b";
    let collapsed = inline_width("normal", text);
    assert_eq!(inline_width("pre-line", text), collapsed);
    assert_eq!(inline_width("nowrap", text), collapsed);
    let kept = inline_width("pre", text);
    assert!(kept > collapsed + 30.0, "{kept} {collapsed}");
    assert_eq!(inline_width("pre-wrap", text), kept);
}

#[test]
fn pre_line_removes_the_spaces_around_new_lines() {
    // "a \n b" is two lines "a" and "b": as wide as the wider of the two, which is one letter
    let around = inline_width("pre-line", "a   \n   b");
    let plain = inline_width("pre-line", "a\nb");
    assert!((around - plain).abs() < 0.01, "{around} {plain}");
    // pre-wrap keeps them
    assert!(inline_width("pre-wrap", "a   \n   b") > plain + 10.0);
}
