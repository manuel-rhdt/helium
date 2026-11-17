//! Floats in the manner of TeX: `float: top` and `float: bottom` take a box out of the text and
//! place it at the top or the bottom of the page that the place where it is called for is on, if it
//! fits there, and else on a later page. Expressed against the layout dump.
//!
//! The pages are 300px wide and 200px high and the text has a line height of 20px, so ten lines
//! fit on a page. Floats take at most 70% of a page that has text on it (140px).

mod common;

use serde_json::{Value, json};

fn dump(name: &str, body: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 200px; margin: 0 }}
        body, p, h2, div {{ margin: 0; padding: 0 }}
        body {{ font: 16px/20px sans-serif }}
        </style></head><body>{}</body></html>"#,
        body
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("floats__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("floats__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A paragraph of `lines` lines with a background of the given colour.
fn text(color: &str, lines: usize) -> String {
    let text = vec!["line"; lines].join("<br>");
    format!(r#"<p style="background-color: {color}">{text}</p>"#)
}

/// A float with a background of the given colour.
fn float(side: &str, color: &str, height: u32, style: &str) -> String {
    format!(r#"<div style="float: {side}; height: {height}px; background-color: {color}; {style}"></div>"#)
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const ORANGE: [u8; 3] = [255, 165, 0];
const RED_CSS: &str = "#ff0000";
const GREEN_CSS: &str = "#00ff00";
const BLUE_CSS: &str = "#0000ff";
const ORANGE_CSS: &str = "#ffa500";

/// `(page, x, y, width, height)` of the pieces of the background of a colour, by page and place.
fn rects(dump: &Value, color: [u8; 3]) -> Vec<(u64, f64, f64, f64, f64)> {
    let color = json!([color[0], color[1], color[2], 255]);
    let mut found: Vec<_> = dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["color"] == color)
        .map(|rect| {
            (
                rect["page"].as_u64().unwrap(),
                rect["x"].as_f64().unwrap(),
                rect["y"].as_f64().unwrap(),
                rect["width"].as_f64().unwrap(),
                rect["height"].as_f64().unwrap(),
            )
        })
        .collect();
    found.sort_by(|a, b| a.partial_cmp(b).unwrap());
    found
}

/// The number of pages that have something painted on them.
fn pages(dump: &Value) -> usize {
    let last = dump["paint"]["rects"].as_array().unwrap().iter().map(|rect| rect["page"].as_u64().unwrap()).max();
    last.map_or(0, |last| last as usize + 1)
}

#[test]
fn a_float_at_the_top_is_above_the_text_that_is_called_for_in_front_of_it() {
    let body = format!("{}{}{}", text(RED_CSS, 6), float("top", GREEN_CSS, 60, ""), text(BLUE_CSS, 2));
    let dump = dump("top", &body);
    // the float is at the top of the page whatever is in front of it in the text
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 300.0, 60.0)]);
    // the text is below it
    assert_eq!(rects(&dump, RED), [(0, 0.0, 60.0, 300.0, 120.0)]);
    // (the second paragraph does not fit next to the float and the first paragraph)
    assert_eq!(rects(&dump, BLUE), [(1, 0.0, 0.0, 300.0, 40.0)]);
}

#[test]
fn a_float_at_the_bottom_ends_at_the_bottom_of_the_page() {
    let body = format!("{}{}{}", text(RED_CSS, 6), float("bottom", GREEN_CSS, 40, ""), text(BLUE_CSS, 2));
    let dump = dump("bottom", &body);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 300.0, 120.0)]);
    assert_eq!(rects(&dump, BLUE), [(0, 0.0, 120.0, 300.0, 40.0)]);
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 160.0, 300.0, 40.0)]);
    assert_eq!(pages(&dump), 1);
}

