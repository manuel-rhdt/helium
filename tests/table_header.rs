//! The header rows of a table (`thead`) are repeated at the top of every page that goes on inside
//! of the table, and take room there. Expressed against the layout dump.
//!
//! The pages are 300px wide and 200px high, and every row is 20px high, so ten rows fit on a page.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, table: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 200px; margin: 0 }}
        body {{ margin: 0; font: 16px/20px sans-serif }}
        table {{ border-collapse: separate; border-spacing: 0; width: 100% }}
        td, th {{ padding: 0; height: 20px }}
        th {{ background-color: #ff0000 }}
        td {{ background-color: #0000ff }}
        </style></head><body>{table}</body></html>"#
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("table_header__{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("table_header__{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rows(count: usize) -> String {
    (1..=count).map(|n| format!("<tr><td>{n}</td></tr>")).collect()
}

/// `(page, y)` of the pieces of the background of a colour.
fn tops(dump: &Value, color: [u8; 3]) -> Vec<(u64, f64)> {
    let color = json!([color[0], color[1], color[2], 255]);
    let mut found: Vec<_> = dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["color"] == color)
        .map(|rect| (rect["page"].as_u64().unwrap(), rect["y"].as_f64().unwrap()))
        .collect();
    found.sort_by(|a, b| a.partial_cmp(b).unwrap());
    found
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn the_header_is_at_the_top_of_every_page_of_the_table() {
    // 25 rows and a header: pages of the header and 9 rows (the first of them has the header, too)
    let doc = dump("every_page", &format!("<table><thead><tr><th>h</th></tr></thead><tbody>{}</tbody></table>", rows(25)));
    assert_eq!(tops(&doc, RED), vec![(0, 0.0), (1, 0.0), (2, 0.0)]);
    // the rows come behind the header on every page, and 9 of them fit
    let blue = tops(&doc, BLUE);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 0).count(), 9);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 1).count(), 9);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 2).count(), 7);
    assert!(blue.iter().filter(|(page, _)| *page == 1).all(|&(_, y)| y >= 20.0));
    assert_eq!(blue.iter().find(|(page, _)| *page == 1).map(|&(_, y)| y), Some(20.0));
}

#[test]
fn a_table_without_a_header_has_all_the_room() {
    let doc = dump("no_header", &format!("<table><tbody>{}</tbody></table>", rows(25)));
    let blue = tops(&doc, BLUE);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 0).count(), 10);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 1).count(), 10);
    assert_eq!(blue.iter().filter(|(page, _)| *page == 2).count(), 5);
}

#[test]
fn a_table_that_fits_on_a_page_does_not_repeat_its_header() {
    let doc = dump("fits", &format!("<table><thead><tr><th>h</th></tr></thead><tbody>{}</tbody></table>", rows(5)));
    assert_eq!(tops(&doc, RED), vec![(0, 0.0)]);
}
