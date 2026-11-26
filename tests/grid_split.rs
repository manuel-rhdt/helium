//! A CSS grid that is taller than the rest of its page is split over pages (see
//! `docs/pagination.md`), expressed against the layout dump: the boxes in the grid items are what
//! is counted, which are painted once if nothing is lost or doubled.

mod common;

use serde_json::Value;

/// A page of 200 x 160 pixels with a margin of 10: the content area is 140 pixels high, from 10
/// to 150.
const PAGE: &str = "@page{size:200px 160px;margin:10px}body{margin:0}\
                    .grid{display:grid;grid-template-columns:1fr 1fr}";

fn dump(name: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("grid_split__{}.html", name));
    std::fs::write(&input, format!("<style>{}</style>{}", PAGE, body)).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("grid_split__{}.pdf", name)))
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
const RED: [u8; 3] = [255, 0, 0];

fn assert_on_pages(found: &[(u64, f64, f64)], expected: usize) {
    assert_eq!(found.len(), expected, "every box is painted once: {found:?}");
    for &(page, y, height) in found {
        assert!(y >= 10.0 - 0.01 && y + height <= 150.0 + 0.01, "a box on page {page} at {y} ({height} high) is off the page");
    }
}

fn pages(found: &[(u64, f64, f64)]) -> std::collections::BTreeSet<u64> {
    found.iter().map(|&(page, ..)| page).collect()
}

fn top_of(found: &[(u64, f64, f64)], page: u64) -> f64 {
    found.iter().filter(|&&(on, ..)| on == page).map(|&(_, y, _)| y).fold(f64::MAX, f64::min)
}

fn items(count: usize, height: u32, color: &str) -> String {
    format!("<div style='height:{height}px;background:{color}'></div>").repeat(count)
}

#[test]
fn a_grid_is_split_between_its_rows() {
    // 20 items of 30 pixels in two columns: ten rows of 30 pixels, which are four rows to a page
    let dump = dump("rows", &format!("<div class=grid>{}</div>", items(20, 30, "#0f0")));
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 20);
    assert_eq!(pages(&green).len(), 3, "{green:?}");
    // the items of a row are side by side: they have the same top, and no row is cut
    for &(page, y, height) in &green {
        assert!((height - 30.0).abs() < 0.01, "an item on page {page} at {y} is {height} high");
        assert!(((y - 10.0) / 30.0).fract().abs() < 0.01, "an item on page {page} is at {y}, not in a row");
    }
}

#[test]
fn the_gap_between_two_rows_is_not_at_the_top_of_a_page() {
    // 4 rows of 50 pixels with a gap of 20: the page has room for two rows (120) and the gap that
    // follows them is not at the top of the next page
    let dump = dump(
        "gap",
        &format!("<div class=grid style='row-gap:20px'>{}</div>", items(8, 50, "#0f0")),
    );
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 8);
    for page in pages(&green) {
        assert!((top_of(&green, page) - 10.0).abs() < 0.01, "page {page} starts at {}", top_of(&green, page));
    }
    assert_eq!(pages(&green).len(), 2, "{green:?}");
}

#[test]
fn the_content_around_a_grid_is_where_it_belongs() {
    let dump = dump(
        "around",
        &format!(
            "{}<div class=grid>{}</div>{}",
            boxes(2, 20, "#f00"),
            items(12, 30, "#00f"),
            boxes(2, 20, "#0f0")
        ),
    );
    let red = painted(&dump, RED);
    let blue = painted(&dump, BLUE);
    let green = painted(&dump, GREEN);
    assert_on_pages(&red, 2);
    assert_on_pages(&blue, 12);
    assert_on_pages(&green, 2);
    // the grid starts on the page of the boxes in front of it, behind them, and the boxes behind
    // it go on directly below its last row
    assert_eq!(red[0].0, blue.iter().map(|&(page, ..)| page).min().unwrap());
    let last_blue = blue.iter().map(|&(page, y, height)| (page, y + height)).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let first_green = green.iter().map(|&(page, y, _)| (page, y)).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    assert_eq!(first_green.0, last_blue.0);
    assert!((first_green.1 - last_blue.1).abs() < 0.01, "{first_green:?} {last_blue:?}");
}