#[test]
fn a_float_that_does_not_fit_waits_for_the_next_page() {
    // nine lines leave no room for the float, and the paragraph after it has to start on the next page
    let body = format!("{}{}{}", text(RED_CSS, 9), float("top", GREEN_CSS, 40, ""), text(BLUE_CSS, 3));
    let dump = dump("waits", &body);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 300.0, 180.0)]);
    assert_eq!(rects(&dump, GREEN), [(1, 0.0, 0.0, 300.0, 40.0)]);
    assert_eq!(rects(&dump, BLUE), [(1, 0.0, 40.0, 300.0, 60.0)]);
}

#[test]
fn the_text_fills_the_page_while_a_float_waits() {
    // the float does not fit next to the first paragraph, but the paragraph after it does: the
    // page is filled with the text, and the float comes at the top of the next page
    let body = format!(
        "{}{}{}{}",
        text(RED_CSS, 6),
        float("top", GREEN_CSS, 100, ""),
        text(BLUE_CSS, 3),
        text(ORANGE_CSS, 4)
    );
    let dump = dump("fills", &body);
    // (6 + 3 lines are 180px, and the float would make it 280px)
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 300.0, 120.0)]);
    assert_eq!(rects(&dump, BLUE), [(0, 0.0, 120.0, 300.0, 60.0)]);
    assert_eq!(rects(&dump, GREEN), [(1, 0.0, 0.0, 300.0, 100.0)]);
    assert_eq!(rects(&dump, ORANGE), [(1, 0.0, 100.0, 300.0, 80.0)]);
}

#[test]
fn floats_keep_their_order() {
    // the second float would fit on the first page, but the first does not, so it waits, too
    let body = format!(
        "{}{}{}{}",
        text(RED_CSS, 8),
        float("top", GREEN_CSS, 50, ""),
        float("top", BLUE_CSS, 10, ""),
        text(ORANGE_CSS, 4)
    );
    let dump = dump("order", &body);
    assert_eq!(rects(&dump, GREEN).iter().map(|rect| (rect.0, rect.2)).collect::<Vec<_>>(), [(1, 0.0)]);
    assert_eq!(rects(&dump, BLUE).iter().map(|rect| (rect.0, rect.2)).collect::<Vec<_>>(), [(1, 50.0)]);
}

#[test]
fn a_float_that_is_too_big_to_share_a_page_with_text_gets_a_page_of_its_own() {
    // 150px are more than the 70% of the page that floats may take next to text
    let body = format!("{}{}{}", text(RED_CSS, 2), float("top", GREEN_CSS, 150, ""), text(BLUE_CSS, 2));
    let dump = dump("page", &body);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 0.0, 300.0, 40.0)]);
    assert_eq!(rects(&dump, BLUE), [(0, 0.0, 40.0, 300.0, 40.0)]);
    // the page of floats follows the page of text
    assert_eq!(rects(&dump, GREEN), [(1, 0.0, 0.0, 300.0, 150.0)]);
}

#[test]
fn floats_that_wait_at_the_end_of_the_document_are_not_lost() {
    let body = format!("{}{}{}", text(RED_CSS, 9), float("top", GREEN_CSS, 40, ""), float("bottom", BLUE_CSS, 40, ""));
    let dump = dump("end", &body);
    assert_eq!(rects(&dump, GREEN), [(1, 0.0, 0.0, 300.0, 40.0)]);
    assert_eq!(rects(&dump, BLUE), [(1, 0.0, 160.0, 300.0, 40.0)]);
    assert_eq!(pages(&dump), 2);
}

#[test]
fn the_margins_of_a_float_are_part_of_what_it_takes() {
    let style = "margin: 10px 20px";
    let body = format!("{}{}", float("top", GREEN_CSS, 40, style), text(RED_CSS, 2));
    let dump = dump("margins", &body);
    // (the margin box is 60px high and the box is as wide as the page minus the margins)
    assert_eq!(rects(&dump, GREEN), [(0, 20.0, 10.0, 260.0, 40.0)]);
    assert_eq!(rects(&dump, RED), [(0, 0.0, 60.0, 300.0, 40.0)]);
}

