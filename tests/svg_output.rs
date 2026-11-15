//! How much the PDF of an SVG takes: the forms, shadings and operators it is made of. These
//! tests count objects and operators, the pictures are in `images.rs` and `inline_svg.rs`.

mod common;

use std::path::Path;

struct Rendered {
    /// The PDF with uncompressed content streams, as text.
    pdf: String,
    log: String,
}

fn render(name: &str, svg: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("svg_output__{}.html", name));
    let output = dir.join(format!("svg_output__{}.pdf", name));
    std::fs::write(
        &input,
        format!(
            "<style>@page{{width:300pt;height:300pt;margin:20pt}}</style>\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"200\">{}</svg>",
            svg
        ),
    )
    .unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{}", log);
    let bytes = std::fs::read(output).unwrap();
    Rendered { pdf: String::from_utf8_lossy(&bytes).into_owned(), log }
}

fn count(pdf: &str, needle: &str) -> usize {
    pdf.matches(needle).count()
}

/// The number of forms of a rendering: the one of the SVG itself and what the drawing needs.
fn forms(r: &Rendered) -> usize {
    count(&r.pdf, "/Subtype /Form")
}

#[test]
fn a_group_with_opacity_around_one_path_needs_no_transparency_group() {
    let plain = render("one_path_plain", "<rect width=\"50\" height=\"50\" fill=\"#f00\"/>");
    let r = render(
        "one_path",
        "<g opacity=\"0.5\"><rect width=\"50\" height=\"50\" fill=\"#f00\"/></g>",
    );
    assert_eq!(forms(&r), forms(&plain), "{}", r.log);
    assert!(r.pdf.contains("/ca 0.5"), "{}", r.pdf);
}

#[test]
fn the_opacity_of_a_flat_group_multiplies_with_that_of_its_content() {
    let r = render(
        "product",
        "<g opacity=\"0.5\"><rect width=\"50\" height=\"50\" fill=\"#f00\" fill-opacity=\"0.5\"/></g>",
    );
    assert!(r.pdf.contains("/ca 0.25"), "{}", r.pdf);
}

#[test]
fn separate_shapes_in_a_group_with_opacity_need_no_transparency_group() {
    let plain = render("apart_plain", "<rect width=\"50\" height=\"50\"/>");
    let r = render(
        "apart",
        "<g opacity=\"0.5\"><rect width=\"50\" height=\"50\" fill=\"#f00\"/>\
         <circle cx=\"120\" cy=\"25\" r=\"20\" fill=\"#00f\"/></g>",
    );
    assert_eq!(forms(&r), forms(&plain), "{}", r.log);
}

#[test]
fn overlapping_shapes_in_a_group_with_opacity_need_one() {
    let plain = render("overlap_plain", "<rect width=\"50\" height=\"50\"/>");
    let r = render(
        "overlap",
        "<g opacity=\"0.5\"><rect width=\"50\" height=\"50\" fill=\"#f00\"/>\
         <circle cx=\"50\" cy=\"25\" r=\"20\" fill=\"#00f\"/></g>",
    );
    assert_eq!(forms(&r), forms(&plain) + 1, "{}", r.log);
}

#[test]
fn a_path_with_fill_and_stroke_in_a_group_with_opacity_needs_one() {
    // the stroke covers half of the fill: with the opacity of the group the stroke does not
    // show the fill through
    let plain = render("both_plain", "<rect width=\"50\" height=\"50\"/>");
    let r = render(
        "both",
        "<g opacity=\"0.5\"><rect width=\"50\" height=\"50\" fill=\"#f00\" stroke=\"#000\" stroke-width=\"8\"/></g>",
    );
    assert_eq!(forms(&r), forms(&plain) + 1, "{}", r.log);
}

#[test]
fn a_mask_around_one_path_needs_only_the_mask() {
    let plain = render("mask_plain", "<rect width=\"50\" height=\"50\"/>");
    let r = render(
        "mask",
        "<mask id=\"m\" maskUnits=\"userSpaceOnUse\" width=\"200\" height=\"200\">\
           <rect width=\"200\" height=\"200\" fill=\"#888\"/></mask>\
         <g mask=\"url(#m)\"><rect width=\"50\" height=\"50\" fill=\"#f00\"/></g>",
    );
    // the group of the mask
    assert_eq!(forms(&r), forms(&plain) + 1, "{}", r.log);
    assert!(r.pdf.contains("/SMask"), "{}", r.pdf);
}

