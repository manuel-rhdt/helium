//! Numeric and relative `font-weight`: which of the faces is embedded.

mod common;

fn base_fonts(name: &str, html: &str) -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("font_weight__{}.html", name));
    let output = dir.join(format!("font_weight__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pdf = String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned();
    let mut fonts: Vec<String> = pdf
        .split("/BaseFont /")
        .skip(1)
        .map(|rest| rest.split(|c: char| c.is_whitespace() || c == '/').next().unwrap().to_owned())
        .map(|name| name.split('+').last().unwrap().trim_end_matches("-Identity-H").to_owned())
        .collect();
    fonts.sort();
    fonts.dedup();
    fonts
}

fn paragraph(style: &str) -> String {
    format!("<body style=\"font-family: Helvetica\"><p style=\"{style}\">weight</p></body>")
}

#[test]
fn light_weights_use_the_regular_face_and_heavy_ones_the_bold_face() {
    for (weight, face) in [(100, "TeXGyreHeros-Regular"), (400, "TeXGyreHeros-Regular"), (500, "TeXGyreHeros-Regular"), (600, "TeXGyreHeros-Bold"), (900, "TeXGyreHeros-Bold")] {
        let fonts = base_fonts(&format!("n{weight}"), &paragraph(&format!("font-weight: {weight}")));
        assert_eq!(fonts, [face], "weight {weight}");
    }
}

#[test]
fn keywords_and_relative_weights() {
    assert_eq!(base_fonts("bold", &paragraph("font-weight: bold")), ["TeXGyreHeros-Bold"]);
    assert_eq!(base_fonts("normal", &paragraph("font-weight: normal")), ["TeXGyreHeros-Regular"]);
    // bolder than 400 is 700, lighter than 700 is 400
    let html = "<body style=\"font-family: Helvetica\"><p style=\"font-weight: bolder\">a</p></body>";
    assert_eq!(base_fonts("bolder", html), ["TeXGyreHeros-Bold"]);
    let html = "<body style=\"font-family: Helvetica; font-weight: 700\"><p style=\"font-weight: lighter\">a</p></body>";
    assert_eq!(base_fonts("lighter", html), ["TeXGyreHeros-Regular"]);
}

#[test]
fn invalid_weights_are_ignored() {
    // the declaration is dropped, the inherited bold weight stays
    let html = "<body style=\"font-family: Helvetica; font-weight: bold\"><p style=\"font-weight: 1500\">a</p></body>";
    assert_eq!(base_fonts("invalid", html), ["TeXGyreHeros-Bold"]);
}

/// The font family of the root element is inherited by the body; the body has none of its own.
#[test]
fn the_font_family_of_the_root_element_reaches_the_body() {
    // the default family is sans-serif, which is Helvetica without system fonts
    let on_html = base_fonts("family_html", "<style>html { font-family: serif }</style><p>text</p>");
    assert_eq!(on_html, ["TeXGyreTermes-Regular"]);
    let on_body = base_fonts("family_body", "<style>body { font-family: serif }</style><p>text</p>");
    assert_eq!(on_body, ["TeXGyreTermes-Regular"]);
    // the body's own value wins over the root element's
    let both = base_fonts(
        "family_both",
        "<style>html { font-family: monospace } body { font-family: serif }</style><p>text</p>",
    );
    assert_eq!(both, ["TeXGyreTermes-Regular"]);
}
