//! Simple MathML tests, expressed against the layout dump.

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
        .arg(dir.join("wpt-pdfs").join(format!("mathml__{}.pdf", name)))
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

fn height(dump: &Value, id: &str) -> f64 {
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {}", id))["border_box"]["height"]
        .as_f64()
        .unwrap()
}

fn fonts(dump: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for run in dump["paint"]["text"].as_array().unwrap() {
        for op in run["ops"].as_array().unwrap() {
            if op["op"] == "font" {
                names.push(op["name"].as_str().unwrap().to_string());
            }
        }
    }
    names
}

#[test]
fn math_elements_are_kept_in_the_tree() {
    let dump = dump(
        "tree",
        r#"<p id="p"><math id="m"><mi>x</mi><mo>+</mo><mn id="n">1</mn></math></p>"#,
    );
    let math = find(&dump["document"], "m").unwrap();
    assert_eq!(math["tag"], "math");
    let tags: Vec<&str> = math["children"].as_array().unwrap().iter().map(|c| c["tag"].as_str().unwrap()).collect();
    assert_eq!(tags, ["mi", "mo", "mn"]);
    assert_eq!(find(&dump["document"], "n").unwrap()["tag"], "mn");
}

#[test]
fn inline_math_does_not_break_the_line() {
    let dump = dump(
        "inline",
        r#"<style>body { margin: 0 }</style>
           <p id="with" style="margin: 0">a <math><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>
           <p id="without" style="margin: 0">a x + 1 b</p>"#,
    );
    assert_eq!(height(&dump, "with"), height(&dump, "without"));
}

#[test]
fn math_text_uses_a_math_font() {
    let dump = dump("font", r#"<p>a <math><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>"#);
    let fonts = fonts(&dump);
    assert!(fonts.iter().any(|f| f.contains("Math")), "no math font in {:?}", fonts);
    assert!(fonts.iter().any(|f| !f.contains("Math")), "surrounding text should keep its font: {:?}", fonts);
}

#[test]
fn text_after_math_flows_on() {
    let dump = dump(
        "flow",
        r#"<style>body { margin: 0 }</style>
           <p id="p" style="margin: 0"><math><mi>x</mi></math></p><div id="d">after</div>"#,
    );
    let y = |id: &str| find(&dump["document"], id).unwrap()["border_box"]["y"].as_f64().unwrap();
    assert!(y("d") >= y("p") + height(&dump, "p") - 0.001);
}
