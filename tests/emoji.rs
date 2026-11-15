mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("emoji_{name}.html"));
    let output = dir.join(format!("emoji_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::helium().unwrap().arg(&input).arg(&output).assert().success();
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
