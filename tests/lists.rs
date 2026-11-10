//! Tests for list markers, expressed against the layout dump.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::helium()
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

/// The glyph runs of the list markers of the only list in the document, in document order.
fn markers(dump: &Value) -> Vec<String> {
    let items = find(&dump["document"], "l").unwrap()["children"]
        .as_array()
        .unwrap();
    let left = items[0]["border_box"]["x"].as_f64().unwrap();
    let mut runs: Vec<(f64, f64, String)> = dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|run| run["x"].as_f64().unwrap() < left)
        .map(|run| {
            // what is drawn, without the marked content of the structure tree, whose node
            // numbers depend on the position of the item
            let ops: Vec<&Value> = run["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|op| !matches!(op["op"].as_str(), Some("tag" | "end_marked_content")))
                .collect();
            (
                run["y"].as_f64().unwrap(),
                run["x"].as_f64().unwrap(),
                serde_json::to_string(&ops).unwrap(),
            )
        })
        .collect();
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    runs.into_iter().map(|(_, _, ops)| ops).collect()
}

#[test]
fn ordered_list_honours_the_start_attribute() {
    let started = dump(
        "ol_start",
        r#"<style>body { margin: 0 }</style><ol id="l" start="5"><li>a</li><li>b</li></ol>"#,
    );
    let plain = dump(
        "ol_plain6",
        r#"<style>body { margin: 0 }</style><ol id="l"><li>a</li><li>b</li><li>c</li><li>d</li><li>e</li><li>f</li></ol>"#,
    );
    // `start="5"` numbers like the fifth and sixth item of a list that starts at one
    assert_eq!(markers(&started), markers(&plain)[4..6]);
}

#[test]
fn list_item_value_restarts_the_numbering() {
    let restarted = dump(
        "li_value",
        r#"<style>body { margin: 0 }</style><ol id="l"><li>a</li><li value="9">b</li><li>c</li></ol>"#,
    );
    let plain = dump(
        "ol_plain10",
        r#"<style>body { margin: 0 }</style><ol id="l"><li>a</li><li>b</li><li>c</li><li>d</li><li>e</li><li>f</li><li>g</li><li>h</li><li>i</li><li>j</li></ol>"#,
    );
    let plain = markers(&plain);
    assert_eq!(
        markers(&restarted),
        vec![plain[0].clone(), plain[8].clone(), plain[9].clone()]
    );
}
