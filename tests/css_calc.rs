//! Math functions (`calc()`, `min()`, `max()`, `clamp()`, ...) in the values of properties: the
//! sizes of the boxes show what the values come to.
//! https://drafts.csswg.org/css-values-4/#math

mod common;

use std::collections::HashMap;
use std::path::Path;

/// The border boxes `(x, y, width, height)` of the elements with an `id`, by id. The page is
/// 800px by 600px with no margin, and the body has none.
fn boxes(name: &str, html: &str) -> HashMap<String, (f64, f64, f64, f64)> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("css_calc__{}.html", name));
    let output = dir.join(format!("css_calc__{}.pdf", name));
    let layout = dir.join(format!("css_calc__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>@page{{size:800px 600px;margin:0}} body{{margin:0;font:16px Ahem}}</style>{}", html),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .args(["--no-system-fonts", "--font-dir"])
        .arg(common::wpt_fonts())
        .arg("--dump-layout")
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let dump: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    fn collect(node: &serde_json::Value, out: &mut HashMap<String, (f64, f64, f64, f64)>) {
        if let Some(id) = node["id"].as_str() {
            let b = &node["border_box"];
            out.insert(
                id.to_owned(),
                (b["x"].as_f64().unwrap(), b["y"].as_f64().unwrap(), b["width"].as_f64().unwrap(), b["height"].as_f64().unwrap()),
            );
        }
        for child in node["children"].as_array().into_iter().flatten() {
            collect(child, out);
        }
    }
    let mut out = Default::default();
    collect(&dump["document"], &mut out);
    out
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn lengths_and_percentages() {
    let b = boxes(
        "lengths_and_percentages",
        "<div id=a style='width:calc(100% - 2em);height:calc(10vh + 4px)'></div>\
         <div id=b style='width:calc((100% - 100px) / 2);height:calc(1in - 2 * 8px)'></div>\
         <div style='width:400px'><div id=c style='width:calc(50% + 1em);margin-left:calc(25% - 10px);height:1px'></div></div>",
    );
    assert_eq!(b["a"].2, 768.0);
    assert_eq!(b["a"].3, 64.0);
    assert_eq!(b["b"].2, 350.0);
    assert_eq!(b["b"].3, 80.0);
    assert_eq!(b["c"].2, 216.0);
    assert_eq!(b["c"].0, 90.0);
}

#[test]
fn comparison_functions_keep_the_percentage() {
    // `min(100%, 300px)` cannot be written as a length plus a percentage: layout evaluates it
    let b = boxes(
        "comparison",
        "<div style='width:500px'><div id=a style='width:min(100%, 300px);height:1px'></div></div>\
         <div style='width:200px'><div id=b style='width:min(100%, 300px);height:1px'></div></div>\
         <div id=c style='width:clamp(100px, 50%, 300px);height:1px'></div>\
         <div style='width:100px'><div id=d style='width:clamp(100px, 10%, 300px);height:1px'></div></div>\
         <div id=e style='width:max(10%, 5em, 20px);height:max(1px, 2px)'></div>",
    );
    assert_eq!(b["a"].2, 300.0);
    assert_eq!(b["b"].2, 200.0);
    assert_eq!(b["c"].2, 300.0);
    assert_eq!(b["d"].2, 100.0);
    assert_eq!(b["e"].2, 80.0);
    assert_eq!(b["e"].3, 2.0);
}

#[test]
fn a_negative_result_is_clamped_where_the_property_is_not_negative() {
    // `padding: calc(50% - 300px)` is -100px of a 400px container: 0, but `margin` may be negative
    let b = boxes(
        "clamped",
        "<div style='width:400px'>\
           <div id=a style='padding-left:calc(50% - 300px);width:100px;height:1px'></div>\
           <div id=b style='padding-left:calc(100% - 300px);width:100px;height:1px'></div>\
           <div id=c style='margin-left:calc(50% - 300px);width:100px;height:1px'></div>\
           <div id=d style='width:calc(10px - 100%);height:calc(-5px)'></div>\
         </div>",
    );
    assert_eq!(b["a"].2, 100.0);
    assert_eq!(b["b"].2, 200.0);
    assert_eq!(b["c"].0, -100.0);
    assert_eq!((b["d"].2, b["d"].3), (0.0, 0.0));
}

#[test]
fn non_negative_shorthands_logical_properties_and_numbers() {
    // the range comes with the property, also through shorthands and logical properties; a
    // negative literal is not valid where a math function is clamped
    let b = boxes(
        "non_negative_kinds",
        "<div style='width:400px'>\
           <div id=a style='padding:calc(10px - 50%) 0;padding-inline-start:calc(20px - 100%);width:100px;height:10px'></div>\
           <div id=b style='inline-size:calc(100px - 100%);block-size:10px'></div>\
         </div>\
         <div style='display:flex;width:400px'>\
           <div id=f1 style='flex:calc(-1) 1 0px;height:10px'></div>\
           <div id=f2 style='flex-grow:1;flex-grow:-1;flex-basis:0;height:10px'></div>\
         </div>\
         <div id=l style='font:10px/calc(-2) Ahem;line-height:calc(1 + 1)'>X</div>",
    );
    assert_eq!((b["a"].2, b["a"].3), (100.0, 10.0));
    assert_eq!(b["b"].2, 0.0);
    // `flex-grow: calc(-1)` is 0; `flex-grow: -1` is ignored, so the item takes all the space
    assert_eq!(b["f1"].2, 0.0);
    assert_eq!(b["f2"].2, 400.0);
    assert_eq!(b["l"].3, 20.0);
}

#[test]
fn positioned_and_grid_boxes() {
    let b = boxes(
        "positioned_grid",
        "<div style='position:relative;width:400px;height:200px'>\
           <div id=a style='position:absolute;left:calc(50% - 50px);top:calc(25% + 1em);width:100px;height:10px'></div>\
         </div>\
         <div style='display:grid;width:400px;grid-template-columns:calc(50% - 10px) 1fr;column-gap:calc(2 * 10px)'>\
           <div id=g1 style='height:10px'></div><div id=g2 style='height:10px'></div>\
         </div>\
         <div style='display:flex;width:400px'><div id=f style='flex:0 0 min(50%, 150px);height:10px'></div></div>",
    );
    assert_eq!((b["a"].0, b["a"].1), (150.0, 66.0));
    assert_eq!(b["g1"].2, 190.0);
    assert_eq!(b["g2"].0, 210.0);
    assert_eq!(b["g2"].2, 190.0);
    assert_eq!(b["f"].2, 150.0);
}

#[test]
fn fonts_and_borders() {
    // the glyphs of Ahem are squares of the font size; percentages of `font-size` are of the
    // font size of the parent
    let b = boxes(
        "fonts_borders",
        "<div style='font-size:calc(1em + 4px)'><span id=a style='display:inline-block'>X</span></div>\
         <div style='font-size:calc(50% + 2px)'><span id=b style='display:inline-block'>X</span></div>\
         <div id=c style='border:calc(1px + 0.125em) solid;width:10px;height:10px'></div>\
         <div id=d style='width:calc(2px * 3px / 1px + 1em * 0);height:round(up, 17px, 5px)'></div>",
    );
    assert_eq!(b["a"].2, 20.0);
    assert_eq!(b["b"].2, 10.0);
    assert_eq!(b["c"].2, 16.0);
    assert_eq!(b["d"].2, 6.0);
    assert_eq!(b["d"].3, 20.0);
}

#[test]
fn invalid_math_functions_are_ignored() {
    // a length plus a number, a missing space around `+` and a percentage of `border-width` are
    // not valid: the declarations before them stay
    let b = boxes(
        "invalid",
        "<div id=a style='width:100px;width:calc(100px + 5);height:10px'></div>\
         <div id=b style='width:100px;width:calc(100px+5px);height:10px'></div>\
         <div id=c style='border:1px solid;border-width:calc(10% + 1px);width:10px;height:10px'></div>",
    );
    assert_eq!(b["a"].2, 100.0);
    assert_eq!(b["b"].2, 100.0);
    assert_eq!(b["c"].2, 12.0);
}

#[test]
fn custom_properties_in_math_functions() {
    let b = boxes(
        "custom_properties",
        "<div style='--w:50px;--n:3'><div id=a style='width:calc(var(--w) * var(--n));height:calc(var(--w) / 5)'></div></div>",
    );
    assert_eq!(b["a"].2, 150.0);
    assert!(approx(b["a"].3, 10.0));
}