#[test]
fn a_grid_row_that_is_taller_than_a_page_is_cut_inside_of_its_items() {
    // one row of two items of 30 boxes of 20 pixels (600) and of 20 boxes of 17 (340): the items
    // are cut where their own boxes end, and what is across the cut goes on the next page
    let dump = dump(
        "tall",
        &format!("<div class=grid>{}{}</div>", format!("<div>{}</div>", boxes(30, 20, "#0f0")), format!("<div>{}</div>", boxes(20, 17, "#00f"))),
    );
    assert_on_pages(&painted(&dump, GREEN), 30);
    assert_on_pages(&painted(&dump, BLUE), 20);
    let green = painted(&dump, GREEN);
    for page in pages(&green) {
        assert!((top_of(&green, page) - 10.0).abs() < 0.01, "page {page} starts at {}", top_of(&green, page));
    }
}

#[test]
fn the_rows_behind_a_tall_row_are_moved_down_with_it() {
    // the first row is cut inside of an item, and the items of the row behind it come below
    // everything that is on the last page of the first row
    let dump = dump(
        "stacked",
        &format!(
            "<div class=grid><div>{}</div><div>{}</div>{}</div>",
            boxes(15, 20, "#0f0"),
            boxes(18, 17, "#00f"),
            items(2, 20, "#f00")
        ),
    );
    let (green, blue, red) = (painted(&dump, GREEN), painted(&dump, BLUE), painted(&dump, RED));
    assert_on_pages(&green, 15);
    assert_on_pages(&blue, 18);
    assert_on_pages(&red, 2);
    let lowest = |found: &[(u64, f64, f64)], page: u64| {
        found.iter().filter(|&&(on, ..)| on == page).map(|&(_, y, height)| y + height).fold(0.0, f64::max)
    };
    let page = red[0].0;
    let bottom = lowest(&green, page).max(lowest(&blue, page));
    assert!(red.iter().all(|&(on, y, _)| on == page && y >= bottom - 0.01), "{red:?} below {bottom}");
}

#[test]
fn an_item_that_spans_rows_is_not_cut_between_them() {
    // an item of three rows next to three rows of one item each: no page break comes inside of
    // the tall item, which has 90 pixels in a page of 140, so the grid is cut in front of it
    let dump = dump(
        "span",
        &format!(
            "{}<div class=grid style='grid-template-columns:1fr 1fr'>\
             <div style='grid-row:1/4;height:90px;background:#00f'></div>{}</div>",
            boxes(1, 100, "#f00"),
            items(3, 30, "#0f0")
        ),
    );
    let blue = painted(&dump, BLUE);
    assert_on_pages(&blue, 1);
    assert_on_pages(&painted(&dump, GREEN), 3);
}

#[test]
fn a_grid_that_is_not_taller_than_the_page_is_not_split() {
    let dump = dump("whole", &format!("{}<div class=grid>{}</div>", boxes(1, 90, "#f00"), items(2, 30, "#0f0")));
    // the grid is one row of 30 pixels and fits on the page behind the box
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 2);
    assert_eq!(pages(&green).len(), 1);
}

#[test]
fn the_box_around_a_grid_has_the_height_of_its_part() {
    // a bordered box around a split grid: it is as high as the content of its page on every page
    let dump = dump(
        "around_box",
        &format!(
            "<div style='background:#f00;padding:5px 0'><div class=grid>{}</div></div>",
            items(12, 30, "#0f0")
        ),
    );
    let red = painted(&dump, RED);
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 12);
    assert!(pages(&red).len() >= 2, "{red:?}");
    for &(page, y, height) in &red {
        assert!(y + height <= 150.0 + 0.01, "the box on page {page} ends at {}", y + height);
        let lowest = green.iter().filter(|&&(on, ..)| on == page).map(|&(_, y, h)| y + h).fold(0.0, f64::max);
        assert!(y + height >= lowest - 0.01, "the box on page {page} ends above its content: {} {lowest}", y + height);
    }
}

#[test]
fn a_card_that_is_taller_than_half_a_page_is_cut() {
    // an item of 400 pixels is more than half a page: it is split like a row of a table is
    let dump = dump(
        "card",
        &format!(
            "<div class=grid style='grid-template-columns:1fr'><div style='background:#00f;padding:4px'>{}</div></div>",
            boxes(20, 20, "#0f0")
        ),
    );
    assert_on_pages(&painted(&dump, GREEN), 20);
    let blue = painted(&dump, BLUE);
    assert!(pages(&blue).len() >= 3, "the card is painted on every page that it goes on: {blue:?}");
}

// flex containers are split the same way

#[test]
fn a_column_flex_container_is_split_between_its_items() {
    let dump = dump("flex_column", &format!("<div style='display:flex;flex-direction:column'>{}</div>", items(12, 30, "#0f0")));
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 12);
    assert_eq!(pages(&green).len(), 3, "{green:?}");
    for page in pages(&green) {
        assert!((top_of(&green, page) - 10.0).abs() < 0.01);
    }
}

