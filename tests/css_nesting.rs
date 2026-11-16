//! Nested style rules and `@media` blocks: `&`, implicit descendants and the order of the cascade.
mod common;

use std::path::Path;

/// The widths and heights (in CSS pixels) of the `<div>` elements of `html`, in tree order.
fn boxes(name: &str, html: &str) -> Vec<(f64, f64)> {
    boxes_and_log(name, html).0
}

fn boxes_and_log(name: &str, html: &str) -> (Vec<(f64, f64)>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("css_nesting__{}.html", name));
    let output = dir.join(format!("css_nesting__{}.pdf", name));
    let layout = dir.join(format!("css_nesting__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
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
fn a_nested_rule_without_ampersand_is_a_descendant() {
    let w = widths(
        "descendant",
        "<style>.a{width:10px; div{width:20px}}</style>\
         <div class=a><div></div></div>",
    );
    assert_eq!(w, [10.0, 20.0]);
}

#[test]
fn the_ampersand_stands_for_the_parent() {
    let w = widths(
        "ampersand",
        "<style>div{width:10px; &.b{width:20px} .c &{width:30px}}</style>\
         <div></div><div class=b></div><section class=c><div></div></section>",
    );
    assert_eq!(w, [10.0, 20.0, 30.0]);
}

#[test]
fn a_nested_selector_may_start_with_a_combinator() {
    let w = widths(
        "combinator",
        "<style>.a{width:10px; > div{width:20px} + div{width:30px}}</style>\
         <div class=a><div><div></div></div></div><div></div>",
    );
    // `.a > div` is the first inner box; `.a + div` is the box after it
    assert_eq!((w[0], w[1], w[3]), (10.0, 20.0, 30.0));
}

#[test]
fn a_selector_list_of_the_parent_applies_to_all_of_its_selectors() {
    let w = widths(
        "list",
        "<style>.a, .b{ & > div{width:10px} }</style>\
         <section class=a><div></div></section><section class=b><div></div></section><section><div></div></section>",
    );
    assert_eq!(w[0], 10.0);
    assert_eq!(w[1], 10.0);
    assert_ne!(w[2], 10.0);
}

#[test]
fn nested_rules_can_be_nested_again() {
    let w = widths(
        "deep",
        "<style>.a{ .b{ width:10px; &:first-child{width:20px} } }</style>\
         <div class=a><div class=b></div><div class=b></div></div>",
    );
    assert_eq!(&w[1..], [20.0, 10.0]);
}

#[test]
fn the_specificity_of_a_nested_rule_includes_the_parent() {
    // `.a div` (0,1,1) beats `div` (0,0,1) although it comes first
    let w = widths(
        "specificity",
        "<style>.a{ div{width:10px} } div{width:99px}</style>\
         <div class=a><div></div></div>",
    );
    assert_eq!(w[1], 10.0);
}

#[test]
fn a_declaration_after_a_nested_rule_comes_after_it_in_the_cascade() {
    let w = widths(
        "order",
        "<style>div{ & {width:10px} width:20px } </style><div></div>",
    );
    assert_eq!(w, [20.0]);
    let w = widths(
        "order_before",
        "<style>div{ width:20px; & {width:10px} } </style><div></div>",
    );
    assert_eq!(w, [10.0]);
}

#[test]
fn a_media_block_in_a_rule_applies_its_declarations_to_the_rule() {
    let w = widths(
        "media",
        "<style>div{ width:10px; @media print { width:20px } @media (max-width: 1px) { width:30px; } }</style><div></div>",
    );
    assert_eq!(w, [20.0]);
}

#[test]
fn a_nested_rule_in_a_media_block() {
    let w = widths(
        "media_outside",
        "<style>@media print { .a { div { width:10px } } }</style><div class=a><div></div></div>",
    );
    assert_eq!(w[1], 10.0);
}

#[test]
fn a_nested_rule_that_looks_like_a_declaration() {
    let w = widths(
        "pseudo_class",
        "<style>div{ width:10px; div:first-child{width:20px} }</style><div><div></div></div>",
    );
    assert_eq!(w, [10.0, 20.0]);
}

#[test]
fn an_invalid_nested_rule_does_not_remove_the_rest() {
    let (w, log) = boxes_and_log(
        "invalid",
        "<style>div{ width:10px; &:unknown-thing{width:99px} height:5px }</style><div></div>",
    );
    assert_eq!(w, [(10.0, 5.0)]);
    assert!(log.contains("Could not") || log.contains("Error"), "{log}");
}

#[test]
fn nesting_is_not_for_the_style_attribute() {
    let w = widths("attribute", "<div style=\"width:10px; &{width:20px}\"></div>");
    assert_eq!(w, [10.0]);
}
