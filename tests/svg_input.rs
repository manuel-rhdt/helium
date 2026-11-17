//! A standalone SVG file as the input: one page of the size of the drawing, with HTML and MathML
//! in its `foreignObject` elements.

mod common;

struct Rendered {
    /// The PDF with uncompressed content streams and object streams expanded, as text.
    pdf: String,
    /// The size of the first page in points, rounded to tenths.
    page: (f64, f64),
    log: String,
}

fn render_with(name: &str, extension: &str, svg: &str, extra: &[&str]) -> Rendered {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("svg_input__{}.{}", name, extension));
    let output = dir.join(format!("svg_input__{}.pdf", name));
    std::fs::write(&input, svg).unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts"])
        .args(extra)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    let pdf = String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(&output).unwrap()))
        .into_owned();
    let start = pdf.find("/MediaBox").expect("a media box") + "/MediaBox".len();
    let numbers: Vec<f64> = pdf[start..]
        .split(']')
        .next()
        .unwrap()
        .trim_start_matches(|c: char| c == ' ' || c == '[')
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let round = |x: f64| (x * 10.0).round() / 10.0;
    let page = (round(numbers[2] - numbers[0]), round(numbers[3] - numbers[1]));
    Rendered { pdf, page, log }
}

fn render(name: &str, svg: &str) -> Rendered {
    render_with(name, "svg", svg, &[])
}

const NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";
const XHTML: &str = "xmlns=\"http://www.w3.org/1999/xhtml\"";

#[test]
fn the_page_has_the_size_of_the_drawing() {
    // 300 x 160 pixels are 225 x 120 points
    let r = render("size", &format!("<svg {NS} width=\"300\" height=\"160\"><rect width=\"10\" height=\"10\"/></svg>"));
    assert_eq!(r.page, (225.0, 120.0), "{}", r.log);
    // without a width and height, the view box is the size
    let r = render("view_box", &format!("<svg {NS} viewBox=\"0 0 400 200\"><rect width=\"10\" height=\"10\"/></svg>"));
    assert_eq!(r.page, (300.0, 150.0), "{}", r.log);
    // physical units
    let r = render("units", &format!("<svg {NS} width=\"100mm\" height=\"50mm\"><rect width=\"10\" height=\"10\"/></svg>"));
    assert_eq!(r.page, (283.5, 141.7), "{}", r.log);
}

#[test]
fn the_prolog_does_not_matter() {
    let r = render(
        "prolog",
        &format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \
             \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg {NS} width=\"40\" height=\"20\">\
             <rect width=\"10\" height=\"10\"/></svg>"
        ),
    );
    assert_eq!(r.page, (30.0, 15.0), "{}", r.log);
    assert_eq!(r.pdf.matches("/Type /Page\n").count().max(1), 1);
}

#[test]
fn it_is_recognized_by_the_content_and_can_be_forced() {
    let svg = format!("<svg {NS} width=\"40\" height=\"20\"><rect width=\"10\" height=\"10\"/></svg>");
    // an unusual extension: found by the first element
    assert_eq!(render_with("sniff", "xml", &svg, &[]).page, (30.0, 15.0));
    // `--html` reads it as an HTML document: A4
    assert_eq!(render_with("html", "svg", &svg, &["--html"]).page, (595.3, 841.9));
}

#[test]
fn html_and_mathml_in_a_foreign_object_are_laid_out() {
    let r = render(
        "foreign",
        &format!(
            "<svg {NS} width=\"300\" height=\"100\">\
               <foreignObject x=\"10\" y=\"10\" width=\"280\" height=\"40\">\
                 <div {XHTML}>Energy <math xmlns=\"http://www.w3.org/1998/Math/MathML\">\
                   <mi>E</mi><mo>=</mo><mi>m</mi><msup><mi>c</mi><mn>2</mn></msup></math></div>\
               </foreignObject>\
             </svg>"
        ),
    );
    // the SVG and the content of its foreignObject are forms, the text is real text
    assert_eq!(r.pdf.matches("/Subtype /Form").count(), 2, "{}", r.log);
    assert!(r.pdf.contains("/Font << /F"), "{}", r.pdf);
    // the formula is set in the math font, which is embedded. (It has no `Formula` tag: the
    // structure tree does not reach into a foreignObject.)
    assert!(r.pdf.contains("STIXTwoMath"), "{}", r.pdf);
    assert!(!r.pdf.contains("/Formula"), "tags inside of a foreignObject are now supported: update the README");
}

#[test]
fn style_elements_of_the_svg_apply() {
    // the rule makes the rectangle red: `1 0 0 rg`; without it the fill is black
    let r = render(
        "style",
        &format!("<svg {NS} width=\"40\" height=\"20\"><style>.a {{ fill: #f00 }}</style><rect class=\"a\" width=\"10\" height=\"10\"/></svg>"),
    );
    assert!(r.pdf.contains("1 0 0 rg"), "{}", r.pdf);
    // inside a CDATA section, as written by many editors
    let r = render(
        "cdata",
        &format!("<svg {NS} width=\"40\" height=\"20\"><style><![CDATA[.a {{ fill: #f00 }}]]></style><rect class=\"a\" width=\"10\" height=\"10\"/></svg>"),
    );
    assert!(r.pdf.contains("1 0 0 rg"), "{}", r.pdf);
    // another language than CSS is not read as CSS
    let r = render(
        "style_type",
        &format!("<svg {NS} width=\"40\" height=\"20\"><style type=\"text/x-other\">.a {{ fill: #f00 }}</style><rect class=\"a\" width=\"10\" height=\"10\"/></svg>"),
    );
    assert!(!r.pdf.contains("1 0 0 rg"), "{}", r.pdf);
}

#[test]
fn an_empty_html_element_does_not_hold_what_follows() {
    // `<div/>` is empty in XML. The HTML parser would take it for a start tag, and the yellow
    // background would be behind the paragraph.
    let r = render(
        "empty",
        &format!(
            "<svg {NS} width=\"200\" height=\"50\"><foreignObject width=\"200\" height=\"50\">\
               <div {XHTML}><div style=\"background:#fc0\"/><p style=\"margin:4px 0\">second</p></div>\
             </foreignObject></svg>"
        ),
    );
    // the empty div has no height: whatever is painted with its color has none either
    for line in r.pdf.lines().filter(|line| line.contains("1 0.8 0 rg")) {
        assert!(line.contains(" 0 re f"), "the background is not empty: {}", line);
    }
}