#[test]
fn the_lines_of_a_wrapping_flex_container_are_not_cut() {
    let dump = dump(
        "flex_wrap",
        &format!("<div style='display:flex;flex-wrap:wrap'>{}</div>", "<div style='width:50%;height:30px;background:#0f0'></div>".repeat(20)),
    );
    let green = painted(&dump, GREEN);
    assert_on_pages(&green, 20);
    for &(_, y, height) in &green {
        assert!((height - 30.0).abs() < 0.01, "{y} {height}");
        assert!(((y - 10.0) / 30.0).fract().abs() < 0.01, "{y}");
    }
}

#[test]
fn the_items_of_a_flex_row_that_is_taller_than_a_page_go_on_together() {
    let dump = dump(
        "flex_row",
        &format!(
            "<div style='display:flex'><div style='flex:1'>{}</div><div style='flex:1'>{}</div></div>",
            boxes(30, 20, "#0f0"),
            boxes(20, 17, "#00f")
        ),
    );
    assert_on_pages(&painted(&dump, GREEN), 30);
    assert_on_pages(&painted(&dump, BLUE), 20);
}

#[test]
fn a_flex_container_with_a_height_is_one_box() {
    let dump = dump("flex_height", &format!("<div style='display:flex;flex-direction:column;height:60px'>{}</div>", items(1, 30, "#0f0")));
    assert_on_pages(&painted(&dump, GREEN), 1);
}

// forced breaks in the items

fn pages_of(found: &[(u64, f64, f64)]) -> Vec<u64> {
    found.iter().map(|&(page, ..)| page).collect()
}

#[test]
fn a_break_before_an_item_of_a_grid_starts_its_row_on_the_next_page() {
    // rows of 30 pixels: the break is in front of the second row, with plenty of room on the page
    let dump = dump(
        "break_before",
        &format!(
            "<div class=grid><div style='height:30px;background:#0f0'></div><div style='height:30px;background:#0f0'></div>\
             <div style='height:30px;background:#00f;break-before:page'></div><div style='height:30px;background:#00f'></div></div>"
        ),
    );
    let (green, blue) = (painted(&dump, GREEN), painted(&dump, BLUE));
    assert_on_pages(&green, 2);
    assert_on_pages(&blue, 2);
    assert_eq!(pages_of(&green), vec![0, 0]);
    assert_eq!(pages_of(&blue), vec![1, 1]);
    assert!((top_of(&blue, 1) - 10.0).abs() < 0.01, "{blue:?}");
}

#[test]
fn a_break_after_an_item_of_a_column_flex_container_ends_the_page() {
    let dump = dump(
        "flex_break_after",
        "<div style='display:flex;flex-direction:column'>\
         <div style='height:30px;background:#0f0'></div><div style='height:30px;background:#0f0;break-after:page'></div>\
         <div style='height:30px;background:#00f'></div></div>",
    );
    let (green, blue) = (painted(&dump, GREEN), painted(&dump, BLUE));
    assert_eq!(pages_of(&green), vec![0, 0]);
    assert_eq!(pages_of(&blue), vec![1]);
    assert!((top_of(&blue, 1) - 10.0).abs() < 0.01);
}

#[test]
fn a_break_between_the_boxes_in_an_item_cuts_all_the_items() {
    // the break is inside of the first item, behind its first box: the second item (no break in
    // it) is cut at the same place when it is taller, and goes on on the next page
    let dump = dump(
        "break_inside_item",
        &format!(
            "<div class=grid><div><div style='height:30px;background:#0f0;break-after:page'></div>{}</div><div>{}</div></div>",
            boxes(2, 30, "#00f"),
            boxes(3, 30, "#f00")
        ),
    );
    let (green, blue, red) = (painted(&dump, GREEN), painted(&dump, BLUE), painted(&dump, RED));
    assert_eq!(pages_of(&green), vec![0]);
    assert!(blue.iter().all(|&(page, ..)| page == 1), "{blue:?}");
    assert_eq!(blue.len(), 2);
    assert_eq!(red.len(), 3);
}

#[test]
fn a_break_after_the_last_item_is_no_break() {
    let dump = dump(
        "break_last",
        "<div class=grid><div style='height:30px;background:#0f0'></div><div style='height:30px;background:#0f0;break-after:page'></div></div>",
    );
    assert_eq!(pages(&painted(&dump, GREEN)).len(), 1);
}
