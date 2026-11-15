//! `<svg>` elements in HTML: replaced boxes whose contents are drawn as vector graphics, with the
//! style sheets of the document applied to them.

mod common;

use std::path::Path;

struct Rendered {
    /// The PDF with uncompressed content streams, as text.
    pdf: String,
    /// The same with the objects of the object streams written out (fonts, resources).
    objects: String,
    /// The border boxes (x, y, width, height in points) of the `<svg>` elements in tree order.
    svgs: Vec<[f64; 4]>,
    log: String,
}

fn boxes_of(node: &serde_json::Value, out: &mut Vec<[f64; 4]>) {
    if node["tag"] == "svg" && node["border_box"].is_object() {
        let b = &node["border_box"];
        let get = |key: &str| b[key].as_f64().unwrap() * 0.75;
        out.push([get("x"), get("y"), get("width"), get("height")]);
    }
    for child in node["children"].as_array().into_iter().flatten() {
        boxes_of(child, out);
    }
}

fn render(name: &str, html: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("inline_svg__{}.html", name));
    let output = dir.join(format!("inline_svg__{}.pdf", name));
    let layout = dir.join(format!("inline_svg__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{}", log);
    let dump: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    let mut svgs = vec![];
    boxes_of(&dump["document"], &mut svgs);
    let bytes = std::fs::read(output).unwrap();
    Rendered {
        pdf: String::from_utf8_lossy(&bytes).into_owned(),
        objects: String::from_utf8_lossy(&common::expand_object_streams(&bytes)).into_owned(),
        svgs,
        log,
    }
}

const PAGE: &str = "<style>@page{width:300pt;height:300pt;margin:20pt}</style>";

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 0.1, "{} != {}", actual, expected);
}

fn count(pdf: &str, needle: &str) -> usize {
    pdf.matches(needle).count()
}

#[test]
fn an_svg_element_is_a_box_of_its_size() {
    let r = render(
        "size",
        &format!("{PAGE}<svg width=\"40\" height=\"20\"><rect width=\"40\" height=\"20\"/></svg>"),
    );
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    assert_close(r.svgs[0][2], 30.0);
    assert_close(r.svgs[0][3], 15.0);
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 1, "{}", r.log);
}

#[test]
fn the_size_comes_from_the_view_box_or_from_css() {
    let r = render(
        "view_box",
        &format!(
            "{PAGE}<style>.w{{width:80px}}</style>\
             <svg viewBox=\"0 0 40 20\"><rect width=\"40\" height=\"20\"/></svg>\
             <svg class=\"w\" viewBox=\"0 0 40 20\"><rect width=\"40\" height=\"20\"/></svg>"
        ),
    );
    assert_eq!(r.svgs.len(), 2, "{}", r.log);
    assert_close(r.svgs[0][2], 30.0);
    assert_close(r.svgs[0][3], 15.0);
    // the other side follows the proportions
    assert_close(r.svgs[1][2], 60.0);
    assert_close(r.svgs[1][3], 30.0);
}

#[test]
fn the_style_sheets_of_the_document_apply() {
    let r = render(
        "cascade",
        &format!(
            "{PAGE}<style>\
               rect {{ fill: #111111 }}\
               svg .a {{ fill: #cc0000 }}\
               .a.b {{ fill: #00cc00 }}\
               #c {{ fill: #0000cc }}\
             </style>\
             <svg width=\"40\" height=\"20\">\
               <rect width=\"5\" height=\"5\"/>\
               <rect class=\"a\" width=\"5\" height=\"5\"/>\
               <rect class=\"a b\" width=\"5\" height=\"5\"/>\
               <rect class=\"a b\" id=\"c\" width=\"5\" height=\"5\"/>\
             </svg>"
        ),
    );
    // the rule with the higher specificity wins, whatever the order in the style sheet is
    for color in ["0.07 0.07 0.07 rg", "0.8 0 0 rg", "0 0.8 0 rg", "0 0 0.8 rg"] {
        assert_eq!(count(&r.pdf, color), 1, "{}: {}\n{}", color, r.pdf, r.log);
    }
}

#[test]
fn css_beats_attributes_and_the_style_attribute_beats_css() {
    let r = render(
        "order",
        &format!(
            "{PAGE}<style>.a {{ fill: #cc0000 }}</style>\
             <svg width=\"40\" height=\"20\">\
               <rect fill=\"#0000cc\" width=\"5\" height=\"5\"/>\
               <rect class=\"a\" fill=\"#0000cc\" width=\"5\" height=\"5\"/>\
               <rect class=\"a\" style=\"fill:#00cc00\" width=\"5\" height=\"5\"/>\
             </svg>"
        ),
    );
    assert_eq!(count(&r.pdf, "0 0 0.8 rg"), 1, "{}", r.pdf);
    assert_eq!(count(&r.pdf, "0.8 0 0 rg"), 1);
    assert_eq!(count(&r.pdf, "0 0.8 0 rg"), 1);
}

