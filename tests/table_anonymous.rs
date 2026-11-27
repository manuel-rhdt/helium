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
