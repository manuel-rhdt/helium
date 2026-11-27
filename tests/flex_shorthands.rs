//! The shorthands of the flex and grid properties: `flex-flow`, `gap` (with two values and the
//! names of the grid module), and `place-content`, `place-items` and `place-self`.

mod common;

use serde_json::Value;
use std::path::Path;

fn layout(name: &str, html: &str) -> Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("flex_shorthands__{}.html", name));
    let output = dir.join(format!("flex_shorthands__{}.pdf", name));
    let json = dir.join(format!("flex_shorthands__{}.json", name));
    std::fs::write(
        &input,
        format!(
            "<style>@page{{size:600px 900px;margin:0}} body{{margin:0}} \
             .c{{display:flex;width:250px;background:#ccc}} .c>div{{width:100px;height:30px;background:#333}}</style>{}",
            html
        ),
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

/// The `(x, y)` of the element, relative to the one with the id `root`.
fn at(dump: &Value, root: &str, id: &str) -> (f64, f64) {
    fn find(node: &Value, id: &str) -> Option<(f64, f64)> {
        if node["id"] == id {
            let b = &node["border_box"];
            return Some((b["x"].as_f64()?, b["y"].as_f64()?));
        }
        node["children"].as_array()?.iter().find_map(|child| find(child, id))
    }
    let (rx, ry) = find(&dump["document"], root).unwrap_or_else(|| panic!("no element #{}", root));
    let (x, y) = find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id));
    (x - rx, y - ry)
}

fn height(dump: &Value, id: &str) -> f64 {
    fn find(node: &Value, id: &str) -> Option<f64> {
        if node["id"] == id {
            return node["border_box"]["height"].as_f64();
        }
        node["children"].as_array()?.iter().find_map(|child| find(child, id))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
}

#[test]
fn flex_flow_sets_the_direction_and_the_wrapping_in_any_order() {
    let dump = layout(
        "flow",
        "<div class=c id=a style='flex-flow:row wrap'><div id=a1></div><div id=a2></div><div id=a3></div></div>\
         <div class=c id=b style='flex-flow:wrap column;height:70px'><div id=b1></div><div id=b2></div><div id=b3></div></div>\
         <div class=c id=c style='flex-flow:column'><div id=c1></div><div id=c2></div></div>",
    );
    // two items fit on the first line
    assert_eq!(at(&dump, "a", "a2"), (100.0, 0.0));
    assert_eq!(at(&dump, "a", "a3"), (0.0, 30.0));
    // two items fit in the first column; the second one is half of the free width over
    assert_eq!(at(&dump, "b", "b2"), (0.0, 30.0));
    assert_eq!(at(&dump, "b", "b3"), (125.0, 0.0));
    assert_eq!(at(&dump, "c", "c2"), (0.0, 30.0));
}

#[test]
fn flex_flow_resets_what_it_leaves_out() {
    let dump = layout(
        "reset",
        "<div class=c id=a style='flex-direction:column;flex-flow:wrap'><div id=a1></div><div id=a2></div></div>\
         <div class=c id=b style='flex-flow:column;flex-flow:bogus'><div id=b1></div><div id=b2></div></div>",
    );
    // the `flex-flow` that follows made it a row again
    assert_eq!(at(&dump, "a", "a2"), (100.0, 0.0));
    // an invalid value is ignored
    assert_eq!(at(&dump, "b", "b2"), (0.0, 30.0));
}

#[test]
fn gap_takes_one_or_two_values() {
    let dump = layout(
        "gap",
        "<div class='c' id=a style='flex-wrap:wrap;gap:10px 20px'><div id=a1></div><div id=a2></div><div id=a3></div></div>\
         <div class='c' id=b style='flex-wrap:wrap;gap:15px'><div id=b1></div><div id=b2></div><div id=b3></div></div>\
         <div class='c' id=c style='flex-wrap:wrap;grid-gap:10px 20px'><div id=c1></div><div id=c2></div><div id=c3></div></div>\
         <div class='c' id=d style='flex-wrap:wrap;grid-row-gap:10px;grid-column-gap:20px'><div id=d1></div><div id=d2></div><div id=d3></div></div>",
    );
    // the first value is the gap between the rows, the second the one between the columns
    assert_eq!(at(&dump, "a", "a2"), (120.0, 0.0));
    assert_eq!(at(&dump, "a", "a3"), (0.0, 40.0));
    assert_eq!(at(&dump, "b", "b2"), (115.0, 0.0));
    assert_eq!(at(&dump, "b", "b3"), (0.0, 45.0));
    assert_eq!(at(&dump, "c", "c2"), (120.0, 0.0));
    assert_eq!(at(&dump, "c", "c3"), (0.0, 40.0));
    assert_eq!(at(&dump, "d", "d2"), (120.0, 0.0));
    assert_eq!(at(&dump, "d", "d3"), (0.0, 40.0));
    assert_eq!(height(&dump, "a"), 70.0);
}

#[test]
fn place_shorthands_set_both_alignments() {
    let dump = layout(
        "place",
        "<div class=c id=a style='height:100px;place-items:center'><div id=a1 style='width:50px'></div></div>\
         <div class=c id=b style='height:100px;place-content:center end'><div id=b1 style='width:50px'></div></div>\
         <div class=c id=c style='height:100px;place-items:flex-end'>\
           <div id=c1 style='width:50px;place-self:flex-start'></div><div id=c2 style='width:50px'></div></div>",
    );
    // `align-items: center`
    assert_eq!(at(&dump, "a", "a1"), (0.0, 35.0));
    // `justify-content: end` (the `align-content` of a single line does nothing)
    assert_eq!(at(&dump, "b", "b1"), (200.0, 0.0));
    // `place-self` is stronger than `place-items`
    assert_eq!(at(&dump, "c", "c1"), (0.0, 0.0));
    assert_eq!(at(&dump, "c", "c2"), (50.0, 70.0));
}
