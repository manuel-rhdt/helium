//! `display: inline-flex` and `display: inline-grid`: a flex or grid container that is placed on a
//! line like an `inline-block`, as wide as its content.

mod common;

use serde_json::Value;
use std::path::Path;

fn layout(name: &str, html: &str) -> Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("inline_flex_grid__{}.html", name));
    let output = dir.join(format!("inline_flex_grid__{}.pdf", name));
    let json = dir.join(format!("inline_flex_grid__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>@page{{size:500px 300px;margin:0}} body{{margin:0;font-size:10px}}</style>{}", html),
    )
    .unwrap();
    let out = common::helium()
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

#[test]
fn an_inline_flex_container_is_as_wide_as_its_items_and_shares_the_line() {
    let dump = layout(
        "flex",
        "<p>a <span id=f style='display:inline-flex'><i style='display:block;width:30px;height:20px'></i><i style='display:block;width:30px;height:10px'></i></span> b <span id=g style='display:inline-flex'><i style='display:block;width:5px;height:5px'></i></span></p>",
    );
    let (fx, _, fw, fh) = border_box(&dump, "f");
    let (gx, gy, gw, _) = border_box(&dump, "g");
    assert_eq!((fw, fh), (60.0, 20.0));
    assert_eq!(gw, 5.0);
    // on the same line, after the first container
    assert!(gx > fx + fw, "{} {}", gx, fx + fw);
    assert!(gy < 30.0);
}

#[test]
fn an_inline_grid_container_is_as_big_as_its_tracks() {
    let dump = layout(
        "grid",
        "<p>x <span id=g style='display:inline-grid;grid-template-columns:100px 150px;grid-template-rows:50px 20px'><i id=a style='grid-column:2;grid-row:1;display:block'></i><i id=b style='display:block;grid-column:1;grid-row:2'></i></span> y</p>",
    );
    let (gx, _, gw, gh) = border_box(&dump, "g");
    assert_eq!((gw, gh), (250.0, 70.0));
    assert!(gx > 0.0, "it is on the line, after the text");
    let (ax, ay, _, _) = border_box(&dump, "a");
    let (bx, by, _, _) = border_box(&dump, "b");
    assert_eq!(ax - gx, 100.0);
    assert_eq!(bx - gx, 0.0);
    assert!(by > ay);
}

#[test]
fn the_children_of_inline_containers_are_items() {
    // a `span` in an `inline-flex` is blockified: the items are side by side, not in a line
    let dump = layout(
        "items",
        "<div style='display:inline-flex'><span id=a style='width:40px;height:10px'>x</span><span id=b style='width:40px;height:10px'>y</span></div>",
    );
    let (ax, ay, aw, _) = border_box(&dump, "a");
    let (bx, by, _, _) = border_box(&dump, "b");
    assert_eq!(aw, 40.0);
    assert_eq!((bx - ax, by - ay), (40.0, 0.0));
}

#[test]
fn a_block_level_neighbour_starts_a_new_line() {
    let dump = layout(
        "block",
        "<span id=f style='display:inline-flex'><i style='display:block;width:30px;height:20px'></i></span><div id=d style='height:5px'></div>",
    );
    let (_, fy, _, fh) = border_box(&dump, "f");
    let (dx, dy, _, _) = border_box(&dump, "d");
    assert_eq!(dx, 0.0);
    assert!(dy >= fy + fh, "{} {}", dy, fy + fh);
}
