//! `inset`, the logical properties (`margin-inline-start`, `padding-block`, `inset-inline-end`,
//! `inline-size`, ...) and `order`.

mod common;

use serde_json::Value;
use std::path::Path;

fn layout(name: &str, html: &str) -> Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("logical__{}.html", name));
    let output = dir.join(format!("logical__{}.pdf", name));
    let json = dir.join(format!("logical__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>@page{{size:600px 900px;margin:0}} body{{margin:0}} div{{width:100px;height:20px}}</style>{}", html),
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

/// The border box `(x, y, width, height)` of the element with this id.
fn bbox(dump: &Value, id: &str) -> (f64, f64, f64, f64) {
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
fn logical_margins_and_padding_follow_the_flow_of_the_box() {
    let dump = layout(
        "box",
        "<div id=a style='margin-inline-start:10px;margin-block-start:5px;padding-inline-end:7px'></div>\
         <div id=b style='margin-block:3px 9px;margin-inline:4px 6px;padding-block:2px 3px'></div>",
    );
    // (the width of the block that fills the line is not what is given: `width` is 100)
    assert_eq!(bbox(&dump, "a"), (10.0, 5.0, 107.0, 20.0));
    // y: 25 (the end of `a`) + 3, and 20 + 2 + 3 of height
    assert_eq!(bbox(&dump, "b"), (4.0, 28.0, 100.0, 25.0));
}

#[test]
fn the_inline_start_side_depends_on_the_direction_and_is_the_one_of_the_box_itself() {
    let dump = layout(
        "direction",
        "<div id=a dir=rtl style='margin-inline-start:30px;width:auto'></div>\
         <div id=b style='direction:rtl;margin-inline-start:30px;width:auto'></div>\
         <div id=c style='margin-inline-start:30px;direction:rtl;width:auto'></div>",
    );
    // the right margin: the box still starts at the left and ends 30px before the right edge
    for id in ["a", "b", "c"] {
        assert_eq!(bbox(&dump, id).0, 0.0, "{}", id);
        assert_eq!(bbox(&dump, id).2, 570.0, "{}", id);
    }
}

#[test]
fn vertical_writing_modes_turn_the_sides() {
    let dump = layout(
        "vertical",
        "<div id=a style='writing-mode:vertical-rl;margin-block-start:10px;margin-inline-start:20px;width:auto;height:50px'></div>\
         <div id=b style='writing-mode:vertical-lr;margin-block-start:10px;width:30px;height:50px'></div>",
    );
    // `vertical-rl`: the block start is the right side, the inline start is the top
    assert_eq!(bbox(&dump, "a"), (0.0, 20.0, 590.0, 50.0));
    // `vertical-lr`: the block start is the left side
    assert_eq!(bbox(&dump, "b").0, 10.0);
}

#[test]
fn physical_and_logical_properties_share_the_cascade() {
    let dump = layout(
        "cascade",
        "<div id=a style='margin-left:50px;margin-inline-start:10px'></div>\
         <div id=b style='margin-inline-start:10px;margin-left:50px'></div>",
    );
    assert_eq!(bbox(&dump, "a").0, 10.0);
    assert_eq!(bbox(&dump, "b").0, 50.0);
}

#[test]
fn logical_sizes_are_the_width_and_the_height_of_a_horizontal_box() {
    let dump = layout(
        "sizes",
        "<div id=a style='inline-size:40px;block-size:7px'></div>\
         <div id=b style='writing-mode:vertical-lr;inline-size:40px;block-size:70px'></div>\
         <div id=c style='max-inline-size:30px;min-block-size:50px'></div>",
    );
    assert_eq!(bbox(&dump, "a"), (0.0, 0.0, 40.0, 7.0));
    // the inline size of a vertical box is its height
    let b = bbox(&dump, "b");
    assert_eq!((b.2, b.3), (70.0, 40.0));
    let c = bbox(&dump, "c");
    assert_eq!((c.2, c.3), (30.0, 50.0));
}

const POSITIONED: &str = "<style>.p{position:relative;width:300px;height:100px;margin-bottom:10px} .p>div{position:absolute}</style>";

#[test]
fn inset_takes_one_to_four_values_like_margin() {
    let dump = layout(
        "inset",
        &format!(
            "{POSITIONED}<div class=p id=p1><div id=a style='inset:10px;width:auto;height:auto'></div></div>\
             <div class=p id=p2><div id=b style='inset:10px 20px;width:auto;height:auto'></div></div>\
             <div class=p id=p3><div id=c style='inset:5px 10px 15px;width:auto;height:auto'></div></div>\
             <div class=p id=p4><div id=d style='inset:5px 10px 15px 20px;width:auto;height:auto'></div></div>\
             <div class=p id=p5><div id=e style='top:7px;inset:10px 20px;width:auto;height:auto'></div></div>"
        ),
    );
    let rel = |id: &str, parent: &str| {
        let (x, y, w, h) = bbox(&dump, id);
        let p = bbox(&dump, parent);
        (x - p.0, y - p.1, w, h)
    };
    assert_eq!(rel("a", "p1"), (10.0, 10.0, 280.0, 80.0));
    assert_eq!(rel("b", "p2"), (20.0, 10.0, 260.0, 80.0));
    assert_eq!(rel("c", "p3"), (10.0, 5.0, 280.0, 80.0));
    assert_eq!(rel("d", "p4"), (20.0, 5.0, 270.0, 80.0));
    // a later `inset` is stronger than an earlier `top`
    assert_eq!(rel("e", "p5"), (20.0, 10.0, 260.0, 80.0));
}

#[test]
fn logical_insets_place_positioned_boxes() {
    let dump = layout(
        "logical-inset",
        &format!(
            "{POSITIONED}<div class=p id=p1><div id=a style='inset-inline-start:15px;inset-block-start:2px'></div></div>\
             <div class=p id=p2 dir=rtl><div id=b style='inset-inline-start:15px;inset-block-end:4px'></div></div>\
             <div class=p id=p3><div id=c style='inset-inline:10px 20px;inset-block:5px 15px;width:auto;height:auto'></div></div>"
        ),
    );
    let rel = |id: &str, parent: &str| {
        let (x, y, w, h) = bbox(&dump, id);
        let p = bbox(&dump, parent);
        (x - p.0, y - p.1, w, h)
    };
    assert_eq!(rel("a", "p1"), (15.0, 2.0, 100.0, 20.0));
    // right-to-left: the start side is the right one; the block end is the bottom
    assert_eq!(rel("b", "p2"), (300.0 - 15.0 - 100.0, 100.0 - 4.0 - 20.0, 100.0, 20.0));
    assert_eq!(rel("c", "p3"), (10.0, 5.0, 270.0, 80.0));
}

#[test]
fn order_puts_the_items_of_a_flex_container_in_order() {
    let dump = layout(
        "order",
        "<style>.c{display:flex;width:300px;margin-bottom:10px} .c>div{width:50px}</style>\
         <div class=c id=a><div id=a1 style='order:2'></div><div id=a2></div><div id=a3 style='order:-1'></div><div id=a4 style='order:2'></div></div>\
         <div class=c id=b style='flex-direction:column'><div id=b1 style='order:1'></div><div id=b2></div></div>\
         <div class=c id=c style='flex-direction:row-reverse'><div id=c1 style='order:1'></div><div id=c2></div></div>",
    );
    let x = |id| bbox(&dump, id).0;
    // by `order`, and in the order of the document for the same value
    assert_eq!((x("a3"), x("a2"), x("a1"), x("a4")), (0.0, 50.0, 100.0, 150.0));
    // the column: the second item comes first
    assert!(bbox(&dump, "b2").1 < bbox(&dump, "b1").1);
    // the main axis is reversed after the items are in order
    assert_eq!((x("c2"), x("c1")), (250.0, 200.0));
}

#[test]
fn order_puts_the_items_of_a_grid_in_order_and_leaves_positioned_boxes_alone() {
    let dump = layout(
        "grid-order",
        "<style>.g{display:grid;grid-template-columns:50px 50px 50px;width:300px;margin-bottom:10px} .g>div{width:auto}</style>\
         <div class=g id=g><div id=g1 style='order:3'></div><div id=g2></div><div id=g3 style='order:1'></div></div>\
         <div class=g id=h style='position:relative'><div id=h1 style='order:5'></div><div id=h2 style='position:absolute;order:-3;width:10px'></div><div id=h3></div></div>",
    );
    let x = |id| bbox(&dump, id).0;
    assert_eq!((x("g2"), x("g3"), x("g1")), (0.0, 50.0, 100.0));
    // `order` does not make an absolutely positioned box an item
    assert_eq!((x("h3"), x("h1")), (0.0, 50.0));
}
