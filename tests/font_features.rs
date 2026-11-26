//! `font-feature-settings`, `font-kerning` and `font-variant`: the features of the font that the
//! author chooses change the glyphs, which shows in the width of the text.

mod common;

use std::path::Path;

/// The widths (in CSS pixels) of the `<span>` elements of `body`, in tree order, set in TeX Gyre
/// Termes.
fn widths(name: &str, body: &str) -> Vec<f64> {
    widths_and_log(name, body, true)
}

fn widths_and_log(name: &str, body: &str, valid: bool) -> Vec<f64> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("font_features__{}.html", name));
    let output = dir.join(format!("font_features__{}.pdf", name));
    let layout = dir.join(format!("font_features__{}.json", name));
    std::fs::write(
        &input,
        format!("<style>body{{font:20px 'TeX Gyre Termes'}}span{{display:inline-block}}</style>{}", body),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    // (the user agent style sheet has declarations that are not supported yet)
    assert!(
        !valid || !log.lines().any(|line| line.contains("Error while parsing declaration \"font")),
        "{}",
        log
    );
    let dump: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    fn collect(node: &serde_json::Value, out: &mut Vec<f64>) {
        if node["tag"] == "span" {
            out.push(node["border_box"]["width"].as_f64().unwrap());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            collect(child, out);
        }
    }
    let mut out = vec![];
    collect(&dump["document"], &mut out);
    out
}

fn differ(a: f64, b: f64) -> bool {
    (a - b).abs() > 0.01
}

#[test]
fn small_caps_are_narrower_than_capitals() {
    let w = widths(
        "small_caps",
        "<span>SMALL</span><span style=\"font-variant-caps:small-caps\">SMALL</span>\
         <span style=\"font-variant-caps:small-caps\">small</span>\
         <span style=\"font-variant:small-caps\">small</span>\
         <span style=\"font:small-caps 20px 'TeX Gyre Termes'\">small</span>\
         <span style=\"font-feature-settings:'smcp'\">small</span>\
         <span>small</span>",
    );
    // capitals are not small capitals, whatever the property says
    assert!(!differ(w[0], w[1]), "{w:?}");
    assert!(w[2] < w[0], "{w:?}");
    // all ways of asking for them give the same text
    for other in &w[3..6] {
        assert!(!differ(w[2], *other), "{w:?}");
    }
    assert!(differ(w[2], w[6]), "{w:?}");
}

#[test]
fn kerning_can_be_turned_off_in_two_ways() {
    let w = widths(
        "kerning",
        "<span>AVAVAV</span><span style=\"font-kerning:none\">AVAVAV</span>\
         <span style=\"font-feature-settings:'kern' off\">AVAVAV</span>\
         <span style=\"font-kerning:none;font-feature-settings:'kern' 1\">AVAVAV</span>",
    );
    assert!(w[1] > w[0], "{w:?}");
    assert!(!differ(w[1], w[2]), "{w:?}");
    // the settings override the property
    assert!(!differ(w[3], w[0]), "{w:?}");
}

#[test]
fn discretionary_ligatures_are_off_unless_asked_for() {
    let w = widths(
        "ligatures",
        "<span>a--b</span>\
         <span style=\"font-variant-ligatures:discretionary-ligatures\">a--b</span>\
         <span style=\"font-feature-settings:'dlig'\">a--b</span>\
         <span style=\"font-variant:discretionary-ligatures no-common-ligatures\">a--b</span>",
    );
    assert!(differ(w[0], w[1]), "{w:?}");
    assert!(!differ(w[1], w[2]), "{w:?}");
    assert!(!differ(w[1], w[3]), "{w:?}");
}

#[test]
fn the_features_are_inherited_and_the_font_shorthand_resets_them() {
    let w = widths(
        "inherit",
        "<div style=\"font-variant-caps:small-caps\">\
           <span>small</span>\
           <p style=\"margin:0\"><span>small</span></p>\
           <span style=\"font-variant-caps:normal\">small</span>\
           <span style=\"font:20px 'TeX Gyre Termes'\">small</span>\
         </div><span>small</span>",
    );
    assert!(differ(w[0], w[4]), "{w:?}");
    assert!(!differ(w[0], w[1]), "{w:?}");
    assert!(!differ(w[2], w[4]), "{w:?}");
    assert!(!differ(w[3], w[4]), "{w:?}");
}

#[test]
fn invalid_values_are_ignored() {
    let w = widths_and_log(
        "invalid",
        "<span style=\"font-variant-caps:small-caps;font-variant-caps:sideways\">small</span>\
         <span style=\"font-feature-settings:'smcp';font-feature-settings:smcp\">small</span>\
         <span style=\"font-variant-ligatures:common-ligatures no-common-ligatures\">small</span>\
         <span>small</span>",
        false,
    );
    assert!(differ(w[0], w[3]), "{w:?}");
    assert!(differ(w[1], w[3]), "{w:?}");
    assert!(!differ(w[2], w[3]), "{w:?}");
}

/// The text operator of the margin box of a page with `style` for its `@top-center`.
fn margin_text(name: &str, style: &str) -> String {
    margin_text_of(name, "Head", style)
}

fn margin_text_of(name: &str, content: &str, style: &str) -> String {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("font_features__{}.html", name));
    let output = dir.join(format!("font_features__{}.pdf", name));
    std::fs::write(
        &input,
        format!(
            "<style>@page{{width:200pt;height:200pt;margin:40pt;\
             @top-center{{content:\"{}\";font-family:'TeX Gyre Termes';{}}}}}</style><p>x</p>",
            content, style
        ),
    )
    .unwrap();
    let out = common::umbruch()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let pdf = String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned();
    let start = pdf.find("/Artifact << /Type /Pagination >> BDC").expect(&pdf);
    pdf[start..start + pdf[start..].find(" EMC").unwrap()].to_string()
}

#[test]
fn margin_boxes_use_the_features_and_the_letter_spacing_of_their_style() {
    let plain = margin_text("margin_plain", "");
    let caps = margin_text("margin_caps", "font-variant-caps:small-caps");
    let spaced = margin_text("margin_spaced", "letter-spacing:5pt");
    // small capitals are other glyphs
    assert_ne!(plain, caps);
    // the spacing is a kern after every glyph
    assert!(!plain.contains(" -416.7"), "{plain}");
    assert_eq!(spaced.matches("-416.7").count(), 4, "{spaced}");
}

#[test]
fn margin_boxes_apply_text_transform() {
    let upper = margin_text_of("margin_upper", "HEAD", "");
    let transformed = margin_text_of("margin_transformed", "Head", "text-transform:uppercase");
    assert_eq!(upper, transformed);
    assert_ne!(upper, margin_text_of("margin_untransformed", "Head", ""));
}
