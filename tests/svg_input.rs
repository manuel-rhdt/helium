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
    // points are points and pixels are 3/4 point, as in HTML
    let r = render("pt", &format!("<svg {NS} width=\"200pt\" height=\"100pt\"><rect width=\"10\" height=\"10\"/></svg>"));
    assert_eq!(r.page, (200.0, 100.0), "{}", r.log);
    // physical units
    let r = render("units",&format!("<svg {NS} width=\"100mm\" height=\"50mm\"><rect width=\"10\" height=\"10\"/></svg>"));
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

fn run(name: &str, extension: &str, text: &str, extra: &[&str]) -> std::process::Output {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("svg_input__{}.{}", name, extension));
    let output = dir.join(format!("svg_input__{}.pdf", name));
    std::fs::write(&input, text).unwrap();
    common::helium().unwrap().args(extra).arg(&input).arg(&output).output().unwrap()
}

#[test]
fn the_parser_follows_the_extension() {
    let svg = format!("<svg {NS} width=\"40\" height=\"20\"><rect width=\"10\" height=\"10\"/></svg>");
    // `.svg`, `.xml` and `.xhtml` are read as XML: the drawing is the page
    for extension in ["svg", "xml", "SVG"] {
        assert_eq!(render_with(&format!("ext_{extension}"), extension, &svg, &[]).page, (30.0, 15.0), "{extension}");
    }
    // any other extension is read by the HTML parser: a document with an `<svg>` in it
    const A4: (f64, f64) = (595.3, 841.9);
    assert_eq!(render_with("ext_txt", "txt", &svg, &[]).page, A4);
    assert_eq!(render_with("ext_html", "html", &svg, &[]).page, A4);
    // the options decide against the extension
    assert_eq!(render_with("flag_xml", "txt", &svg, &["--xml"]).page, (30.0, 15.0));
    assert_eq!(render_with("flag_html", "svg", &svg, &["--html"]).page, A4);
}

#[test]
fn xml_is_strict_and_html_is_not() {
    // an element that is never closed
    let bad = format!("<svg {NS} width=\"40\" height=\"20\"><rect width=\"10\" height=\"10\"></svg>");
    let out = run("bad", "svg", &bad, &[]);
    assert!(!out.status.success());
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(message.contains("bad.svg") && message.contains("not well-formed XML"), "{message}");
    // the HTML parser repairs it
    assert!(run("bad_html", "svg", &bad, &["--html"]).status.success());
    // an XML document that is neither SVG nor XHTML
    let out = run("other", "xml", "<doc/>", &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("neither SVG nor XHTML"));
}

#[test]
fn namespace_prefixes_are_resolved() {
    // what the HTML parser cannot read: prefixes for SVG, XHTML and MathML
    let svg = "<s:svg xmlns:s=\"http://www.w3.org/2000/svg\" xmlns:x=\"http://www.w3.org/1999/xhtml\" \
               xmlns:m=\"http://www.w3.org/1998/Math/MathML\" width=\"100\" height=\"40\">\
               <s:style>.a { fill: #f00 }</s:style><s:rect class=\"a\" width=\"10\" height=\"10\"/>\
               <s:foreignObject width=\"100\" height=\"40\"><x:div><m:math><m:mi>x</m:mi></m:math></x:div>\
               </s:foreignObject></s:svg>";
    let r = render("prefixed", svg);
    assert_eq!(r.page, (75.0, 30.0), "{}", r.log);
    assert!(r.pdf.contains("1 0 0 rg"), "{}", r.pdf);
    assert!(r.pdf.contains("STIXTwoMath"), "{}", r.pdf);
    assert_eq!(r.pdf.matches("/Subtype /Form").count(), 2, "{}", r.log);
}

#[test]
fn an_xhtml_document_is_laid_out_as_it_is() {
    let xhtml = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><style>@page { size: 200pt 100pt }</style></head>\
                 <body><p>Hello</p></body></html>";
    assert_eq!(render_with("xhtml", "xhtml", xhtml, &[]).page, (200.0, 100.0));
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
    let svg = format!(
        "<svg {NS} width=\"200\" height=\"50\"><foreignObject width=\"200\" height=\"50\">\
           <div {XHTML}><div style=\"background:#fc0\"/><p style=\"margin:4px 0\">second</p></div>\
         </foreignObject></svg>"
    );
    let r = render("empty", &svg);
    // the empty div has no height: whatever is painted with its color has none either
    let filled: Vec<_> = r.pdf.lines().filter(|line| line.contains("1 0.8 0 sc")).collect();
    assert!(!filled.is_empty(), "the background is not painted at all");
    for line in filled {
        assert!(line.contains(" 0 re f"), "the background is not empty: {}", line);
    }
}
