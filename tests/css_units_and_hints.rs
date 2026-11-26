//! Units that depend on the font or the page (`ex`, `ch`, `vw`, `vh`), what the HTML attributes
//! `width`, `height`, `cellspacing` and `cellpadding` mean, the initial border width, and the
//! values of `justify-self`: the sizes of the boxes show what the styles are.

mod common;

use std::path::Path;

/// The border boxes `(x, y, width, height)` of the elements with an `id`, by id. The page has
/// no margin and the body none, so the boxes are measured from the corner of the page.
fn boxes(name: &str, html: &str) -> std::collections::HashMap<String, (f64, f64, f64, f64)> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("units_hints__{}.html", name));
    let output = dir.join(format!("units_hints__{}.pdf", name));
    let layout = dir.join(format!("units_hints__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>@page{{size:800px 600px;margin:0}} body{{margin:0}}</style>{}", html),
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
    fn collect(node: &serde_json::Value, out: &mut std::collections::HashMap<String, (f64, f64, f64, f64)>) {
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

#[test]
fn ex_and_ch_come_from_the_font() {
    // Ahem has an x-height of 0.8 em and a `0` that is 1 em wide
    let b = boxes(
        "ex_ch",
        "<div id=a style='font:20px Ahem;width:2ex;height:3ch'></div>",
    );
    assert_eq!(b["a"].2, 32.0);
    assert_eq!(b["a"].3, 60.0);
}

#[test]
fn viewport_units_are_relative_to_the_page_area() {
    let b = boxes(
        "viewport",
        "<div id=a style='width:10vw;height:50vh'></div><div id=b style='width:10vmin;height:10vmax'></div>",
    );
    assert_eq!((b["a"].2, b["a"].3), (80.0, 300.0));
    assert_eq!((b["b"].2, b["b"].3), (60.0, 80.0));
}

#[test]
fn a_negative_padding_or_size_is_not_valid() {
    let b = boxes(
        "negative",
        "<div id=a style='padding-left:-5px;width:100px;height:10px'></div><div id=b style='width:-5px;height:10px'></div>",
    );
    assert_eq!(b["a"].2, 100.0);
    // the width is `auto`
    assert_eq!(b["b"].2, 800.0);
}

#[test]
fn the_initial_border_width_is_medium() {
    let b = boxes("medium", "<div id=a style='border-left-style:solid;border-left-width:-1px;width:10px;height:10px'></div>");
    assert_eq!(b["a"].2, 13.0);
}

#[test]
fn percentages_of_image_attributes_are_hints() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::copy(
        common::wpt_root().join("css/CSS2/normal-flow/support/black96x96.png"),
        dir.join("units_hints__black.png"),
    )
    .unwrap();
    let b = boxes(
        "img_percent",
        "<div style='width:400px'><img id=a src='units_hints__black.png' width='50%' height='20' style='display:block'></div>",
    );
    assert_eq!((b["a"].2, b["a"].3), (200.0, 20.0));
}

#[test]
fn cellspacing_and_cellpadding_set_the_spacing_and_padding_of_the_cells() {
    let b = boxes(
        "cells",
        "<table cellspacing=0 cellpadding=5><tr><td id=a><div style='width:20px;height:20px'></div></td></tr></table>",
    );
    assert_eq!((b["a"].2, b["a"].3), (30.0, 30.0));
}

#[test]
fn an_iframe_is_an_empty_box_of_300_by_150() {
    let b = boxes("iframe", "<iframe id=a style='display:block;border:0'></iframe>");
    assert_eq!((b["a"].2, b["a"].3), (300.0, 150.0));
}

#[test]
fn self_start_and_right_align_a_grid_item_to_the_start_and_the_end() {
    let b = boxes(
        "self",
        "<div style='display:grid;width:100px;grid-template-columns:100px'>\
         <div id=a style='justify-self:right;width:10px;height:10px'></div>\
         <div id=b style='justify-self:self-end;width:10px;height:10px'></div></div>",
    );
    assert_eq!(b["a"].0, 90.0);
    assert_eq!(b["b"].0, 90.0);
}
