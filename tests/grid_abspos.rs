//! Absolutely positioned children of grid containers (and of `inline-block` boxes): they are
//! placed in the grid area that `grid-row` and `grid-column` name, and move with the box when it
//! is placed on a line.

mod common;

use serde_json::Value;
use std::path::Path;

fn layout(name: &str, html: &str) -> Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("grid_abspos__{}.html", name));
    let output = dir.join(format!("grid_abspos__{}.pdf", name));
    let json = dir.join(format!("grid_abspos__{}.json", name));
    // a page margin and a body margin, so that offsets from the page origin are not zero
    std::fs::write(
        &input,
        format!("<style>@page{{size:600px 400px;margin:50px}} body{{margin:20px;font-size:10px}}</style>{}", html),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&json)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap()
}

fn border_box(dump: &Value, id: &str) -> (f64, f64, f64, f64) {
    fn find(node: &Value, id: &str) -> Option<(f64, f64, f64, f64)> {
        if node["id"] == id {
            let b = &node["border_box"];
            return Some((b["x"].as_f64()?, b["y"].as_f64()?, b["width"].as_f64()?, b["height"].as_f64()?));
        }
        node["children"].as_array()?.iter().find_map(|child| find(child, id))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
}

const GRID: &str = "position:relative;grid-template-columns:100px 150px;grid-template-rows:150px 100px";

/// The origin of the page content: the page margin and the margin of the body.
const ORIGIN: f64 = 70.0;

#[test]
fn a_grid_with_only_absolute_children_is_still_a_grid() {
    let dump = layout(
        "only-abs",
        &format!(
            "<div id=g style='display:grid;{GRID}'><div id=a style='position:absolute;grid-row:1;grid-column:2;width:10px;height:10px'></div></div>"
        ),
    );
    // the tracks give it its size
    let (gx, gy, gw, gh) = border_box(&dump, "g");
    assert_eq!((gx, gy, gh), (ORIGIN, ORIGIN, 250.0));
    assert!(gw > 0.0);
    // and the child is in its area: the second column
    let (ax, ay, _, _) = border_box(&dump, "a");
    assert_eq!((ax, ay), (ORIGIN + 100.0, ORIGIN));
}

#[test]
fn an_empty_grid_has_the_size_of_its_tracks() {
    let dump = layout("empty", &format!("<div id=g style='display:inline-grid;{GRID}'></div>"));
    let (_, _, w, h) = border_box(&dump, "g");
    assert_eq!((w, h), (250.0, 250.0));
}

#[test]
fn the_insets_are_relative_to_the_grid_area() {
    let dump = layout(
        "insets",
        &format!(
            "<div id=g style='display:grid;{GRID}'>\
             <div id=a style='position:absolute;grid-row:1/2;grid-column:2/3;left:5px;top:3px;width:10px;height:10px'></div>\
             <div id=b style='position:absolute;grid-row:2/3;grid-column:2/3;right:0;bottom:0;margin-right:7px;width:30px;height:10px'></div></div>"
        ),
    );
    let (ax, ay, _, _) = border_box(&dump, "a");
    assert_eq!((ax, ay), (ORIGIN + 105.0, ORIGIN + 3.0));
    // the bottom right corner of the area, which is the one of the grid
    let (bx, by, _, _) = border_box(&dump, "b");
    assert_eq!((bx, by), (ORIGIN + 250.0 - 7.0 - 30.0, ORIGIN + 240.0));
}

#[test]
fn an_inline_grid_takes_its_absolute_children_to_the_line() {
    let dump = layout(
        "inline",
        &format!(
            "<p>text <span id=g style='display:inline-grid;{GRID}'><span id=a style='position:absolute;grid-row:2;grid-column:2;width:10px;height:10px'></span></span></p>"
        ),
    );
    let (gx, gy, _, _) = border_box(&dump, "g");
    let (ax, ay, _, _) = border_box(&dump, "a");
    assert!(gx > ORIGIN, "after the text");
    assert_eq!((ax - gx, ay - gy), (100.0, 150.0));
}

#[test]
fn an_inline_block_takes_its_absolute_children_to_the_line() {
    let dump = layout(
        "inline-block",
        "<p>text <span id=g style='position:relative;display:inline-block;width:100px;height:60px'><span id=a style='position:absolute;left:5px;top:7px;width:10px;height:10px'></span>in</span> more</p>",
    );
    let (gx, gy, _, _) = border_box(&dump, "g");
    let (ax, ay, _, _) = border_box(&dump, "a");
    assert_eq!((ax - gx, ay - gy), (5.0, 7.0));
}

#[test]
fn right_to_left_grids_place_absolute_children_in_the_mirrored_areas() {
    let dump = layout(
        "rtl",
        &format!(
            "<div id=g dir=rtl style='display:grid;width:400px;{GRID}'>\
             <div id=a style='position:absolute;grid-row:1;grid-column:1;justify-self:start;width:10px;height:10px'></div>\
             <div id=b style='position:absolute;grid-row:1;grid-column:2;left:5px;top:3px;width:20px;height:10px'></div>\
             <div id=c style='position:absolute;grid-row:2;grid-column:2;right:0;bottom:0;margin-right:7px;width:30px;height:10px'></div></div>"
        ),
    );
    // the first column is at the right edge; `start` is the right side of it
    let (ax, _, _, _) = border_box(&dump, "a");
    assert_eq!(ax, ORIGIN + 400.0 - 10.0);
    // the area of `grid-column: 2` reaches from the second line to the padding edge (the left one)
    // and `left` is a physical inset
    let (bx, by, _, _) = border_box(&dump, "b");
    assert_eq!((bx, by), (ORIGIN + 5.0, ORIGIN + 3.0));
    let (cx, _, _, _) = border_box(&dump, "c");
    assert_eq!(cx, ORIGIN + 300.0 - 7.0 - 30.0);
}
