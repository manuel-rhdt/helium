//! `!important`, the origins of the cascade, `rem` and `@media`: the declarations that win show
//! in the size of the boxes.

mod common;

use std::path::Path;

/// The widths and heights (in CSS pixels) of the `<div>` elements of `html`, in tree order.
fn boxes(name: &str, html: &str) -> Vec<(f64, f64)> {
    boxes_and_log(name, html).0
}

fn boxes_and_log(name: &str, html: &str) -> (Vec<(f64, f64)>, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("css_cascade__{}.html", name));
    let output = dir.join(format!("css_cascade__{}.pdf", name));
    let layout = dir.join(format!("css_cascade__{}.json", name));
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
fn an_important_declaration_beats_one_of_a_more_specific_rule() {
    let w = widths(
        "important_specificity",
        "<style>div{width:100px !important} #a{width:200px} .b{width:300px}</style>\
         <div id=a></div><div class=b></div><div></div>",
    );
    assert_eq!(w, [100.0, 100.0, 100.0]);
}

#[test]
fn an_important_declaration_beats_the_style_attribute() {
    let w = widths(
        "important_inline",
        "<style>div{width:100px !important}</style><div style=\"width:200px\"></div>\
         <div style=\"width:200px !important\"></div>",
    );
    // the one of the style attribute wins over an important rule only if it is important too, and
    // then it is the later (more specific) one
    assert_eq!(w, [100.0, 200.0]);
}

#[test]
fn of_two_important_declarations_the_more_specific_wins() {
    let w = widths(
        "important_both",
        "<style>div{width:100px !important} #a{width:200px !important} \
         #a{width:300px !important}</style><div id=a></div><div></div>",
    );
    assert_eq!(w, [300.0, 100.0]);
}

#[test]
fn important_is_matched_in_any_case_and_with_spaces() {
    let w = widths(
        "important_spelling",
        "<style>div{width:100px ! IMPORTANT} #a{width:200px}</style><div id=a></div>\
         <style>#b{height:10px!important} div{height:20px}</style><div id=b></div>",
    );
    assert_eq!(w[0], 100.0);
}

#[test]
fn an_important_declaration_is_not_a_syntax_error() {
    let (_, log) = boxes_and_log(
        "important_log",
        "<style>div{margin:0 !important;color:red!important;width:50px !important}</style><div></div>",
    );
    // (the user agent style sheet has declarations that are not supported yet)
    assert!(
        !log.lines().any(|line| line.contains("Error while parsing") && line.contains("mportant")),
        "{}",
        log
    );
}

#[test]
fn the_normal_declarations_of_an_author_beat_those_of_the_user_agent_whatever_the_specificity() {
    // `[hidden]` of the user agent style sheet is more specific than `div`
    let w = widths(
        "origin",
        "<style>div{display:block}</style><div hidden style=\"width:10px\"></div>",
    );
    assert_eq!(w, [10.0]);
}

#[test]
fn a_declaration_with_a_syntax_error_after_important_is_dropped() {
    let w = widths(
        "important_junk",
        "<style>div{width:100px !important foo} div{height:5px}</style><div></div>",
    );
    assert_ne!(w, [100.0]);
}

#[test]
fn rem_is_relative_to_the_font_size_of_the_root_element() {
    let w = widths(
        "rem",
        "<style>html{font-size:10px} body{font-size:30px} div{width:2rem} \
         .em{width:2em} .nested{font-size:50px}</style>\
         <div></div><div class=em></div><div class=nested><div></div></div>",
    );
    // the last two are in the element of 50px: `rem` does not follow
    assert_eq!(w, [20.0, 60.0, 20.0, 20.0]);
}

#[test]
fn rem_of_the_font_size_of_the_root_is_relative_to_the_initial_one() {
    let w = widths(
        "rem_root",
        "<style>html{font-size:2rem} div{width:1rem}</style><div></div>",
    );
    // the initial font size is 16px
    assert_eq!(w, [32.0]);
}

#[test]
fn rem_does_not_change_with_the_font_size_of_the_element() {
    let b = boxes(
        "rem_inherit",
        "<style>html{font-size:20px} div{font-size:40px;width:1rem;height:1em}</style><div></div>",
    );
    assert_eq!(b, [(20.0, 40.0)]);
}

