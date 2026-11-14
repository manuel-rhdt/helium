//! A table row is as tall as the lines of its cells, also when the cell is wider than its
//! content (padding) and the text wraps only in the width of the content.

mod common;

fn row_heights(html: &str) -> Vec<f64> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("table_row_height.html");
    let output = dir.join("table_row_height.pdf");
    let layout = dir.join("table_row_height.json");
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let dump: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    fn walk(node: &serde_json::Value, out: &mut Vec<f64>) {
        if node["tag"] == "tr" {
            out.push(node["border_box"]["height"].as_f64().unwrap() * 0.75);
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut heights = vec![];
    walk(&dump["document"], &mut heights);
    heights
}

#[test]
fn a_row_is_as_tall_as_the_wrapped_text_of_its_cells() {
    // On a 312pt page the second cell is 214pt wide with 12pt of padding: its text fits on one
    // line in the 226pt that the cell takes up, but needs two lines in the 214pt of the content.
    let heights = row_heights(
        "<style>@page { size: 312pt 200pt; margin: 20pt; }
         html { font: 9.5pt serif; }
         table { width: 100%; border-collapse: collapse; }
         td { padding: 5pt 6pt; border-bottom: 0.5pt solid #000; }</style>
         <table>
           <tr><td>3 · Planning</td><td>Application, heritage statement, neighbour consultation</td></tr>
           <tr><td>4 · Design</td><td>next row</td></tr>
         </table>",
    );
    assert_eq!(heights.len(), 2);
    // two lines of text and the padding, against one line for the second row
    assert!(
        heights[0] > heights[1] + 10.0,
        "the first row ({}pt) must be a line taller than the second ({}pt)",
        heights[0],
        heights[1]
    );
}
