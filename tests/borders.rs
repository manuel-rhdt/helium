//! `border-style`: the styles that are painted, the width of the borders that have none.

mod common;

use std::path::Path;

struct Rendered {
    /// The content stream of the page, uncompressed.
    content: String,
    log: String,
    /// The width of the border box of the `<div>`, in CSS pixels.
    width: f64,
}

fn render(name: &str, css: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("borders__{}.html", name));
    let output = dir.join(format!("borders__{}.pdf", name));
    let layout = dir.join(format!("borders__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>@page{{size:600px 300px;margin:10px}} div{{width:40px;height:20px;{}}}</style><div></div>", css),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    let pdf = String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned();
    // the stream of the page is the first that starts with a graphics state
    let start = pdf.find("stream\nq ").map_or(pdf.len(), |at| at + "stream\n".len());
    let end = pdf[start..].find("endstream").map_or(pdf.len(), |at| start + at);
    let dump: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    fn find(node: &serde_json::Value) -> Option<f64> {
        if node["tag"] == "div" {
            return node["border_box"]["width"].as_f64();
        }
        node["children"].as_array()?.iter().find_map(find)
    }
    Rendered { content: pdf[start..end].to_owned(), log, width: find(&dump["document"]).unwrap() }
}

fn count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

/// The colours (the operands of `rg`) of the fills, in stream order.
fn fill_colors(content: &str) -> Vec<String> {
    let words: Vec<&str> = content.split_whitespace().collect();
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| **word == "rg")
        .map(|(at, _)| words[at - 3..at].join(" "))
        .collect()
}

fn brightness(color: &str) -> f64 {
    color.split(' ').map(|channel| channel.parse::<f64>().unwrap()).sum()
}

const RED: &str = "rgb(200, 100, 50)";

#[test]
fn every_style_is_valid_in_every_notation() {
    for style in ["none", "hidden", "dotted", "dashed", "solid", "double", "groove", "ridge", "inset", "outset"] {
        for css in [
            format!("border-style:{style}"),
            format!("border:3px {style} red"),
            format!("border-top-style:{style}"),
            format!("border-left-style:{style};border-right-style:{style}"),
            format!("border-style:{style} solid"),
        ] {
            let r = render("valid", &css);
            assert!(
                !r.log.lines().any(|line| line.contains("Error while parsing") && line.contains("border")),
                "{css}: {}",
                r.log
            );
        }
    }
    let r = render("invalid", "border:3px wavy red");
    assert!(r.log.contains("Error while parsing"), "{}", r.log);
}

#[test]
fn the_styles_that_show_nothing_take_no_space() {
    for style in ["none", "hidden"] {
        let r = render(&format!("nothing_{style}"), &format!("border:6px {style} red"));
        assert_eq!(r.width, 40.0, "{style}");
        assert!(!r.content.contains(" rg") && !r.content.contains(" RG"), "{style}: {}", r.content);
    }
    for style in ["solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset"] {
        let r = render(&format!("space_{style}"), &format!("border:6px {style} red"));
        assert_eq!(r.width, 52.0, "{style}");
    }
}

#[test]
fn a_solid_border_is_four_lines() {
    let r = render("solid", &format!("border:9px solid {RED}"));
    assert_eq!(count(&r.content, " S "), 4, "{}", r.content);
    assert_eq!(count(&r.content, "6.75 w"), 4, "{}", r.content);
}

#[test]
fn a_dashed_border_has_gaps() {
    let r = render("dashed", "width:400px;border:4px dashed red");
    // the dashes of the top and bottom sides, and one for each side that is short
    assert!(count(&r.content, " l h ") >= 20, "{}", r.content);
    assert_eq!(count(&r.content, " S "), 0);
    let solid = render("dashed_solid", "width:400px;border:4px solid red");
    assert_eq!(count(&solid.content, " l h "), 0);
}

#[test]
fn a_dotted_border_is_made_of_circles() {
    let r = render("dotted", "width:400px;border:4px dotted red");
    // four curves for each dot
    assert!(count(&r.content, " c ") >= 4 * 20, "{}", r.content);
    assert_eq!(count(&r.content, " S "), 0);
}

#[test]
fn a_double_border_is_two_lines_with_a_gap() {
    let r = render("double", &format!("border:9px double {RED}"));
    // two bands for each of the four sides, in one colour
    assert_eq!(count(&r.content, " l h f"), 8, "{}", r.content);
    let colors = fill_colors(&r.content);
    assert!(colors.iter().all(|color| *color == colors[0]), "{colors:?}");
    // a border that is too thin to show two lines is solid
    let thin = render("double_thin", &format!("border:2px double {RED}"));
    assert_eq!(count(&thin.content, " S "), 4, "{}", thin.content);
}

#[test]
fn groove_and_ridge_have_two_tones_that_are_swapped() {
    let groove = render("groove", &format!("border:9px groove {RED}"));
    let ridge = render("ridge", &format!("border:9px ridge {RED}"));
    for r in [&groove, &ridge] {
        let colors = fill_colors(&r.content);
        assert_eq!(colors.len(), 8, "{colors:?}");
        let mut distinct = colors.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 2, "{colors:?}");
    }
    // the outside of the top of a groove is dark (it is cut into the page), of a ridge light
    let outer_top = |r: &Rendered| brightness(&fill_colors(&r.content)[0]);
    assert!(outer_top(&groove) < outer_top(&ridge));
}

#[test]
fn inset_is_dark_at_the_top_and_outset_is_light() {
    let inset = render("inset", &format!("border:9px inset {RED}"));
    let outset = render("outset", &format!("border:9px outset {RED}"));
    // top, bottom, left, right
    let inset = fill_colors(&inset.content);
    let outset = fill_colors(&outset.content);
    assert_eq!(inset.len(), 4, "{inset:?}");
    assert!(brightness(&inset[0]) < brightness(&inset[1]), "{inset:?}");
    assert!(brightness(&inset[2]) < brightness(&inset[3]), "{inset:?}");
    assert!(brightness(&outset[0]) > brightness(&outset[1]), "{outset:?}");
    assert_eq!(inset[0], outset[1]);
    assert_eq!(inset[1], outset[0]);
}

#[test]
fn a_dark_colour_still_has_two_tones() {
    let black = render("black_inset", "border:9px inset black");
    let colors = fill_colors(&black.content);
    assert_ne!(colors[0], colors[1], "{colors:?}");
}

#[test]
fn the_sides_have_their_own_styles() {
    let r = render("mixed", "border:6px solid red;border-bottom-style:dashed;border-left-style:none");
    // the left has no width, the top and the right are lines, the bottom has dashes
    assert_eq!(r.width, 40.0 + 6.0);
    assert_eq!(count(&r.content, " S "), 2, "{}", r.content);
    assert!(count(&r.content, " l h ") >= 1, "{}", r.content);
}

#[test]
fn a_rounded_border_is_painted_in_the_tones_of_its_style() {
    let r = render("rounded_inset", &format!("border:9px inset {RED};border-radius:8px"));
    let mut distinct = fill_colors(&r.content);
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 2, "{distinct:?}");
}
