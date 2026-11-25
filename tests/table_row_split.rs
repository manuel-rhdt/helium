//! Table rows that are taller than a page are split over pages (see `docs/pagination.md`),
//! expressed against the layout dump: the boxes in the cells are what is counted, which are
//! painted once if the cells go on from the cut and nothing is lost.

mod common;

use serde_json::Value;

/// A page of 200 x 160 pixels with a margin of 10: the content area is 140 pixels high, from 10
/// to 150.
const PAGE: &str = "@page{size:200px 160px;margin:10px}body{margin:0}\
                    table{border-spacing:0;width:100%}td{padding:0;vertical-align:top}";

fn dump(name: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("table_row_split__{}.html", name));
    std::fs::write(&input, format!("<style>{}</style>{}", PAGE, body)).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("table_row_split__{}.pdf", name)))
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

fn boxes(count: usize, height: u32, color: &str) -> String {
    format!("<div style='height:{height}px;background:{color}'></div>").repeat(count)
}

const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn assert_on_pages(found: &[(u64, f64, f64)], expected: usize) {
    assert_eq!(found.len(), expected, "every box is painted once: {found:?}");
    for &(page, y, height) in found {
        assert!(y >= 10.0 - 0.01 && y + height <= 150.0 + 0.01, "a box on page {page} at {y} ({height} high) is off the page");
    }
}

#[test]
fn a_cell_that_is_taller_than_a_page_goes_on_over_the_pages() {
    // 30 boxes of 20 pixels in a cell: 600 pixels, which is five pages of 7 boxes
    let dump = dump("one_cell", &format!("<table><tr><td>{}</td></tr></table>", boxes(30, 20, "#0f0")));
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 30);
    let pages: std::collections::BTreeSet<u64> = green.iter().map(|&(page, ..)| page).collect();
    assert!(pages.len() >= 5, "{pages:?}");
    // the first box of every page is at the top of the page
    for page in pages {
        let top = green.iter().filter(|&&(on, ..)| on == page).map(|&(_, y, _)| y).fold(f64::MAX, f64::min);
        assert!((top - 10.0).abs() < 0.01, "page {page} starts at {top}");
    }
}

#[test]
fn cells_with_other_heights_of_their_lines_go_on_together() {
    // boxes of 17 and of 20 pixels have a common cut only at 340: the cells are cut where their
    // own boxes end, and what is across the cut goes on the next page
    let dump = dump(
        "two_cells",
        &format!(
            "<table><tr><td>{}</td><td>{}</td></tr></table>",
            boxes(24, 17, "#0f0"),
            boxes(20, 20, "#00f")
        ),
    );
    assert_on_pages(&painted(&dump, GREEN), 24);
    assert_on_pages(&painted(&dump, BLUE), 20);
}

#[test]
fn the_rows_around_a_split_row_are_where_they_belong() {
    let dump = dump(
        "around",
        &format!(
            "<table><tr><td>{}</td></tr><tr><td>{}</td></tr><tr><td>{}</td></tr></table>",
            boxes(2, 20, "#0f0"),
            boxes(15, 20, "#00f"),
            boxes(2, 20, "#f00")
        ),
    );
    assert_on_pages(&painted(&dump, GREEN), 2);
    let blue = painted(&dump, BLUE);
    assert_on_pages(&blue, 15);
    let red = painted(&dump, [255, 0, 0]);
    assert_on_pages(&red, 2);
    // the row behind the split one starts below the last box of it on the last page, and the one
    // in front of it is on the first page, above the first box
    let last_blue = blue.iter().map(|&(page, y, height)| (page, y + height)).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let first_red = red.iter().map(|&(page, y, _)| (page, y)).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    assert_eq!(first_red.0, last_blue.0, "the rows after the split one go on on its last page");
    assert!(first_red.1 >= last_blue.1 - 0.01, "{first_red:?} {last_blue:?}");
    let green_page = painted(&dump, GREEN).iter().map(|&(page, ..)| page).max().unwrap();
    let blue_first = blue.iter().map(|&(page, ..)| page).min().unwrap();
    assert!(green_page <= blue_first);
}

#[test]
fn a_row_of_up_to_half_a_page_is_not_split() {
    // 3 boxes of 20 pixels are less than half of the page of 140: the row is kept whole, on the
    // next page where it does not fit what is left of the first
    let dump = dump(
        "kept_whole",
        &format!(
            "<table><tr><td>{}</td></tr><tr><td>{}</td></tr></table>",
            boxes(5, 20, "#0f0"),
            boxes(3, 20, "#00f")
        ),
    );
    let blue = painted(&dump, BLUE);
    assert_on_pages(&blue, 3);
    let pages: std::collections::BTreeSet<u64> = blue.iter().map(|&(page, ..)| page).collect();
    assert_eq!(pages.len(), 1, "{blue:?}");
    assert_eq!(*pages.iter().next().unwrap(), 1, "{blue:?}");
}

#[test]
fn a_row_of_more_than_half_a_page_fills_the_page_and_goes_on() {
    // 6 boxes of 20 pixels are more than half of the page: where only two fit on the first page
    // the row is split, instead of leaving the room empty
    let dump = dump(
        "fills",
        &format!(
            "<table><tr><td>{}</td></tr><tr><td>{}</td></tr></table>",
            boxes(5, 20, "#0f0"),
            boxes(6, 20, "#00f")
        ),
    );
    let blue = painted(&dump, BLUE);
    assert_on_pages(&blue, 6);
    let first = blue.iter().filter(|&&(page, ..)| page == 0).count();
    let later = blue.iter().filter(|&&(page, ..)| page == 1).count();
    assert_eq!((first, later), (2, 4), "{blue:?}");
}