#[test]
fn a_float_in_a_box_is_placed_on_the_page_not_in_the_box() {
    let body = format!(
        r#"<div style="padding: 0 30px">{}{}</div>"#,
        text(RED_CSS, 2),
        float("top", GREEN_CSS, 40, "")
    );
    let dump = dump("nested", &body);
    // (the float is as wide as the page, whatever box it is in)
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 300.0, 40.0)]);
    assert_eq!(rects(&dump, RED), [(0, 30.0, 40.0, 240.0, 40.0)]);
}

#[test]
fn a_float_is_a_block_where_the_document_is_one_page_that_is_as_tall_as_its_content() {
    // with `--crop` there are no pages to place floats on
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        body, p, div {{ margin: 0; padding: 0 }}
        body {{ font: 16px/20px sans-serif }}
        </style></head><body>{}{}{}</body></html>"#,
        text(RED_CSS, 2),
        float("bottom", GREEN_CSS, 40, ""),
        text(BLUE_CSS, 2)
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("floats__crop.html");
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--crop")
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("floats__crop.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    // the float stays where it is called for
    assert_eq!(rects(&dump, GREEN)[0].2, 40.0);
    assert_eq!(rects(&dump, BLUE)[0].2, 80.0);
}

#[test]
fn a_container_with_columns_below_a_float_starts_on_the_page_of_the_float() {
    // the room that the columns have is what the float leaves of the page
    let lines = vec!["line"; 60].join("<br>");
    let body = format!(
        r#"{}<div style="column-count: 2; column-gap: 0; background-color: {BLUE_CSS}">{lines}</div>"#,
        float("top", GREEN_CSS, 60, "")
    );
    let dump = dump("columns", &body);
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 300.0, 60.0)]);
    // 60 lines in two columns are 30 lines, 600px, over 140px of the first page
    let first = rects(&dump, BLUE)[0];
    assert_eq!((first.0, first.2, first.4), (0, 60.0, 140.0));
}

/// A document with a container with two columns, on pages that are 300px high.
fn columns_dump(name: &str, content: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 300px; margin: 0 }}
        body, p, div {{ margin: 0; padding: 0 }}
        body {{ font: 16px/20px sans-serif }}
        </style></head><body><div style="column-count: 2; column-gap: 20px">{content}</div></body></html>"#
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("floats__{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("floats__{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn a_float_in_a_container_with_columns_is_placed_in_a_column() {
    // (a float that is called for in a column is as wide as the column)
    let content = format!(
        "{}{}{}{}",
        text(RED_CSS, 6),
        float("top", GREEN_CSS, 50, ""),
        text(ORANGE_CSS, 8),
        float("bottom", BLUE_CSS, 30, "")
    ) + &text(ORANGE_CSS, 8);
    let dump = columns_dump("column", &content);
    // the one at the top is above the text of the first column, and the one at the bottom ends
    // the second, which is as high as the first
    assert_eq!(rects(&dump, GREEN), [(0, 0.0, 0.0, 140.0, 50.0)]);
    assert_eq!(rects(&dump, BLUE), [(0, 160.0, 240.0, 140.0, 30.0)]);
    assert_eq!(rects(&dump, RED)[0].2, 50.0);
}

#[test]
fn a_float_for_the_page_takes_room_from_the_container_on_the_next_page() {
    let lines = |lines, color| {
        format!(r#"<p style="background-color: {color}">{}</p>"#, vec!["line"; lines].join("<br>"))
    };
    // 30 lines fill the two columns of the first page
    let content = format!(
        r#"{}<div style="float: top; float-reference: page; height: 70px; background-color: {GREEN_CSS}"></div>{}"#,
        lines(30, RED_CSS),
        lines(30, BLUE_CSS)
    );
    let dump = columns_dump("page_float", &content);
    // the float is as wide as the page, at the top of the page after the one it is called for on
    assert_eq!(rects(&dump, GREEN), [(1, 0.0, 0.0, 300.0, 70.0)]);
    // the columns start below it and are 70px lower than they would be
    assert_eq!(rects(&dump, BLUE)[0], (1, 0.0, 70.0, 140.0, 220.0));
    assert_eq!(rects(&dump, BLUE)[1], (1, 160.0, 70.0, 140.0, 220.0));
}
