//! The alpha channel of colors and the CSS `opacity` property.

mod common;

use std::path::Path;

fn render(name: &str, html: &str) -> String {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("opacity__{}.html", name));
    let output = dir.join(format!("opacity__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    let pdf = common::expand_object_streams(&std::fs::read(output).unwrap());
    String::from_utf8_lossy(&pdf).into_owned()
}

const PAGE: &str = "<style>@page{width:300pt;height:300pt;margin:20pt} body{margin:0}</style>";

#[test]
fn an_opaque_document_has_no_graphics_state_for_alpha() {
    let pdf = render("opaque", &format!("{PAGE}<div style=\"background:#c00;color:#00c\">text</div>"));
    assert!(!pdf.contains("/ca "), "{}", pdf);
    assert!(!pdf.contains("/CA "), "{}", pdf);
}

#[test]
fn the_alpha_of_a_background_is_painted() {
    let pdf = render("background", &format!("{PAGE}<div style=\"background:rgba(200,0,0,0.5);height:50px\"></div>"));
    assert!(pdf.contains("/ca 0.5"), "{}", pdf);
}

#[test]
fn the_alpha_of_text_is_painted() {
    let pdf = render("text", &format!("{PAGE}<p style=\"color:rgb(0 0 200 / 25%)\">text</p>"));
    assert!(pdf.contains("/ca 0.25"), "{}", pdf);
}

#[test]
fn the_alpha_of_a_border_is_painted_as_a_fill_or_stroke() {
    let pdf = render(
        "border",
        &format!("{PAGE}<div style=\"border:4px solid #0000ff80;height:50px\"></div>"),
    );
    assert!(pdf.contains("/ca 0.50") || pdf.contains("/CA 0.50") || pdf.contains("/ca 0.5") || pdf.contains("/CA 0.5"), "{}", pdf);
}

#[test]
fn the_alpha_does_not_leak_to_the_next_color() {
    let pdf = render(
        "leak",
        &format!(
            "{PAGE}<div style=\"background:rgba(200,0,0,0.5);height:50px\"></div><div style=\"background:#00c;height:50px\"></div>"
        ),
    );
    // each box is in its own saved graphics state: only the translucent one sets an opacity
    assert_eq!(pdf.matches(" gs ").count(), 1, "{}", pdf);
    let at = pdf.find(" gs ").unwrap();
    assert!(!pdf[at + 4..].contains(" gs "), "{}", pdf);
}

/// The number of the forms that are transparency groups.
fn groups(pdf: &str) -> usize {
    pdf.matches("/S /Transparency").count()
}

#[test]
fn opacity_paints_the_box_as_a_translucent_group() {
    let pdf = render(
        "group",
        &format!("{PAGE}<div style=\"opacity:0.4;background:#c00;width:100px;height:50px\">text</div>"),
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
    // the group is drawn with the opacity, for fills and strokes
    assert!(pdf.contains("/ca 0.4"), "{}", pdf);
    assert!(pdf.contains("/CA 0.4"), "{}", pdf);
}

#[test]
fn an_opaque_box_makes_no_group() {
    for value in ["1", "100%", "2", "initial"] {
        let pdf = render(
            "no_group",
            &format!("{PAGE}<div style=\"opacity:{value};background:#c00;height:50px\">text</div>"),
        );
        assert_eq!(groups(&pdf), 0, "opacity: {value}");
    }
}

#[test]
fn percentages_are_accepted_and_values_are_clamped() {
    let pdf = render("percent", &format!("{PAGE}<div style=\"opacity:25%;background:#c00;height:50px\"></div>"));
    assert!(pdf.contains("/ca 0.25"), "{}", pdf);
    let pdf = render("negative", &format!("{PAGE}<div style=\"opacity:-3;background:#c00;height:50px\"></div>"));
    // nothing is painted for an invisible box
    assert_eq!(groups(&pdf), 0, "{}", pdf);
    assert!(!pdf.contains(" re"), "{}", pdf);
}

#[test]
fn an_invalid_opacity_is_ignored() {
    let pdf = render("invalid", &format!("{PAGE}<div style=\"opacity:half;background:#c00;height:50px\"></div>"));
    assert_eq!(groups(&pdf), 0, "{}", pdf);
    assert!(pdf.contains(" re"), "{}", pdf);
}

#[test]
fn opacity_is_not_inherited_but_nests() {
    let pdf = render(
        "nested",
        &format!(
            "{PAGE}<div style=\"opacity:0.5;background:#c00;height:80px\"><div style=\"opacity:0.5;background:#00c;height:40px\"></div></div>"
        ),
    );
    // one group inside of the other, each with its own opacity (not 0.25 for the inner one)
    assert_eq!(groups(&pdf), 2, "{}", pdf);
    assert!(pdf.contains("/ca 0.5"), "{}", pdf);
    assert!(!pdf.contains("/ca 0.25"), "{}", pdf);
    let child_only = render(
        "child",
        &format!("{PAGE}<div style=\"opacity:0.5\"><div style=\"background:#00c;height:40px\"></div></div>"),
    );
    assert_eq!(groups(&child_only), 1, "{}", child_only);
}

#[test]
fn opacity_and_transform_on_the_same_box() {
    let pdf = render(
        "transform",
        &format!(
            "{PAGE}<div style=\"opacity:0.5;transform:translate(10px,20px);width:100px;background:#c00\">text</div>"
        ),
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
    // the transformation is inside of the group
    assert!(pdf.contains("1 0 0 1 7.5 -15 cm"), "{}", pdf);
}

#[test]
fn the_opacity_of_an_inline_box_is_in_its_colors_and_those_of_its_descendants() {
    let pdf = render(
        "inline",
        &format!(
            "{PAGE}<p>a <span style=\"opacity:0.5;background:#c00\">b <em style=\"color:#00c\">c</em></span> d</p>"
        ),
    );
    // not a group: the text of the span is part of the paragraph
    assert_eq!(groups(&pdf), 0, "{}", pdf);
    // the background, the text of the span and the text of its child (with its own color)
    assert!(pdf.contains("/ca 0.5"), "{}", pdf);
    // (one graphics state serves the background and the text, which are both at 0.5)
    assert!(pdf.matches(" gs ").count() >= 2, "{}", pdf);
}

#[test]
fn an_inline_block_and_a_replaced_element_are_groups() {
    let pdf = render(
        "inline_block",
        &format!("{PAGE}<p>a <span style=\"display:inline-block;opacity:0.5;background:#c00\">b</span></p>"),
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
    let pdf = render(
        "image",
        &format!(
            "{PAGE}<p><img style=\"opacity:0.5\" width=\"40\" height=\"40\" src=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill='red'/%3E%3C/svg%3E\"></p>"
        ),
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
}

/// The text that is painted in the page content after the (only) transparency group, i.e.
/// outside of it.
fn text_after_the_group(pdf: &str) -> usize {
    let at = pdf.find(" Do").expect("a form is drawn");
    let rest = &pdf[at..];
    let end = rest.find("endstream").unwrap_or(rest.len());
    rest[..end].matches("BT").count()
}

#[test]
fn the_opacity_of_a_text_block_does_not_leak_to_the_next_adjacent_block() {
    let pdf = render(
        "adjacent",
        "<style>@page{size:120mm 50mm;margin:4mm}body{background:#36c;color:#fff;font:16pt serif}</style>\
         <div style=\"opacity:.45\">A: half</div><div>B: full?</div><p style=\"margin:2mm 0\">C: full?</p>",
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
    // B and C are painted outside of the group
    assert_eq!(text_after_the_group(&pdf), 2, "{}", pdf);
}

#[test]
fn the_opacity_of_a_block_does_not_leak_to_a_following_blockquote() {
    let pdf = render(
        "blockquote",
        "<style>@page{size:120mm 50mm;margin:4mm}body{color:#fff;background:#36c;font:16pt serif}</style>\
         <div style=\"opacity:.45;height:10mm\">\u{201C}</div><blockquote>quoted</blockquote>",
    );
    assert_eq!(groups(&pdf), 1, "{}", pdf);
    assert_eq!(text_after_the_group(&pdf), 1, "{}", pdf);
}
