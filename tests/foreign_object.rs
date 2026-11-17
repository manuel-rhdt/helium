//! `<foreignObject>`: HTML and MathML inside of SVG, laid out with the style sheets of the page.

mod common;

use std::path::Path;

struct Rendered {
    /// The PDF with uncompressed content streams, as text.
    pdf: String,
    /// The border boxes (x, y, width, height in points) of the `<svg>` elements in tree order.
    svgs: Vec<[f64; 4]>,
    log: String,
}

fn boxes_of(node: &serde_json::Value, out: &mut Vec<[f64; 4]>) {
    if (node["tag"] == "svg" || node["tag"] == "img") && node["border_box"].is_object() {
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
    let input = dir.join(format!("foreign_object__{}.html", name));
    let output = dir.join(format!("foreign_object__{}.pdf", name));
    let layout = dir.join(format!("foreign_object__{}.json", name));
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
    Rendered {
        pdf: String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned(),
        svgs,
        log,
    }
}

const PAGE: &str = "<style>@page{width:300pt;height:300pt;margin:20pt}</style>";

fn count(pdf: &str, needle: &str) -> usize {
    pdf.matches(needle).count()
}

#[test]
fn html_inside_of_svg_is_laid_out_in_the_rectangle() {
    let r = render(
        "html",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"100\">\
               <foreignObject x=\"20\" y=\"10\" width=\"100\" height=\"40\"><p>Hello world</p></foreignObject>\
             </svg>"
        ),
    );
    assert_eq!(r.svgs.len(), 1, "{}", r.log);
    // the SVG and the content of its foreignObject are forms; the form of the content is as big
    // as the element (100 x 40 pixels are 75 x 30 points) and clips what does not fit
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 2, "{}", r.log);
    assert_eq!(count(&r.pdf, "/BBox [ 0 0 75 30 ]"), 1, "{}", r.pdf);
    // the text is real text with an embedded font
    assert!(r.pdf.contains("/Font << /F0"), "{}", r.pdf);
    assert!(r.pdf.contains("BT"), "{}", r.pdf);
}

#[test]
fn the_style_sheets_of_the_page_apply_to_the_content() {
    let r = render(
        "css",
        &format!(
            "{PAGE}<style>.card {{ background: #cc0000 }} svg .card b {{ color: #0000cc }}</style>\
             <svg width=\"200\" height=\"100\">\
               <foreignObject width=\"100\" height=\"40\"><div class=\"card\"><b>x</b></div></foreignObject>\
             </svg>"
        ),
    );
    // the descendant selector reaches through the SVG elements into the HTML
    assert_eq!(count(&r.pdf, "0.8 0 0 sc"), 1, "{}", r.pdf);
    assert!(r.pdf.contains("0 0 0.8 sc"), "{}", r.pdf);
}

#[test]
fn the_content_inherits_from_the_page() {
    let r = render(
        "inherit",
        &format!(
            "{PAGE}<style>p {{ color: #00cc00; font-size: 20px }}</style>\
             <p><svg width=\"200\" height=\"100\">\
               <foreignObject width=\"150\" height=\"40\"><span>x</span></foreignObject>\
             </svg></p>"
        ),
    );
    // the text color is the one of the `<p>` that the `<svg>` is in
    assert!(r.pdf.contains("0 0.8 0 sc"), "{}", r.pdf);
}

#[test]
fn mathml_inside_of_svg_is_typeset() {
    let r = render(
        "math",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"100\">\
               <foreignObject width=\"150\" height=\"60\">\
                 <math><mfrac><mi>a</mi><mn>2</mn></mfrac><mo>=</mo><msqrt><mi>x</mi></msqrt></math>\
               </foreignObject>\
             </svg>"
        ),
    );
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 2, "{}", r.log);
    // the fraction bar and the radical: rectangles or paths, and a math font
    assert!(r.pdf.contains("/Font << /F"), "{}", r.pdf);
    assert!(!r.log.contains("could not be laid out"), "{}", r.log);
}

#[test]
fn content_that_does_not_fit_is_cut_off() {
    let r = render(
        "overflow",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"100\">\
               <foreignObject width=\"100\" height=\"20\"><p>one</p><p>two</p><p>three</p><p>four</p></foreignObject>\
             </svg>"
        ),
    );
    // the form clips to its bounding box, which is the size of the element
    assert_eq!(count(&r.pdf, "/BBox [ 0 0 75 15 ]"), 1, "{}", r.pdf);
}

#[test]
fn transforms_of_the_svg_apply_to_the_content() {
    let r = render(
        "transform",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"100\"><g transform=\"rotate(10 50 50)\">\
               <foreignObject x=\"10\" y=\"10\" width=\"100\" height=\"40\"><p>x</p></foreignObject></g></svg>"
        ),
    );
    // a rotation by 10 degrees: cos 0.98 and sin 0.17
    assert!(r.pdf.contains("0.98 0.17 -0.17 0.98"), "{}", r.pdf);
}

#[test]
fn svg_inside_of_html_inside_of_svg() {
    let r = render(
        "nested",
        &format!(
            "{PAGE}<svg width=\"200\" height=\"100\">\
               <foreignObject width=\"150\" height=\"60\">\
                 <p>text <svg width=\"20\" height=\"20\"><circle cx=\"10\" cy=\"10\" r=\"8\" fill=\"#cc0000\"/>\
                   <foreignObject width=\"20\" height=\"10\"><span>in</span></foreignObject></svg></p>\
               </foreignObject>\
             </svg>"
        ),
    );
    // outer SVG, its content, the inner SVG and its content
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 4, "{}\n{}", r.log, r.pdf);
    assert!(r.pdf.contains("0.8 0 0 rg"), "{}", r.pdf);
}

#[test]
fn an_svg_file_cannot_have_foreign_content() {
    // `<img src="x.svg">`: there is no document that knows the content
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\">\
               <foreignObject width=\"40\" height=\"20\"><p xmlns=\"http://www.w3.org/1999/xhtml\">x</p></foreignObject></svg>";
    let encoded: String = svg
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{:02X}", b) })
        .collect();
    let r = render("file", &format!("{PAGE}<img src=\"data:image/svg+xml,{encoded}\">"));
    assert!(r.log.contains("foreignObject"), "{}", r.log);
}
