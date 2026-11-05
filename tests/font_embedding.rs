mod common;

use assert_cmd::prelude::*;
use std::process::Command;

fn render(name: &str, html: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("font_embedding_{}.html", name));
    let output = dir.join(format!("font_embedding_{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg(&input)
        .arg(&output)
        .assert()
        .success();
    common::expand_object_streams(&std::fs::read(output).unwrap())
}

/// Finds `/BaseFont /ABCDEF+Name` entries and returns the six-letter tags.
fn subset_tags(pdf: &[u8]) -> Vec<String> {
    let key = b"/BaseFont /";
    let mut tags = vec![];
    for i in 0..pdf.len().saturating_sub(key.len() + 7) {
        if &pdf[i..i + key.len()] == key {
            let name = &pdf[i + key.len()..i + key.len() + 7];
            if name[6] == b'+' && name[..6].iter().all(u8::is_ascii_uppercase) {
                tags.push(String::from_utf8_lossy(&name[..6]).into_owned());
            }
        }
    }
    tags
}

#[test]
fn embedded_fonts_are_subsetted_and_tagged() {
    let pdf = render("small", "<p>Hello subsetting</p>");
    // The complete system font is far larger than a document with 12 glyphs.
    assert!(pdf.len() < 100_000, "PDF is {} bytes", pdf.len());
    assert!(!subset_tags(&pdf).is_empty(), "no tagged subset font name");
}

#[test]
fn different_glyph_sets_give_different_tags() {
    let a = subset_tags(&render("tag_a", "<p>abc</p>"));
    let b = subset_tags(&render("tag_b", "<p>xyz</p>"));
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

#[test]
fn bigger_text_gives_bigger_subset() {
    let small = render("size_a", "<p>a</p>").len();
    let large = render(
        "size_b",
        "<p>The quick brown fox jumps over the lazy dog. PACK MY BOX WITH FIVE DOZEN LIQUOR JUGS 0123456789</p>",
    )
    .len();
    assert!(large > small);
}

/// Characters the font has no glyph for are taken from a fallback font. A built-in PDF font
/// (used when no font of `font-family` is installed) only covers WinAnsi, so the arrow must
/// come from another font, or, if no installed font has one, be reported by name.
#[test]
fn missing_glyphs_use_a_fallback_font_or_are_reported() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("font_embedding_fallback.html");
    let output = dir.join("font_embedding_fallback.pdf");
    std::fs::write(
        &input,
        r#"<p style="font-family: 'Helium Missing Font'">A &#x2192; B, &#x0378;</p>"#,
    )
    .unwrap();
    let result = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);

    // U+0378 is unassigned, so no font has it.
    assert!(stderr.contains("U+0378"), "{stderr}");
    assert!(stderr.contains("font: Helvetica"), "{stderr}");
    assert!(stderr.contains("\"Helium Missing Font\""), "{stderr}");

    let arrow_reported = stderr.contains("U+2192");
    let pdf = common::expand_object_streams(&std::fs::read(output).unwrap());
    let has_fallback_font = !subset_tags(&pdf).is_empty();
    assert_ne!(arrow_reported, has_fallback_font, "{stderr}");
}
