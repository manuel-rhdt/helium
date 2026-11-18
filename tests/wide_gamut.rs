//! Colours that sRGB cannot hold are not clipped to it: they are written in one ICC space with the
//! primaries of Rec. 2020 and a gamma of 2.4 (the one that Display P3 and nearly all of CSS fit in, and
//! that an iPhone shows as the profile says). Colours of sRGB stay device RGB, which `DefaultRGB`
//! defines as sRGB. Only a colour that Rec. 2020 cannot hold is clipped, to Rec. 2020.

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

/// Profiles in the file: the ones that `DefaultRGB` and the colours write are streams with `/N 3`.
fn profiles(pdf: &str) -> usize {
    pdf.matches("/N 3").count()
}

#[test]
fn a_color_of_display_p3_is_in_the_rec2020_profile() {
    let pdf = render("p3", "<p style=\"background: color(display-p3 1 0 0)\">x</p>");
    // the black of the text is in the sRGB space too: it is the colour that is not 0 0 0
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).take(1).collect();
    assert!(!fills.is_empty(), "{fills:?}");
    // Display P3 red in Rec. 2020: the linear coordinates to the power of 1 / 2.4
    assert!(close(&fills[0].1, [0.88892, 0.27657, 0.0]), "{:?}", fills[0]);
    assert!(pdf.contains("/ICCBased"), "{pdf}");
    // the profile of the colours and the sRGB profile of DefaultRGB
    assert_eq!(profiles(&pdf), 2);
    assert!(!pdf.contains("/CalRGB") && !pdf.contains("/Lab"));
}

#[test]
fn rec2020_green_is_its_own_primary() {
    let pdf = render("rec2020", "<p style=\"background: color(rec2020 0 1 0)\">x</p>");
    // the black of the text is in the sRGB space too: it is the colour that is not 0 0 0
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).take(1).collect();
    assert!(!fills.is_empty(), "{fills:?}");
    assert!(close(&fills[0].1, [0.0, 1.0, 0.0]), "{:?}", fills[0]);
}

#[test]
fn a_color_that_rec2020_cannot_hold_is_clipped_to_it() {
    let pdf = render("prophoto", "<p style=\"background: color(prophoto-rgb 0 1 0)\">x</p>");
    // the black of the text is in the sRGB space too: it is the colour that is not 0 0 0
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).take(1).collect();
    assert!(!fills.is_empty(), "{fills:?}");
    assert!(fills[0].1.iter().all(|v| (0.0..=1.0).contains(v)), "{:?}", fills[0]);
    assert!(!pdf.contains("/CalRGB"));
}

#[test]
fn text_borders_and_backgrounds_keep_their_colors() {
    let pdf = render(
        "all",
        "<p style=\"color: color(rec2020 0 1 0); background: oklch(0.7 0.3 150); border: 3px solid color(prophoto-rgb 1 0 0)\">word</p>",
    );
    // the background and the text are fills, and the border too
    assert!(space_colors(&pdf, true).len() >= 2, "{:?}", space_colors(&pdf, true));
    assert!(pdf.contains("/ICCBased") && !pdf.contains("/CalRGB"));
}

#[test]
fn the_profile_is_written_once_for_the_whole_document() {
    let pdf = render(
        "once",
        "<p style=\"color: color(display-p3 1 0 0); background: color(display-p3 0 1 0)\">x</p>\
         <p style=\"break-before: page; color: color(display-p3 0 0.5 1)\">y</p>",
    );
    assert!(space_colors(&pdf, true).len() >= 3);
    assert_eq!(profiles(&pdf), 2, "the colour profile and the sRGB one, for two pages");
}

#[test]
fn a_color_inside_of_srgb_is_in_the_srgb_profile() {
    let pdf = render(
        "inside",
        "<p style=\"color: color(display-p3 0.5 0.5 0.5); background: lab(50 20 30); border: 2px solid oklch(0.6 0.1 200)\">x</p>",
    );
    // the colours are in the sRGB profile (one for the document), the same one that DefaultRGB uses
    assert!(space_colors(&pdf, true).len() >= 2, "{:?}", space_colors(&pdf, true));
    assert!(!space_colors(&pdf, true).iter().chain(space_colors(&pdf, false).iter()).any(|(_, c)| c.iter().any(|v| !(0.0..=1.0).contains(v))));
    assert_eq!(rgb_count(&pdf), 0, "no device RGB colour in the content");
    assert!(!pdf.contains("/CalRGB"));
    assert_eq!(profiles(&pdf), 1, "only the sRGB profile");
    assert!(pdf.contains("/DefaultRGB"));
}

#[test]
fn opacity_and_alpha_keep_the_wide_color() {
    // the alpha of a colour is set apart from its colour space
    let pdf = render("alpha", "<p style=\"background: color(display-p3 0 1 0 / 40%); opacity: 0.8\">x</p>");
    assert!(!space_colors(&pdf, true).is_empty(), "{:?}", space_colors(&pdf, true));
    assert_eq!(profiles(&pdf), 2, "the sRGB profile and the wide one");
    assert!(pdf.contains("/ca 0.4") || pdf.contains("/ca .4"), "{pdf}");
}

#[test]
fn a_gradient_with_wide_colors_is_in_the_wide_profile() {
    let pdf = render("gradient_wide", "<div style=\"height:100px;background:linear-gradient(color(display-p3 1 0 0), color(display-p3 0 1 0))\"></div>");
    assert!(pdf.contains("/ShadingType 2"), "{pdf}");
    assert!(pdf.contains("/ColorSpace [ /ICCBased"), "{pdf}");
    assert!(!pdf.contains("/ColorSpace /DeviceRGB"), "{pdf}");
    assert_eq!(profiles(&pdf), 2, "the wide profile and the sRGB one");
}

#[test]
fn a_radial_gradient_with_wide_colors_is_in_the_wide_profile() {
    let pdf = render("gradient_radial", "<div style=\"height:100px;background:radial-gradient(color(display-p3 0 1 0), color(rec2020 1 0 0))\"></div>");
    assert!(pdf.contains("/ShadingType 3"), "{pdf}");
    assert!(pdf.contains("/ColorSpace [ /ICCBased"), "{pdf}");
}

#[test]
fn a_gradient_inside_of_srgb_keeps_its_device_colors() {
    let pdf = render("gradient_srgb", "<div style=\"height:100px;background:linear-gradient(red, blue)\"></div>");
    assert!(pdf.contains("/ColorSpace /DeviceRGB"), "{pdf}");
    assert!(!pdf.contains("/ColorSpace [ /ICCBased"), "{pdf}");
    assert_eq!(profiles(&pdf), 1, "only the sRGB profile");
}

#[test]
fn a_mix_of_wide_colors_is_still_wide() {
    let pdf = render("mix_wide", "<p style=\"background: color-mix(in oklab, color(display-p3 1 0 0), color(display-p3 0 1 0))\">x</p>");
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).collect();
    assert!(!fills.is_empty(), "{pdf}");
    assert_eq!(profiles(&pdf), 2, "the wide profile, not clipped to the sRGB one");
}

#[test]
fn a_mix_of_colors_of_srgb_stays_in_srgb() {
    let pdf = render("mix_srgb", "<p style=\"background: color-mix(in srgb, red, blue)\">x</p>");
    assert_eq!(profiles(&pdf), 1, "{pdf}");
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
    assert!(dump.contains("\"rec2020\"") || dump.contains("\"xyz\""), "{dump}");
}
