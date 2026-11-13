//! Which elements the rules of a style sheet apply to, independent of where the sheet is.

mod common;

use serde_json::Value;

fn text_colors(name: &str, html: &str) -> Vec<Value> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("stylesheets__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("stylesheets__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|text| text["ops"].as_array().unwrap().clone())
        .filter(|op| op["op"] == "color")
        .map(|op| op["value"].clone())
        .collect()
}

const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn rules_for_the_root_element_apply_and_are_inherited() {
    for (name, selector) in [("html", "html"), ("root", ":root")] {
        let colors = text_colors(
            name,
            &format!("<style>{} {{ color: #ff0000 }}</style><p>text</p>", selector),
        );
        assert_eq!(colors, [serde_json::json!(RED)], "{}", selector);
    }
}

#[test]
fn rules_for_the_head_and_the_root_apply_with_an_explicit_head() {
    let colors = text_colors(
        "explicit",
        "<html><head><style>html { color: #ff0000 }</style></head><body><p>text</p></body></html>",
    );
    assert_eq!(colors, [serde_json::json!(RED)]);
}

#[test]
fn a_style_element_in_the_body_applies_to_the_elements_in_front_of_it() {
    let colors = text_colors(
        "late",
        "<p>first</p><style>p { color: #ff0000 }</style><p>second</p>",
    );
    assert_eq!(colors, [serde_json::json!(RED), serde_json::json!(RED)]);
}

#[test]
fn the_rules_of_the_root_element_lose_against_those_of_the_body() {
    let colors = text_colors(
        "body_wins",
        "<style>html { color: #ff0000 } body { color: #0000ff }</style><p>text</p>",
    );
    assert_eq!(colors, [serde_json::json!([0, 0, 255, 255])]);
}
