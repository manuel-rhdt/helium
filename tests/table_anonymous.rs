//! The anonymous table box around table-internal boxes whose parent is not a table.
//! https://www.w3.org/TR/CSS22/tables.html#anonymous-boxes

mod common;

use serde_json::Value;

fn dump(name: &str, display: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("table_anonymous_{}.html", name));
    let html = format!(
        r#"<html><head><style>body {{ font-family: "DejaVu Sans"; }} table {{ display: {display}; }} td, th {{ padding: 0 }}</style></head><body>
<table id="t"><thead><tr><th id="a">Month</th><th id="b">Inflow</th></tr></thead>
<tbody><tr><td id="c">January</td><td id="d">18</td></tr></tbody></table></body></html>"#
    );
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("table_anonymous_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"].as_array().into_iter().flatten().find_map(|child| find(child, id))
}

fn xy(dump: &Value, id: &str) -> (f64, f64) {
    let b = &find(&dump["document"], id).unwrap_or_else(|| panic!("no #{id}"))["border_box"];
    (b["x"].as_f64().unwrap(), b["y"].as_f64().unwrap())
}

#[test]
fn block_table_keeps_its_cells_in_a_grid() {
    let block = dump("block", "block");
    let table = dump("table", "table");
    for id in ["a", "b", "c", "d"] {
        assert_eq!(xy(&block, id), xy(&table, id), "cell #{id}");
    }
    // the cells of a row are side by side
    assert_eq!(xy(&block, "a").1, xy(&block, "b").1);
    assert!(xy(&block, "b").0 > xy(&block, "a").0);
}

fn check_layout(name: &str, html: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("table_anonymous_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--check-layout")
        .arg(&input)
        .arg(dir.join(format!("table_anonymous_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn inline_table_keeps_block_children_at_their_own_widths() {
    check_layout(
        "inline_width",
        r#"<!doctype html>
<style>
  .container { width: 160px }
  .table { display: inline-table; border-spacing: 0 }
  .wide { width: 320px; height: 20px }
  .narrow { width: 200px; height: 30px }
</style>
<div class="container">
  <div class="table" data-expected-width="320" data-expected-height="50">
    <div class="wide" data-expected-width="320"></div>
    <div class="narrow" data-expected-width="200"></div>
  </div>
</div>"#,
    );
}

#[test]
fn inline_table_percent_height_requires_a_definite_basis() {
    check_layout(
        "inline_height",
        r#"<!doctype html>
<style>
  .table { display: inline-table; border-spacing: 0; width: 100px }
  .child { height: 50% }
  .content { height: 100px }
</style>
<div>
  <div class="table" style="height: 100%" data-expected-height="100">
    <div class="child" data-expected-height="100"><div class="content"></div></div>
  </div>
</div>
<div>
  <div class="table" style="height: 160px" data-expected-height="160">
    <div class="child" data-expected-height="80"><div class="content"></div></div>
  </div>
</div>"#,
    );
}
