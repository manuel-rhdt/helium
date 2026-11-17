//! Colours that sRGB cannot hold are written to the PDF as CIELAB colours, not clipped.

mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("wide_{name}.html"));
    let output = dir.join(format!("wide_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::helium().unwrap().arg("--no-compression").arg("--no-system-fonts").arg(&input).arg(&output).assert().success();
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

/// The values of the `cs`/`sc` operations (fills) and `CS`/`SC` operations (strokes).
fn lab_colors(pdf: &str, fill: bool) -> Vec<Vec<f64>> {
    let (cs, sc) = if fill { ("cs", "sc") } else { ("CS", "SC") };
    let tokens: Vec<&str> = pdf.split_whitespace().collect();
    let mut found = vec![];
    for at in 0..tokens.len().saturating_sub(5) {
        if tokens[at].starts_with("/CS") && tokens[at + 1] == cs && tokens[at + 5] == sc {
            found.push(tokens[at + 2..at + 5].iter().map(|v| v.parse().unwrap()).collect());
        }
    }
    found
}

fn rgb_count(pdf: &str) -> usize {
    pdf.split_whitespace().filter(|token| *token == "rg").count()
}

#[test]
fn a_color_outside_of_srgb_is_written_as_lab() {
    let pdf = render("fill", "<p style=\"background: color(display-p3 1 0 0)\">x</p>");
    let fills = lab_colors(&pdf, true);
    assert_eq!(fills.len(), 1, "{fills:?}");
    // checked against the formulas of CSS Color 4 in float64
    let [l, a, b] = [fills[0][0], fills[0][1], fills[0][2]];
    assert!((l - 56.21).abs() < 0.02 && (a - 94.46).abs() < 0.02 && (b - 98.89).abs() < 0.02, "{l} {a} {b}");
    // the colour space is D50 CIELAB, named in the resources
    assert!(pdf.contains("/Lab"), "{pdf}");
    assert!(pdf.contains("/WhitePoint"));
}

#[test]
fn text_borders_and_backgrounds_keep_their_colors() {
    let pdf = render(
        "all",
        "<p style=\"color: color(rec2020 0 1 0); background: oklch(0.7 0.3 150); border: 3px solid color(prophoto-rgb 1 0 0)\">word</p>",
    );
    // the background and the text are fills
    assert!(lab_colors(&pdf, true).len() >= 2, "{:?}", lab_colors(&pdf, true));
    assert!(pdf.contains("/Lab"));
}

#[test]
fn a_color_inside_of_srgb_stays_device_rgb() {
    let pdf = render(
        "inside",
        "<p style=\"color: color(display-p3 0.5 0.5 0.5); background: lab(50 20 30); border: 2px solid oklch(0.6 0.1 200)\">x</p>",
    );
    assert!(lab_colors(&pdf, true).is_empty() && lab_colors(&pdf, false).is_empty());
    assert!(rgb_count(&pdf) >= 2);
    assert!(!pdf.contains("/Lab"), "a document without colours outside of sRGB has no Lab space");
}

#[test]
fn opacity_and_alpha_keep_the_wide_color() {
    // the alpha of a colour is set apart from its colour space
    let pdf = render("alpha", "<p style=\"background: color(display-p3 0 1 0 / 40%); opacity: 0.8\">x</p>");
    assert_eq!(lab_colors(&pdf, true).len(), 1, "{:?}", lab_colors(&pdf, true));
    assert!(pdf.contains("/ca 0.4") || pdf.contains("/ca .4"), "{pdf}");
}

#[test]
fn the_layout_dump_says_lab_for_a_wide_color() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("wide_dump.html");
    std::fs::write(&input, "<div style=\"background: color(display-p3 1 0 0); width: 10px; height: 10px\"></div>").unwrap();
    let output = common::helium().unwrap().arg("--dump-layout").arg("-").arg(&input).arg(dir.join("wide_dump.pdf")).output().unwrap();
    let dump = String::from_utf8_lossy(&output.stdout);
    assert!(dump.contains("\"lab\""), "{dump}");
}