#[test]
fn rem_without_a_font_size_of_the_root_is_16_pixels() {
    assert_eq!(widths("rem_default", "<style>div{width:3rem}</style><div></div>"), [48.0]);
}

#[test]
fn rem_is_valid_in_every_length() {
    let (b, log) = boxes_and_log(
        "rem_props",
        "<style>html{font-size:10px} div{width:5rem;height:4rem;padding:1rem;border:1rem solid;margin:0}</style><div></div>",
    );
    assert!(!log.lines().any(|line| line.contains("Error while parsing") && line.contains("rem")), "{}", log);
    assert_eq!(b, [(5.0 * 10.0 + 40.0, 4.0 * 10.0 + 40.0)]);
}

#[test]
fn an_em_length_uses_the_font_size_of_a_later_rule_of_the_same_element() {
    // `margin` is declared before the more specific rule that sets the font size
    let (b, _) = boxes_and_log(
        "em_later_rule",
        "<style>div{width:1em;height:1em} .big{font-size:40px}</style><div class=big></div>",
    );
    assert_eq!(b, [(40.0, 40.0)]);
}

#[test]
fn media_print_rules_apply() {
    let w = widths(
        "media_print",
        "<style>div{width:10px} @media print{div{width:20px}} @media screen{div{width:30px}} \
         @media all{div{height:7px}}</style><div></div>",
    );
    assert_eq!(w, [20.0]);
}

#[test]
fn media_rules_follow_the_cascade_like_other_rules() {
    // the rule in the block is later, but less specific
    let w = widths(
        "media_cascade",
        "<style>#a{width:10px} @media print{div{width:20px !important}} div{width:30px}</style>\
         <div id=a></div>",
    );
    assert_eq!(w, [20.0]);
}

#[test]
fn media_queries_are_evaluated_for_the_page() {
    // an A4 page is 793.7 CSS pixels wide
    let w = widths(
        "media_size",
        "<style>div{width:1px}\
         @media (min-width: 700px) and (max-width: 800px){div{width:2px}}\
         @media (min-width: 900px){div{width:3px}}\
         @media (orientation: landscape){div{width:4px}}\
         @media not print{div{width:5px}}\
         @media screen, (min-height: 1000px){div{height:6px}}</style><div></div>",
    );
    assert_eq!(w, [2.0]);
    let (_, height) = boxes(
        "media_size_h",
        "<style>@media screen, (min-height: 1000px){div{height:6px}}</style><div></div>",
    )[0];
    assert_eq!(height, 6.0);
}

#[test]
fn media_blocks_nest() {
    let w = widths(
        "media_nested",
        "<style>@media print{@media (min-width: 100px){div{width:20px}} @media screen{div{width:30px}}}\
         @media screen{@media print{div{width:40px}}}</style><div></div>",
    );
    assert_eq!(w, [20.0]);
}

#[test]
fn a_page_rule_inside_a_media_block_applies() {
    let (b, _) = boxes_and_log(
        "media_page",
        "<style>@media print{@page{margin:0;width:300px;height:400px}} div{width:100%}</style><div></div>",
    );
    assert_eq!(b[0].0, 300.0);
}

#[test]
fn a_style_sheet_for_another_medium_is_not_read() {
    let w = widths(
        "media_attribute",
        "<style media=screen>div{width:10px}</style>\
         <style media=print>div{height:20px}</style>\
         <style media=\"(min-width: 9999px)\">div{height:30px}</style>\
         <style media=\"\">div{margin:0}</style>\
         <div></div>",
    );
    assert_ne!(w, [10.0]);
    let b = boxes(
        "media_attribute_h",
        "<style media=screen>div{height:10px}</style><style media=print>div{width:20px}</style><div></div>",
    );
    assert_eq!(b, [(20.0, 0.0)]);
}

#[test]
fn a_media_rule_that_is_not_valid_is_skipped_with_its_block() {
    let w = widths(
        "media_invalid",
        "<style>@media ((( {div{width:10px}} div{height:5px}</style><div></div>",
    );
    assert_ne!(w, [10.0]);
}
