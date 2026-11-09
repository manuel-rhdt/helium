//! The widths of `<col>` and `<colgroup>` elements (CSS 2.1, 17.5.2.2), expressed against the
//! layout dump.

mod common;

use serde_json::Value;

/// A 300pt (400px) page without margins, so that a column width in pt is `4/3` px.
const PAGE: &str = "@page{width:300pt;height:300pt;margin:0}body{margin:0}\
                    table{border-spacing:0}td{padding:0;border:0;height:10px}";

fn dump(name: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("tables_{}.html", name));
    std::fs::write(&input, format!("<style>{}</style>{}", PAGE, body)).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("tables_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The width of the content box of the element with the given `id`.
fn width(dump: &Value, id: &str) -> f64 {
    fn find(node: &Value, id: &str) -> Option<f64> {
        if node["id"] == id && node["content_box"].is_object() {
            return node["content_box"]["width"].as_f64();
        }
        node["children"]
            .as_array()
            .and_then(|children| children.iter().find_map(|child| find(child, id)))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no box for #{id}"))
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.5
}

#[test]
fn col_widths_size_their_columns() {
    let dump = dump(
        "colgroup",
        "<table><colgroup><col style=\"width:90pt\"><col style=\"width:30pt\"></colgroup>\
         <tr><td id=a></td><td id=b></td></tr></table>",
    );
    // 90pt and 30pt are 120px and 40px
    assert!(close(width(&dump, "a"), 120.0), "{}", width(&dump, "a"));
    assert!(close(width(&dump, "b"), 40.0), "{}", width(&dump, "b"));
}

#[test]
fn a_col_spans_its_columns() {
    let dump = dump(
        "span",
        "<table><colgroup><col span=\"2\" style=\"width:60pt\"></colgroup>\
         <tr><td id=a></td><td id=b></td></tr></table>",
    );
    // both columns are at least 60pt (80px)
    assert!(close(width(&dump, "a"), 80.0), "{}", width(&dump, "a"));
    assert!(close(width(&dump, "b"), 80.0), "{}", width(&dump, "b"));
}

#[test]
fn a_colgroup_without_cols_covers_its_columns() {
    let dump = dump(
        "colgroup_span",
        "<table><colgroup span=\"2\" style=\"width:45pt\"></colgroup>\
         <tr><td id=a></td><td id=b></td></tr></table>",
    );
    assert!(close(width(&dump, "a"), 60.0), "{}", width(&dump, "a"));
    assert!(close(width(&dump, "b"), 60.0), "{}", width(&dump, "b"));
}

#[test]
fn a_colgroup_width_applies_to_the_cols_without_their_own() {
    let dump = dump(
        "colgroup_inherit",
        "<table><colgroup style=\"width:30pt\"><col><col style=\"width:15pt\"></colgroup>\
         <tr><td id=a></td><td id=b></td></tr></table>",
    );
    // the first column takes the group's width, the second its own
    assert!(close(width(&dump, "a"), 40.0), "{}", width(&dump, "a"));
    assert!(close(width(&dump, "b"), 20.0), "{}", width(&dump, "b"));
}

#[test]
fn a_column_is_at_least_as_wide_as_its_col() {
    let dump = dump(
        "cell_wider",
        "<table><colgroup><col style=\"width:15pt\"></colgroup>\
         <tr><td id=a style=\"width:60pt\"></td><td id=b></td></tr></table>",
    );
    // the cell is wider than the column element, so it wins
    assert!(close(width(&dump, "a"), 80.0), "{}", width(&dump, "a"));
}
