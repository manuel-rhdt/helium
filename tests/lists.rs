//! Tests for list markers, expressed against the layout dump.

use assert_cmd::prelude::*;
use serde_json::Value;
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
        .arg(dir.join("wpt-pdfs").join(format!("lists__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| find(child, id))
}

fn y(dump: &Value, id: &str) -> f64 {
    find(&dump["document"], id).unwrap()["border_box"]["y"]
        .as_f64()
        .unwrap()
}

fn x(dump: &Value, id: &str) -> f64 {
    find(&dump["document"], id).unwrap()["border_box"]["x"]
        .as_f64()
        .unwrap()
}

/// The number of text runs (paragraphs) in the dump.
fn runs(dump: &Value) -> usize {
    dump["paint"]["text"].as_array().unwrap().len()
}

#[test]
fn list_items_are_stacked_blocks() {
    let dump = dump(
        "stacked",
        r#"<style>body { margin: 0 }</style><ul id="l" style="margin: 0"><li id="a">a</li><li id="b">b</li></ul>"#,
    );
    assert!(y(&dump, "b") > y(&dump, "a"));
    assert_eq!(x(&dump, "a"), x(&dump, "b"));
}

#[test]
fn list_is_indented() {
    let dump = dump(
        "indent",
        r#"<style>body { margin: 0 }</style><ul id="l" style="margin: 0"><li id="a">a</li></ul>"#,
    );
    assert_eq!(x(&dump, "a") - x(&dump, "l"), 40.0);
}

#[test]
fn every_item_gets_a_marker_unless_none() {
    let with = dump("markers", r#"<ol><li>a</li><li>b</li><li>c</li></ol>"#);
    let without = dump(
        "no_markers",
        r#"<ol style="list-style-type: none"><li>a</li><li>b</li><li>c</li></ol>"#,
    );
    assert_eq!(runs(&with), runs(&without) + 3);
}

#[test]
fn ordered_list_numbers_its_items() {
    use std::collections::HashSet;
    let dump = dump("numbers", r#"<ol><li>a</li><li>b</li></ol>"#);
    // different digits are different glyphs
    let glyphs: HashSet<String> = dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["ops"].to_string())
        .collect();
    assert_eq!(glyphs.len(), 4, "{:?}", glyphs);
}