#[test]
fn current_color_is_the_color_of_the_page() {
    let r = render(
        "color",
        &format!(
            "{PAGE}<style>p {{ color: #0000cc }} .x {{ color: #cc0000 }}</style>\
             <p><svg width=\"40\" height=\"20\"><rect fill=\"currentColor\" width=\"5\" height=\"5\"/></svg>\
             <svg class=\"x\" width=\"40\" height=\"20\"><rect fill=\"currentColor\" width=\"5\" height=\"5\"/></svg></p>"
        ),
    );
    assert_eq!(count(&r.pdf, "0 0 0.8 rg"), 1, "{}", r.pdf);
    assert_eq!(count(&r.pdf, "0.8 0 0 rg"), 1);
}

#[test]
fn svg_elements_use_what_other_svg_elements_define() {
    // an icon sprite: a hidden `<svg>` with a symbol and a gradient
    let r = render(
        "sprite",
        &format!(
            "{PAGE}<svg style=\"display:none\">\
               <symbol id=\"dot\" viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"5\"/></symbol>\
               <linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"#f00\"/><stop offset=\"1\" stop-color=\"#00f\"/></linearGradient>\
             </svg>\
             <svg width=\"40\" height=\"20\"><use href=\"#dot\" width=\"20\" height=\"20\"/>\
               <rect x=\"20\" width=\"20\" height=\"20\" fill=\"url(#g)\"/></svg>"
        ),
    );
    // the hidden `<svg>` is not a box, the other one draws both
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/ShadingType 2"), 1);
}

#[test]
fn the_contents_of_an_svg_are_not_laid_out() {
    let r = render(
        "contents",
        &format!(
            "{PAGE}<p>a<svg width=\"40\" height=\"20\"><text x=\"1\" y=\"10\">inside</text></svg>b</p>"
        ),
    );
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    // the text is shown by the SVG code with the embedded font of the document, with its own text
    // operators, and there is nothing in the layout that could be broken into lines
    assert!(count(&r.pdf, " Tj") >= 1, "{}", r.pdf);
    assert!(count(&r.objects, "/Type /Font") >= 1, "{}", r.objects);
}

#[test]
fn an_svg_without_a_size_is_as_big_as_its_contents() {
    // no `width`, `height` or `viewBox`: the parser takes the bounding box of the contents
    let r = render("no_size", &format!("{PAGE}<svg><rect width=\"10\" height=\"10\"/></svg>"));
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    assert_close(r.svgs[0][2], 7.5);
    assert_close(r.svgs[0][3], 7.5);
}

#[test]
fn text_is_shaped_with_the_fonts_of_the_document() {
    // Arabic: right-to-left, joined letters that need the glyphs of the bundled Noto font
    let r = render(
        "text",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"60\">\
               <text x=\"100\" y=\"40\" font-size=\"30\" text-anchor=\"middle\">مرحبا</text>\
             </svg>"
        ),
    );
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    // shown with the embedded font of the document (Noto Naskh Arabic), not drawn as outlines
    assert!(count(&r.pdf, " Tj") >= 1, "{}", r.pdf);
    assert!(count(&r.pdf, " c ") < 5, "{}", r.pdf);
    assert!(r.objects.contains("NotoNaskhArabic"), "{}", r.objects);
    assert!(!r.log.contains("SVG:"), "{}", r.log);
}

#[test]
fn svg_text_is_text_that_can_be_extracted_and_shares_the_fonts_of_the_page() {
    let r = render(
        "shared_font",
        &format!(
            "{PAGE}<p style=\"font-family: serif\">page</p>\
             <svg width=\"200\" height=\"40\"><text x=\"5\" y=\"30\" font-size=\"24\" \
             style=\"font-family: serif\">svg</text></svg>"
        ),
    );
    // one embedded font for both, and its map back to text covers the glyphs of the SVG
    assert_eq!(count(&r.objects, "/Subtype /Type0"), 1, "{}", r.objects);
    assert!(count(&r.pdf, " Tj") >= 2, "{}", r.pdf);
    assert!(count(&r.objects, "/ToUnicode") >= 1);
}

#[test]
fn gradient_text_is_drawn_as_outlines() {
    // a text operator paints with a color, so a gradient goes through the shapes of the glyphs
    let r = render(
        "gradient_text",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"60\"><defs><linearGradient id=\"g\">\
               <stop offset=\"0\" stop-color=\"red\"/><stop offset=\"1\" stop-color=\"blue\"/>\
             </linearGradient></defs>\
             <text x=\"5\" y=\"40\" font-size=\"30\" fill=\"url(#g)\">outline</text></svg>"
        ),
    );
    assert_eq!(count(&r.pdf, " Tj"), 0, "{}", r.pdf);
    assert!(count(&r.pdf, " c ") > 10, "{}", r.pdf);
}

#[test]
fn stroked_text_uses_the_stroke_text_rendering_mode() {
    let r = render(
        "stroked_text",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"60\">\
             <text x=\"5\" y=\"40\" font-size=\"30\" fill=\"none\" stroke=\"red\">outline</text>\
             <text x=\"5\" y=\"55\" font-size=\"20\" fill=\"blue\" stroke=\"red\">both</text></svg>"
        ),
    );
    assert!(count(&r.pdf, "BT 1 Tr") >= 1, "{}", r.pdf);
    assert!(count(&r.pdf, "BT 2 Tr") >= 1, "{}", r.pdf);
}
