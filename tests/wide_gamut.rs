//! Colours that sRGB cannot hold are not clipped: a colour of Display P3 is written with an ICC
//! profile of that space, and a colour outside of Display P3 in a CalRGB space that carries CIE
//! XYZ.

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

/// The values of the `cs`/`sc` operations (fills) and `CS`/`SC` operations (strokes): the name of
/// the colour space and the three components.
fn space_colors(pdf: &str, fill: bool) -> Vec<(String, Vec<f64>)> {
    let (cs, sc) = if fill { ("cs", "sc") } else { ("CS", "SC") };
    let tokens: Vec<&str> = pdf.split_whitespace().collect();
    let mut found = vec![];
    for at in 0..tokens.len().saturating_sub(5) {
        if tokens[at].starts_with("/CS") && tokens[at + 1] == cs && tokens[at + 5] == sc {
            found.push((tokens[at].to_owned(), tokens[at + 2..at + 5].iter().map(|v| v.parse().unwrap()).collect()));
        }
    }
    found
}

fn rgb_count(pdf: &str) -> usize {
    pdf.split_whitespace().filter(|token| *token == "rg").count()
}

fn close(found: &[f64], expected: [f64; 3]) -> bool {
    found.iter().zip(expected).all(|(found, expected)| (found - expected).abs() < 0.0006)
}

#[test]
fn a_color_of_display_p3_has_an_icc_profile() {
    let pdf = render("p3", "<p style=\"background: color(display-p3 1 0 0)\">x</p>");
    let fills = space_colors(&pdf, true);
    assert_eq!(fills.len(), 1, "{fills:?}");
    // the colour is the one that was written: the components of Display P3
    assert!(close(&fills[0].1, [1.0, 0.0, 0.0]), "{:?}", fills[0]);
    assert!(pdf.contains("/ICCBased"), "{pdf}");
    // the profile is in the file (its signature `acsp`), once
    assert_eq!(pdf.matches("acsp").count(), 1);
    assert!(!pdf.contains("/CalRGB") && !pdf.contains("/Lab"));
}

#[test]
fn a_color_outside_of_display_p3_is_xyz_in_a_calrgb_space() {
    let pdf = render("xyz", "<p style=\"background: color(rec2020 0 1 0)\">x</p>");
    let fills = space_colors(&pdf, true);
    assert_eq!(fills.len(), 1, "{fills:?}");
    // X, Y and Z of Rec. 2020 green adapted to D50 (Bradford) and divided by the white of D50: the
    // components of the carrier whose (1, 1, 1) is the white point (the one that is exact on an iPhone)
    assert!(close(&fills[0].1, [0.17183, 0.67532, 0.03633]), "{:?}", fills[0]);
    assert!(pdf.contains("/CalRGB"), "{pdf}");
    assert!(pdf.contains("/Gamma"));
    assert!(pdf.contains("/WhitePoint"));
    // the matrix is the white point on its diagonal: (1, 1, 1) is the white
    assert!(pdf.contains("0.964296") && pdf.contains("0.825105"), "{pdf}");
    assert!(!pdf.contains("/ICCBased"), "an ICC profile is only written for a colour that needs it");
}

#[test]
fn text_borders_and_backgrounds_keep_their_colors() {
    let pdf = render(
        "all",
        "<p style=\"color: color(rec2020 0 1 0); background: oklch(0.7 0.3 150); border: 3px solid color(prophoto-rgb 1 0 0)\">word</p>",
    );
    // the background and the text are fills, and the border too
    assert!(space_colors(&pdf, true).len() >= 2, "{:?}", space_colors(&pdf, true));
    assert!(pdf.contains("/CalRGB"));
}

#[test]
fn the_profile_is_written_once_for_the_whole_document() {
    let pdf = render(
        "once",
        "<p style=\"color: color(display-p3 1 0 0); background: color(display-p3 0 1 0)\">x</p>\
         <p style=\"break-before: page; color: color(display-p3 0 0.5 1)\">y</p>",
    );
    assert!(space_colors(&pdf, true).len() >= 3);
    assert_eq!(pdf.matches("acsp").count(), 1, "one profile for two pages");
}

#[test]
fn a_color_inside_of_srgb_stays_device_rgb() {
    let pdf = render(
        "inside",
        "<p style=\"color: color(display-p3 0.5 0.5 0.5); background: lab(50 20 30); border: 2px solid oklch(0.6 0.1 200)\">x</p>",
    );
    assert!(space_colors(&pdf, true).is_empty() && space_colors(&pdf, false).is_empty());
    assert!(rgb_count(&pdf) >= 2);
    assert!(!pdf.contains("/ICCBased") && !pdf.contains("/CalRGB"), "no colour outside of sRGB, so no colour space");
}

#[test]
fn opacity_and_alpha_keep_the_wide_color() {
    // the alpha of a colour is set apart from its colour space
    let pdf = render("alpha", "<p style=\"background: color(display-p3 0 1 0 / 40%); opacity: 0.8\">x</p>");
    assert_eq!(space_colors(&pdf, true).len(), 1, "{:?}", space_colors(&pdf, true));
    assert!(pdf.contains("/ca 0.4") || pdf.contains("/ca .4"), "{pdf}");
}

#[test]
fn the_layout_dump_names_the_space_of_a_wide_color() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("wide_dump.html");
    std::fs::write(
        &input,
        "<div style=\"background: color(display-p3 1 0 0); width: 10px; height: 10px\"></div>\
         <div style=\"background: color(rec2020 0 1 0); width: 10px; height: 10px\"></div>",
    )
    .unwrap();
    let output = common::helium().unwrap().arg("--dump-layout").arg("-").arg(&input).arg(dir.join("wide_dump.pdf")).output().unwrap();
    let dump = String::from_utf8_lossy(&output.stdout);
    assert!(dump.contains("\"display-p3\""), "{dump}");
    assert!(dump.contains("\"xyz\""), "{dump}");
}
