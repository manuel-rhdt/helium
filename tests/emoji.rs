#![cfg(feature = "embedded-fonts")]
mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("emoji_{name}.html"));
    let output = dir.join(format!("emoji_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::umbruch().unwrap().arg(&input).arg(&output).assert().success();
    common::expand_object_streams(&std::fs::read(output).unwrap())
}

fn contains(pdf: &[u8], needle: &str) -> bool {
    pdf.windows(needle.len()).any(|window| window == needle.as_bytes())
}

#[test]
fn emoji_are_painted_in_colour_with_the_built_in_font() {
    let pdf = render("body", "<p style=\"font-size: 40px\">Hi 😀 🎉 🇩🇪 👨‍👩‍👧 👍🏽</p>");
    // the text keeps its glyphs (for copying), and the layers are forms with shadings
    assert!(contains(&pdf, "NotoColorEmoji"));
    assert!(contains(&pdf, "/ShadingType"));
    assert!(contains(&pdf, "/Subtype /Form"));
    // the glyphs of the colour font are subset, and its tables for painting are not embedded
    assert!(pdf.len() < 400_000, "PDF is {} bytes", pdf.len());
}

#[test]
fn text_presentation_stays_text() {
    // no emoji presentation without U+FE0F
    let pdf = render("text", "<p>✓ ☃ ❤</p>");
    assert!(!contains(&pdf, "NotoColorEmoji"));
}

#[test]
fn emoji_in_margin_boxes_and_list_items_are_painted() {
    let pdf = render(
        "margins",
        "<style>@page { margin: 40px; @bottom-center { content: \"🎉 \" counter(page); } }</style>\
         <ul><li>🐱 cat</li></ul>",
    );
    // one form per distinct glyph: the party popper and the cat
    let forms = pdf.windows(b"/Subtype /Form".len()).filter(|w| *w == b"/Subtype /Form").count();
    assert!(forms >= 2, "{forms} forms");
}

#[test]
fn emoji_in_svg_text_are_painted_like_the_emoji_of_the_page() {
    let page = render("page_only", "<p style=\"font-size: 40px\">😀</p>");
    let both = render(
        "svg_and_page",
        "<p style=\"font-size: 40px\">😀</p>\
         <svg width=\"200\" height=\"60\"><text x=\"5\" y=\"40\" font-size=\"30\">😀 🎉</text></svg>",
    );
    let forms = |pdf: &[u8]| pdf.windows(b"/Subtype /Form".len()).filter(|w| *w == b"/Subtype /Form").count();
    assert!(contains(&both, "/ShadingType"));
    // the smiley is one form for the page and the image, the party popper adds its own: the
    // forms of a glyph are written once for the whole document
    let (page_forms, both_forms) = (forms(&page), forms(&both));
    assert!(both_forms > page_forms, "{page_forms} vs {both_forms}");
    let single = render("svg_smiley", "<svg width=\"200\" height=\"60\"><text x=\"5\" y=\"40\" font-size=\"30\">😀</text></svg>");
    assert_eq!(forms(&single), page_forms, "the smiley takes the same forms on the page and in an SVG");
}

#[test]
fn emoji_in_svg_text_with_a_gradient_fill_are_still_painted() {
    // the letters are outlines painted with the gradient, the emoji have no outline and take the
    // same path as everywhere else
    let pdf = render(
        "svg_gradient",
        "<svg width=\"200\" height=\"60\"><defs><linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"red\"/>\
         <stop offset=\"1\" stop-color=\"blue\"/></linearGradient></defs>\
         <text x=\"5\" y=\"40\" font-size=\"30\" fill=\"url(#g)\">ab 😀</text></svg>",
    );
    let forms = pdf.windows(b"/Subtype /Form".len()).filter(|w| *w == b"/Subtype /Form").count();
    assert!(forms >= 1, "{forms} forms");
}

#[test]
fn emoji_with_a_variation_selector_do_not_drop_the_fallback_of_the_other_emoji_in_svg_text() {
    // the emoji font makes one glyph of "❤️" where the font of the text makes two: the fallback
    // replaces clusters, so the smiley next to it is found too
    let pdf = render(
        "svg_mixed",
        "<svg width=\"260\" height=\"60\"><text x=\"5\" y=\"40\" font-size=\"30\" font-family=\"serif\">Hi 😀 🎉 ❤️ ab</text></svg>",
    );
    let forms = pdf.windows(b"/Subtype /Form".len()).filter(|w| *w == b"/Subtype /Form").count();
    assert!(forms >= 3, "{forms} forms");
}