#[test]
fn a_gradient_that_paths_share_is_one_shading() {
    // every bar has its own transformation of the gradient (the one of its box), the shading
    // is the same
    let mut bars = String::new();
    for i in 0..10 {
        bars.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"8\" height=\"{}\" fill=\"url(#g)\"/>",
            i * 10,
            100 - i * 7,
            i * 7 + 10
        ));
    }
    let r = render(
        "shared_gradient",
        &format!(
            "<linearGradient id=\"g\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"#00f\"/>\
             <stop offset=\"1\" stop-color=\"#0ff\"/></linearGradient>{}",
            bars
        ),
    );
    assert_eq!(count(&r.pdf, "/ShadingType 2"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/PatternType 2"), 0);
    assert_eq!(count(&r.pdf, " sh "), 10, "{}", r.pdf);
}

#[test]
fn the_opacity_of_a_gradient_fill_applies_to_the_shading() {
    let r = render(
        "gradient_opacity",
        "<linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"#00f\"/>\
         <stop offset=\"1\" stop-color=\"#0ff\"/></linearGradient>\
         <rect width=\"50\" height=\"50\" fill=\"url(#g)\" fill-opacity=\"0.5\"/>",
    );
    assert!(r.pdf.contains("/ca 0.5"), "{}", r.pdf);
    assert_eq!(count(&r.pdf, " sh "), 1);
}

#[test]
fn a_gradient_stroke_is_a_pattern() {
    // a stroke cannot clip a shading
    let r = render(
        "gradient_stroke",
        "<linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"#00f\"/>\
         <stop offset=\"1\" stop-color=\"#0ff\"/></linearGradient>\
         <rect width=\"50\" height=\"50\" fill=\"none\" stroke=\"url(#g)\" stroke-width=\"4\"/>",
    );
    assert_eq!(count(&r.pdf, "/PatternType 2"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, " sh "), 0);
}

#[test]
fn a_tiling_pattern_that_paths_share_is_written_once() {
    let r = render(
        "shared_pattern",
        "<pattern id=\"p\" width=\"8\" height=\"8\" patternUnits=\"userSpaceOnUse\">\
           <rect width=\"4\" height=\"4\"/></pattern>\
         <rect width=\"50\" height=\"50\" fill=\"url(#p)\"/>\
         <rect x=\"60\" width=\"50\" height=\"50\" fill=\"url(#p)\"/>\
         <rect x=\"120\" width=\"50\" height=\"50\" fill=\"url(#p)\"/>",
    );
    assert_eq!(count(&r.pdf, "/PatternType 1"), 1, "{}", r.log);
}

#[test]
fn colors_and_line_styles_are_set_when_they_change() {
    let r = render(
        "state",
        "<rect width=\"10\" height=\"10\" fill=\"#f00\" stroke=\"#000\" stroke-width=\"2\"/>\
         <rect x=\"20\" width=\"10\" height=\"10\" fill=\"#f00\" stroke=\"#000\" stroke-width=\"2\"/>\
         <rect x=\"40\" width=\"10\" height=\"10\" fill=\"#f00\" stroke=\"#000\" stroke-width=\"2\"/>",
    );
    assert_eq!(count(&r.pdf, "1 0 0 rg"), 1, "{}", r.pdf);
    assert_eq!(count(&r.pdf, "0 0 0 RG"), 1, "{}", r.pdf);
    assert_eq!(count(&r.pdf, "2 w"), 1, "{}", r.pdf);
    // the paths need no `q` and `Q` of their own: the one of the page, and the one that scopes
    // the form's clip of the image
    assert!(count(&r.pdf, " q ") + count(&r.pdf, "\nq ") <= 3, "{}", r.pdf);
}

#[test]
fn a_dash_pattern_is_reset_for_the_next_stroke() {
    let r = render(
        "dash",
        "<path d=\"M0 10 h50\" stroke=\"#000\" stroke-dasharray=\"4 2\"/>\
         <path d=\"M0 20 h50\" stroke=\"#000\"/>",
    );
    let dashed = r.pdf.find("[ 4 2 ] 0 d").expect(&r.pdf);
    let solid = r.pdf.find("[ ] 0 d").expect(&r.pdf);
    assert!(dashed < solid, "{}", r.pdf);
}

#[test]
fn a_line_is_not_filled() {
    // a line has no inside: the fill of the black default paints nothing (in some viewers a
    // hairline)
    let r = render("line", "<line x2=\"50\" y2=\"50\" stroke=\"#f00\"/>");
    assert!(!r.pdf.contains("0 0 0 rg"), "{}", r.pdf);
    assert!(r.pdf.contains("1 0 0 RG"), "{}", r.pdf);
}

#[test]
fn a_mask_that_groups_share_is_written_once() {
    let r = render(
        "shared_mask",
        "<mask id=\"m\" maskUnits=\"userSpaceOnUse\" width=\"200\" height=\"200\">\
           <rect width=\"200\" height=\"200\" fill=\"#888\"/></mask>\
         <g mask=\"url(#m)\"><rect width=\"50\" height=\"50\" fill=\"#f00\"/></g>\
         <g mask=\"url(#m)\"><rect x=\"60\" width=\"50\" height=\"50\" fill=\"#00f\"/></g>",
    );
    assert_eq!(count(&r.pdf, "/S /Transparency"), 1, "{}", r.log);
}
