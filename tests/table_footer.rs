//! The footer rows of a table (`tfoot`) are repeated behind the last row of every page that ends
//! inside of the table (CSS Tables 3, 6.2), and the room of the repeated header is kept on a page
//! that starts inside of a row that is split over pages; expressed against the layout dump.

mod common;

use serde_json::Value;

/// A page of 200 x 160 pixels with a margin of 10: the content area is 140 pixels high, from 10
/// to 150.
const PAGE: &str = "@page{size:200px 160px;margin:10px}body{margin:0}\
                    table{border-spacing:0;width:100%}td,th{padding:0}";

fn dump(name: &str, table: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("table_footer__{}.html", name));
    std::fs::write(&input, format!("<style>{}</style>{}", PAGE, table)).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("table_footer__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The painted rectangles of a color, as `(page, y, height)`.
fn painted(dump: &Value, color: [u8; 3]) -> Vec<(u64, f64, f64)> {
    let color = serde_json::json!([color[0], color[1], color[2], 255]);
    dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["color"] == color)
        .map(|rect| (rect["page"].as_u64().unwrap(), rect["y"].as_f64().unwrap(), rect["height"].as_f64().unwrap()))
        .collect()
}

const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const ORANGE: [u8; 3] = [255, 153, 0];

fn row(color: &str, height: u32) -> String {
    format!("<tr><td><div style='height:{height}px;background:{color}'></div></td></tr>")
}

fn table(header: u32, footer: u32, rows: usize) -> String {
    format!(
        "<table><thead>{}</thead><tfoot>{}</tfoot><tbody>{}</tbody></table>",
        row("#00f", header),
        row("#f90", footer),
        row("#0f0", 20).repeat(rows)
    )
}

fn pages(rects: &[(u64, f64, f64)]) -> std::collections::BTreeSet<u64> {
    rects.iter().map(|&(page, ..)| page).collect()
}

#[test]
fn the_footer_is_behind_the_last_row_of_every_page() {
    // 20 rows of 20 pixels, a header and a footer of 20: 5 rows on a page of 140
    let dump = dump("every_page", &table(20, 20, 20));
    let green = painted(&dump, GREEN);
    let footer = painted(&dump, ORANGE);
    let header = painted(&dump, BLUE);
    assert_eq!(green.len(), 20, "{green:?}");
    let pages = pages(&green);
    assert_eq!(pages.len(), 4, "{pages:?}");
    assert_eq!(header.len(), 4, "the header is on every page: {header:?}");
    assert_eq!(footer.len(), 4, "the footer is on every page: {footer:?}");
    for page in pages {
        let on = |rects: &[(u64, f64, f64)]| rects.iter().filter(|r| r.0 == page).cloned().collect::<Vec<_>>();
        let rows_bottom = on(&green).iter().map(|&(_, y, height)| y + height).fold(0.0, f64::max);
        let footer_on = on(&footer);
        assert_eq!(footer_on.len(), 1, "page {page}");
        assert!((footer_on[0].1 - rows_bottom).abs() < 0.01, "the footer is behind the last row of page {page}: {footer_on:?} {rows_bottom}");
        assert!(footer_on[0].1 + footer_on[0].2 <= 150.01, "the footer is on the page {page}: {footer_on:?}");
    }
}

#[test]
fn a_footer_of_more_than_a_quarter_of_the_page_is_not_repeated() {
    // 50 pixels are more than a quarter of 140: it is behind the last row of the table, once
    let dump = dump("too_tall", &table(20, 50, 10));
    let footer = painted(&dump, ORANGE);
    assert_eq!(footer.len(), 1, "{footer:?}");
    // (on the last page of the table, which can be one of its own if the footer does not fit
    // behind the last row)
    let last_page = pages(&painted(&dump, BLUE)).into_iter().max().unwrap();
    assert_eq!(footer[0].0, last_page);
    assert!(footer[0].0 >= pages(&painted(&dump, GREEN)).into_iter().max().unwrap());
}

#[test]
fn a_table_on_one_page_has_its_footer_once() {
    let dump = dump("one_page", &table(20, 20, 3));
    assert_eq!(painted(&dump, ORANGE).len(), 1);
    assert_eq!(pages(&painted(&dump, GREEN)).len(), 1);
}

#[test]
fn the_room_of_the_header_is_kept_where_a_page_starts_inside_of_a_row_that_is_split() {
    // one row of 14 boxes of 20 pixels (more than a page) behind a header of 20: the pages that
    // continue it have the header at their top, and the boxes below it, inside of the page (without
    // the room of the header on the second page, 7 boxes would be taken, which are 160 with it)
    let boxes = "<div style='height:20px;background:#0f0'></div>".repeat(14);
    let dump = dump(
        "header_in_split_row",
        &format!("<table><thead>{}</thead><tbody><tr><td>{boxes}</td></tr></tbody></table>", row("#00f", 20)),
    );
    let green = painted(&dump, GREEN);
    assert_eq!(green.len(), 14, "{green:?}");
    let header = painted(&dump, BLUE);
    assert_eq!(header.len(), pages(&green).len(), "{header:?}");
    for &(page, y, height) in &green {
        assert!(y >= 30.0 - 0.01, "a box on page {page} is above the header: {y}");
        assert!(y + height <= 150.0 + 0.01, "a box on page {page} is below the page: {y} + {height}");
    }
}
