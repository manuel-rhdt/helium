//! A table that is sized by its contents gives every column the width of its contents, so that no
//! cell breaks its text although the table has room (Taffy's sums of the track sizes are short of
//! the measured width by rounding errors).

mod common;

use serde_json::Value;

fn dump(html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("table_column_width.html");
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("table_column_width.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn heights(node: &Value, tag: &str, found: &mut Vec<f64>) {
    if node["tag"] == tag {
        found.push(node["border_box"]["height"].as_f64().unwrap());
    }
    for child in node["children"].as_array().into_iter().flatten() {
        heights(child, tag, found);
    }
}

fn row_height(size: u32, hyphens: &str, white_space: &str) -> f64 {
    let doc = dump(&format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><style>
        @page {{ size: 420pt 260pt; margin: 20pt }}
        body {{ margin: 0; font-size: {size}pt; hyphens: {hyphens} }}
        table {{ display: block; width: 100% }}
        th, td {{ padding: 0; white-space: {white_space} }}
        </style></head><body><table><tr><th>Month</th><th>End storage</th></tr>
        <tr><td>January</td><td>41</td></tr></table></body></html>"#
    ));
    let mut found = vec![];
    heights(&doc["document"], "th", &mut found);
    assert_eq!(found.len(), 2);
    found[1]
}

#[test]
fn a_column_is_as_wide_as_its_longest_line() {
    for size in [10, 12, 14, 16, 18] {
        for hyphens in ["none", "auto"] {
            // (the height of the row when no cell can break its text)
            let one_line = row_height(size, hyphens, "nowrap");
            assert_eq!(row_height(size, hyphens, "normal"), one_line, "{size}pt, hyphens: {hyphens}");
        }
    }
}
