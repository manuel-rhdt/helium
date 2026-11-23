//! A colour is written in the space that CSS wrote it in. `rgb()`, `hsl()`, hex codes, names (and an
//! `oklab()` or `oklch()` that sRGB holds) are device RGB, which `DefaultRGB` defines as sRGB in the page and
//! in every form. A colour of `color()` in a space with a profile (Display P3, Rec. 2020, ...) is in an ICC
//! space of that profile. Everything else (`lab()`, `lch()`, `xyz`, an `oklab()` that sRGB cannot hold, a
//! colour outside of the space it was written in) is CIELAB, and a colour with a* or b* beyond 128 (which an
//! iPhone does not show right in Lab) is in the ICC space of Rec. 2020 if that holds it, else its chroma is reduced.

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
    pdf.split_whitespace().filter(|token| *token == "rg" || *token == "RG").count()
}

fn close(found: &[f64], expected: [f64; 3]) -> bool {
    found.iter().zip(expected).all(|(found, expected)| (found - expected).abs() < 0.0006)
}

/// Profiles in the file: the ones that `DefaultRGB` and the colours write are streams with `/N 3`.
fn profiles(pdf: &str) -> usize {
    pdf.matches("/N 3").count()
}

#[test]
fn a_color_of_display_p3_is_in_the_profile_of_display_p3() {
    let pdf = render("p3", "<p style=\"background: color(display-p3 1 0 0)\">x</p>");
    // the black of the text is in the sRGB space too: it is the colour that is not 0 0 0
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).take(1).collect();
    assert!(!fills.is_empty(), "{fills:?}");
    // the components are the ones that CSS wrote
    assert!(close(&fills[0].1, [1.0, 0.0, 0.0]), "{:?}", fills[0]);
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
fn a_color_of_prophoto_is_in_its_own_profile_and_not_clipped() {
    let pdf = render("prophoto", "<p style=\"background: color(prophoto-rgb 0 1 0)\">x</p>");
    // the black of the text is in the sRGB space too: it is the colour that is not 0 0 0
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).take(1).collect();
    assert!(!fills.is_empty(), "{fills:?}");
    assert!(close(&fills[0].1, [0.0, 1.0, 0.0]), "{:?}", fills[0]);
    assert!(pdf.contains("/ICCBased") && !pdf.contains("/CalRGB") && !pdf.contains("/Lab"));
    assert_eq!(profiles(&pdf), 2, "the profile of ProPhoto and the sRGB one");
}

/// The values of the fills in a colour space of the kind `name` (`/Lab`, `/CalRGB`): the page resources have
/// `/CSn [ /Lab ...`, and the content says `/CSn cs a b c sc`.
fn fills_in(pdf: &str, name: &str) -> Vec<Vec<f64>> {
    let spaces: Vec<String> = pdf
        .split("/CS")
        .skip(1)
        .filter(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start().starts_with(&format!("[ {name}")))
        .map(|rest| format!("/CS{}", rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()))
        .collect();
    space_colors(pdf, true).into_iter().filter(|(space, _)| spaces.contains(space)).map(|(_, c)| c).collect()
}

#[test]
fn a_color_outside_of_its_space_is_cielab_and_not_clipped() {
    let pdf = render("outside", "<p style=\"background: color(display-p3 1.2 0 0)\">x</p>");
    let labs = fills_in(&pdf, "/Lab");
    assert!(!labs.is_empty(), "{pdf}");
    assert!(labs[0][0] > 0.0 && labs[0][0] < 100.0 && labs[0][1].abs() <= 128.0 && labs[0][2].abs() <= 128.0, "{labs:?}");
    assert!(!pdf.contains("/CalRGB"));
}

#[test]
fn lab_is_written_as_the_lab_that_it_is() {
    let pdf = render("lab", "<p style=\"background: lab(50 20 30)\">x</p>");
    let labs = fills_in(&pdf, "/Lab");
    assert!(labs.iter().any(|c| close(c, [50.0, 20.0, 30.0])), "{labs:?}");
    // the white point is the D50 one of CSS, and the range is the one that is right on iOS
    assert!(pdf.contains("/WhitePoint") && pdf.contains("/Range"), "{pdf}");
}

#[test]
fn lch_and_an_oklch_that_srgb_cannot_hold_are_cielab() {
    let pdf = render("lch", "<p style=\"background: lch(60 40 100); border: 2px solid oklch(0.7 0.3 150)\">x</p>");
    assert!(fills_in(&pdf, "/Lab").iter().any(|c| close(c, [60.0, -6.9459, 39.3923])), "{:?}", fills_in(&pdf, "/Lab"));
}

#[test]
fn a_color_beyond_128_in_a_or_b_is_in_the_rec2020_profile_if_it_fits() {
    // Rec. 2020 green: a* is about -190
    let pdf = render("beyond", "<p style=\"background: color(rec2020 0 1 0)\">x</p>");
    assert!(fills_in(&pdf, "/Lab").is_empty(), "{pdf}");
    let fills: Vec<_> = space_colors(&pdf, true).into_iter().filter(|(_, c)| c.iter().any(|v| *v != 0.0)).collect();
    assert!(close(&fills[0].1, [0.0, 1.0, 0.0]), "{fills:?}");
    // a colour that Rec. 2020 does not hold either has its chroma reduced, the lightness and the hue stay
    let pdf = render("beyond_far", "<p style=\"background: lab(60 300 100)\">x</p>");
    let labs = fills_in(&pdf, "/Lab");
    assert!(labs.iter().any(|c| close(c, [60.0, 128.0, 42.6667])), "{labs:?}");
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
fn the_syntaxes_of_srgb_are_device_colors_that_the_page_defines_as_srgb() {
    let pdf = render(
        "inside",
        "<p style=\"color: hsl(200 50% 40%); background: #369; border: 2px solid oklch(0.6 0.1 200)\">x</p>",
    );
    assert!(rgb_count(&pdf) >= 3, "device RGB for the text, the background and the border: {pdf}");
    assert!(space_colors(&pdf, true).is_empty() && space_colors(&pdf, false).is_empty());
    assert!(!pdf.contains("/CalRGB") && !pdf.contains("/Lab"));
    assert_eq!(profiles(&pdf), 1, "only the sRGB profile of DefaultRGB");
    assert!(pdf.contains("/DefaultRGB"));
}

#[test]
fn a_form_defines_the_device_colors_too() {
    // an opacity group is a form with resources of its own, which the ones of the page do not reach
    let pdf = render("form", "<p style=\"opacity: 0.5; background: red\">x</p>");
    assert!(pdf.matches("/DefaultRGB").count() >= 2, "{pdf}");
}

#[test]
fn opacity_and_alpha_keep_the_wide_color() {
    // the alpha of a colour is set apart from its colour space
    let pdf = render("alpha", "<p style=\"background: color(display-p3 0 1 0 / 40%); opacity: 0.8\">x</p>");
    assert!(!space_colors(&pdf, true).is_empty(), "{:?}", space_colors(&pdf, true));
    assert_eq!(profiles(&pdf), 2, "the sRGB profile and the one of Display P3");
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
    // the result is in the space of the mix, which nothing was written in: CIELAB, not clipped to sRGB
    assert!(!fills_in(&pdf, "/Lab").is_empty(), "{pdf}");
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
    assert!(dump.contains("\"rec2020\""), "{dump}");
}
