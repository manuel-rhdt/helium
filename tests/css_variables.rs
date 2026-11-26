//! Custom properties (`--name`) and `var()`.
mod common;

use std::path::Path;

/// The widths and heights (in CSS pixels) of the `<div>` elements of `html`, in tree order.
fn boxes(name: &str, html: &str) -> Vec<(f64, f64)> {
    boxes_and_log(name, html).0
}

fn boxes_and_log(name: &str, html: &str) -> (Vec<(f64, f64)>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("css_variables__{}.html", name));
    let output = dir.join(format!("css_variables__{}.pdf", name));
    let layout = dir.join(format!("css_variables__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    let dump: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    fn collect(node: &serde_json::Value, out: &mut Vec<(f64, f64)>) {
        if node["tag"] == "div" {
            let border_box = &node["border_box"];
            out.push((border_box["width"].as_f64().unwrap(), border_box["height"].as_f64().unwrap()));
        }
        for child in node["children"].as_array().into_iter().flatten() {
            collect(child, out);
        }
    }
    let mut out = vec![];
    collect(&dump["document"], &mut out);
    (out, log)
}

fn widths(name: &str, html: &str) -> Vec<f64> {
    boxes(name, html).into_iter().map(|(width, _)| width).collect()
}

#[test]
fn a_property_takes_the_value_of_a_custom_property() {
    let w = widths("basic", "<style>div{--w: 40px; width: var(--w)}</style><div></div>");
    assert_eq!(w, [40.0]);
}

#[test]
fn custom_properties_are_inherited() {
    let w = widths(
        "inherited",
        "<style>.a{--w: 40px} div{width: var(--w)}</style><div class=a><div></div></div>",
    );
    assert_eq!(w[1], 40.0);
}

#[test]
fn a_custom_property_can_be_overridden_by_a_descendant() {
    let w = widths(
        "override",
        "<style>div{width: var(--w)} .a{--w: 40px} .b{--w: 60px}</style>\
         <div class=a><div></div><div class=b><div></div></div></div>",
    );
    assert_eq!(&w[1..], [40.0, 60.0, 60.0]);
}

#[test]
fn the_value_is_substituted_where_it_is_used() {
    // `em` is relative to the element that uses the value, not the one that declares it
    let w = widths(
        "em",
        "<style>.a{--w: 2em; font-size:10px} .b{font-size:20px; width: var(--w)}</style>\
         <div class=a><div class=b></div></div>",
    );
    assert_eq!(w[1], 40.0);
}

#[test]
fn the_fallback_is_used_for_a_missing_property() {
    let w = widths(
        "fallback",
        "<style>div{width: var(--missing, 30px)} .a{--w: 50px; width: var(--w, 30px)}</style>\
         <div></div><div class=a></div>",
    );
    assert_eq!(w, [30.0, 50.0]);
}

#[test]
fn the_fallback_may_use_var_and_commas() {
    let w = widths(
        "fallback_var",
        "<style>div{--b: 70px; width: var(--missing, var(--b))} .c{margin-left: var(--m, 5px)}</style><div></div>",
    );
    assert_eq!(w, [70.0]);
}

#[test]
fn a_custom_property_can_use_another() {
    let w = widths(
        "chain",
        "<style>div{--a: 10px; --b: var(--a); --c: var(--b); width: var(--c)}</style><div></div>",
    );
    assert_eq!(w, [10.0]);
}

#[test]
fn custom_properties_that_depend_on_each_other_are_invalid() {
    let w = widths(
        "cycle",
        "<style>div{--a: var(--b); --b: var(--a); width: var(--a, 10px)}</style><div></div>",
    );
    // the custom properties are invalid, so the fallback is used
    assert_eq!(w, [10.0]);
}

#[test]
fn a_value_that_cannot_be_used_unsets_the_property() {
    // the invalid declaration is `unset`, so the width is `auto`, not the 50px before it
    let (w, _) = boxes_and_log(
        "invalid_value",
        "<style>div{width:50px} div{--w: red; width: var(--w)}</style><div></div>",
    );
    assert_ne!(w[0].0, 50.0);
    let w = widths(
        "missing",
        "<style>div{width: 20px} div{width: var(--undefined)}</style><div></div>",
    );
    assert_ne!(w[0], 20.0);
}

#[test]
fn shorthands_can_use_var() {
    let (b, _) = boxes_and_log(
        "shorthand",
        "<style>div{--m: 10px 20px; --p: 5px; margin: var(--m); padding: var(--p) var(--p); width: 100px}</style><div></div>",
    );
    assert_eq!(b[0].0, 110.0);
}

#[test]
fn important_applies_to_declarations_with_var() {
    let w = widths(
        "important",
        "<style>div{--w: 40px; width: var(--w) !important} #a{width: 99px}</style><div id=a></div>",
    );
    assert_eq!(w, [40.0]);
}

#[test]
fn the_style_attribute_sets_and_uses_custom_properties() {
    let w = widths(
        "attribute",
        "<style>div{width: var(--w, 10px)}</style><div style=\"--w: 33px\"></div><div></div>",
    );
    assert_eq!(w, [33.0, 10.0]);
}

#[test]
fn the_font_size_can_come_from_var() {
    let w = widths(
        "font_size",
        "<style>div{--s: 20px; font-size: var(--s); width: 2em}</style><div></div>",
    );
    assert_eq!(w, [40.0]);
}

#[test]
fn custom_properties_work_with_nesting() {
    let w = widths(
        "nesting",
        "<style>.a{--w: 10px; div{width: var(--w)} &.b{--w: 20px} }</style>\
         <div class=a><div></div></div><div class='a b'><div></div></div>",
    );
    assert_eq!(w[1], 10.0);
    assert_eq!(w[3], 20.0);
}

#[test]
fn custom_properties_are_case_sensitive() {
    let w = widths(
        "case",
        "<style>div{--W: 10px; width: var(--w, 20px)}</style><div></div>",
    );
    assert_eq!(w, [20.0]);
}

#[test]
fn an_unknown_property_with_var_is_reported() {
    let (_, log) = boxes_and_log(
        "unknown_property",
        "<style>div{--x: 1px; frobnicate: var(--x)}</style><div></div>",
    );
    assert!(log.contains("frobnicate"), "{log}");
}
