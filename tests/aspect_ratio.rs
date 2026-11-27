//! `aspect-ratio`, and the size that the ratio of an image or a `<canvas>` gives to a box with a
//! size on one side only.

mod common;

use serde_json::Value;
use std::path::Path;

fn layout(name: &str, html: &str) -> Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("aspect_ratio__{}.html", name));
    let output = dir.join(format!("aspect_ratio__{}.pdf", name));
    let json = dir.join(format!("aspect_ratio__{}.json", name));
    // the images are 100x100 and 10x10 pixels
    std::fs::write(
        &input,
        format!("<style>@page{{size:700px 500px;margin:0}} body{{margin:0}}</style>{}", html),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--resource-root"])
        .arg(common::wpt_root())
        .arg("--dump-layout")
        .arg(&json)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_str(&std::fs::read_to_string(json).unwrap()).unwrap()
}

fn size(dump: &Value, id: &str) -> (f64, f64) {
    fn find(node: &Value, id: &str) -> Option<(f64, f64)> {
        if node["id"] == id {
            let b = &node["border_box"];
            return Some((b["width"].as_f64()?, b["height"].as_f64()?));
        }
        node["children"].as_array()?.iter().find_map(|child| find(child, id))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
}

fn image() -> String {
    common::wpt_root().join("css/css-flexbox/support/100x100-blue.png").display().to_string()
}

fn small() -> String {
    common::wpt_root().join("css/css-flexbox/support/10x10-green.png").display().to_string()
}

#[test]
fn a_ratio_gives_the_height_from_the_width() {
    let dump = layout(
        "height",
        "<div id=a style='width:100px;aspect-ratio:2/1'></div>\
         <div id=b style='width:320px;aspect-ratio:1.6'></div>\
         <div id=c style='width:100px;aspect-ratio:0.00025 / 0.0001'></div>",
    );
    assert_eq!(size(&dump, "a"), (100.0, 50.0));
    assert_eq!(size(&dump, "b"), (320.0, 200.0));
    assert_eq!(size(&dump, "c"), (100.0, 40.0));
}

#[test]
fn a_ratio_gives_the_width_from_the_height_of_a_box_that_is_not_stretched() {
    let dump = layout(
        "width",
        "<div style='display:flex'><div id=a style='height:50px;aspect-ratio:2'></div></div>",
    );
    assert_eq!(size(&dump, "a"), (100.0, 50.0));
}

#[test]
fn auto_and_degenerate_ratios_give_nothing() {
    let dump = layout(
        "auto",
        "<div id=a style='width:100px;aspect-ratio:auto'></div>\
         <div id=b style='width:100px;aspect-ratio:0 / 1'></div>\
         <div id=c style='width:100px;aspect-ratio:auto 4/1'></div>\
         <div id=d style='width:100px;aspect-ratio:1/1;height:30px'></div>",
    );
    assert_eq!(size(&dump, "a"), (100.0, 0.0));
    assert_eq!(size(&dump, "b"), (100.0, 0.0));
    // `auto <ratio>`: the ratio of a box without a content ratio
    assert_eq!(size(&dump, "c"), (100.0, 25.0));
    // a length on both sides: nothing is left to give
    assert_eq!(size(&dump, "d"), (100.0, 30.0));
}

#[test]
fn an_invalid_ratio_is_ignored() {
    let dump = layout("invalid", "<div id=a style='width:100px;aspect-ratio:2/1;aspect-ratio:wide'></div>");
    assert_eq!(size(&dump, "a"), (100.0, 50.0));
}

#[test]
fn an_image_keeps_its_own_ratio_unless_the_ratio_is_given_without_auto() {
    let dump = layout(
        "image",
        &format!(
            "<img id=a src='{}' style='width:50px'>\
             <img id=b src='{}' style='width:50px;aspect-ratio:2/1'>\
             <img id=c src='{}' style='width:50px;aspect-ratio:auto 2/1'>",
            image(),
            image(),
            image()
        ),
    );
    assert_eq!(size(&dump, "a"), (50.0, 50.0));
    assert_eq!(size(&dump, "b"), (50.0, 25.0));
    assert_eq!(size(&dump, "c"), (50.0, 50.0));
}

#[test]
fn flex_items_that_are_images_get_their_height_from_the_width_they_are_given() {
    let dump = layout(
        "flex-images",
        &format!(
            "<div style='display:flex;width:600px'>\
             <img id=a src='{}' style='flex:1 0 auto;min-width:0'>\
             <img id=b src='{}' style='flex:1 0 auto;min-width:0'></div>",
            image(),
            small()
        ),
    );
    // 490 pixels are left over, 245 for each; the line is as high as the highest item and the
    // other one is stretched to it
    assert_eq!(size(&dump, "a"), (345.0, 345.0));
    assert_eq!(size(&dump, "b"), (255.0, 345.0));
}

#[test]
fn a_canvas_is_a_replaced_box() {
    let dump = layout(
        "canvas",
        "<canvas id=a></canvas><canvas id=b width=40 height=20></canvas>\
         <canvas id=c width=40 height=20 style='width:100px'></canvas>\
         <div style='display:flex'><canvas id=d width=40 height=20></canvas></div>",
    );
    assert_eq!(size(&dump, "a"), (300.0, 150.0));
    assert_eq!(size(&dump, "b"), (40.0, 20.0));
    // the other side follows the ratio of the size of the canvas
    assert_eq!(size(&dump, "c"), (100.0, 50.0));
    assert_eq!(size(&dump, "d"), (40.0, 20.0));
}

#[test]
fn limits_on_an_image_keep_its_proportions() {
    // the usual `img { max-width: 100%; height: auto }` and its relatives, in a box 200 wide
    let case = |style: &str| {
        let dump = layout(
            "limits",
            &format!("<div style='width:200px'><img id=a src='{}' style='{}'></div>", image(), style),
        );
        size(&dump, "a")
    };
    assert_eq!(case("width:100%"), (200.0, 200.0));
    assert_eq!(case("width:300px;max-width:100%;height:auto"), (200.0, 200.0));
    assert_eq!(case("width:300px;max-width:200px"), (200.0, 200.0));
    assert_eq!(case("height:300px;max-height:80px"), (80.0, 80.0));
    assert_eq!(case("width:20px;min-width:150px"), (150.0, 150.0));
    assert_eq!(case("min-width:150px"), (150.0, 150.0));
    assert_eq!(case("max-height:40px"), (40.0, 40.0));
    // limits on both sides: the one that is further off wins
    assert_eq!(case("width:300px;height:300px;max-width:200px;max-height:250px"), (200.0, 250.0));
    // the limits are for the content box: the padding and border come on top
    assert_eq!(case("width:300px;max-width:100%;padding:3px;border:2px solid"), (210.0, 210.0));
}

#[test]
fn limits_apply_to_the_box_that_box_sizing_names() {
    let dump = layout(
        "border-box",
        &format!(
            "<div style='width:200px'><img id=a src='{}' style='box-sizing:border-box;width:300px;max-width:100%;padding:5px;border:5px solid'></div>",
            image()
        ),
    );
    // the border box is 200 wide: 170 of content plus 30 of padding and border, and as high
    assert_eq!(size(&dump, "a").0, 200.0);
    assert_eq!(size(&dump, "a").1, 200.0);
}

#[test]
fn a_stretched_image_with_a_border_gets_its_width_from_the_height_without_the_border_twice() {
    // The width of the content box is the stretched height (the ratio is 1); the border on the
    // right comes on top of it once.
    let dump = layout(
        "stretched_border",
        &format!(
            "<div style='display:flex;width:0;height:120px'>\
             <img id=a src='{0}' style='border-right:10px solid;margin-bottom:20px'></div>\
             <div style='display:flex;width:0;height:120px'>\
             <img id=b src='{0}' style='border-right:10px solid'></div>",
            image()
        ),
    );
    assert_eq!(size(&dump, "a"), (110.0, 100.0));
    assert_eq!(size(&dump, "b"), (130.0, 120.0));
}

#[test]
fn an_svg_element_has_a_size_of_300_by_150_unless_a_view_box_gives_a_ratio() {
    let dump = layout(
        "svg_sizes",
        "<svg id=a></svg><svg id=b width=40></svg><svg id=c height=40></svg>\
         <svg id=d width=40 height=20></svg><svg id=e height=40 viewBox='0 0 4 1'></svg>\
         <div style='width:200px'><svg id=f viewBox='0 0 4 1'></svg></div>\
         <svg id=g style='height:45px'></svg>",
    );
    assert_eq!(size(&dump, "a"), (300.0, 150.0));
    assert_eq!(size(&dump, "b"), (40.0, 150.0));
    assert_eq!(size(&dump, "c"), (300.0, 40.0));
    assert_eq!(size(&dump, "d"), (40.0, 20.0));
    assert_eq!(size(&dump, "e"), (160.0, 40.0));
    // a viewBox alone is a ratio: the width of the line, the height from it
    assert_eq!(size(&dump, "f"), (200.0, 50.0));
    // without a viewBox there is no ratio either
    assert_eq!(size(&dump, "g"), (300.0, 45.0));
}
