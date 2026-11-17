//! `--crop`: one page that is as large as the content: no pagination, no page margins, no `@page`,
//! and as wide as the content needs (what shrink-to-fit comes to when nothing limits it).

mod common;

struct Rendered {
    /// The number of pages.
    pages: usize,
    /// The size of the first page in points, rounded to hundredths.
    size: (f64, f64),
}

fn crop(name: &str, extension: &str, text: &str) -> Rendered {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("crop__{}.{}", name, extension));
    let output = dir.join(format!("crop__{}.pdf", name));
    std::fs::write(&input, text).unwrap();
    let result = common::helium()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts", "--crop"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
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
    let round = |x: f64| (x * 100.0).round() / 100.0;
    Rendered {
        pages: pdf.matches("/MediaBox").count(),
        // (the corners of the box may be given in any order)
        size: (round((numbers[2] - numbers[0]).abs()), round((numbers[3] - numbers[1]).abs())),
    }
}

fn html(name: &str, text: &str) -> Rendered {
    crop(name, "html", text)
}

#[test]
fn the_page_is_the_box_of_the_content() {
    // 300 x 100 pixels are 225 x 75 points; the margin of the body is not part of it
    let r = html("fixed", "<div style=\"width:300px;height:100px;background:#fc0\">fixed</div>");
    assert_eq!(r.size, (225.0, 75.0));
    assert_eq!(r.pages, 1);
    let r = html("fixed_no_margin", "<body style=\"margin:0\"><div style=\"width:300px;height:100px;background:#fc0\"></div></body>");
    assert_eq!(r.size, (225.0, 75.0));
}

#[test]
fn the_width_is_what_the_content_needs() {
    // a block without a width is as wide as its widest part, here the box of 200 pixels
    let r = html(
        "shrink",
        "<body style=\"margin:0\"><p style=\"margin:0;height:10px\"></p>\
         <div style=\"width:200px;height:40px;background:#fc0\"></div></body>",
    );
    assert_eq!(r.size.0, 150.0);
    // not the width of a page
    assert!(r.size.0 < 595.0);
}

#[test]
fn text_is_not_broken_into_lines() {
    // there is no width to break at: the paragraph is one line of 20 pixels, however long it is
    let words = "word ".repeat(80);
    let r = html(
        "one_line",
        &format!("<body style=\"margin:0\"><p style=\"margin:0;font:16px/20px sans-serif\">{words}</p></body>"),
    );
    assert_eq!(r.size.1, 15.0);
    assert!(r.size.0 > 595.0, "{:?}", r.size);
    // a width of the document is kept
    let r = html(
        "wrapped",
        &format!("<body style=\"margin:0;width:200px\"><p style=\"margin:0;font:16px/20px sans-serif\">{words}</p></body>"),
    );
    assert_eq!(r.size.0, 150.0);
    assert!(r.size.1 > 15.0, "{:?}", r.size);
}

#[test]
fn page_rules_and_margins_do_not_apply() {
    let r = html(
        "page_rules",
        "<style>@page { size: 500pt 500pt; margin: 100pt; @top-center { content: \"head\" } } body { margin: 0 }</style>\
         <div style=\"width:120px;height:40px;background:#fc0\"></div>",
    );
    assert_eq!(r.size, (90.0, 30.0));
    assert_eq!(r.pages, 1);
}

#[test]
fn content_that_is_taller_than_a_page_is_one_page() {
    let r = html("tall", "<div style=\"width:100px;height:3000px;background:#ccc\"></div>");
    assert_eq!(r.pages, 1);
    assert_eq!(r.size, (75.0, 2250.0));
    // and the breaks in the content do not break the page
    let r = html(
        "breaks",
        "<body style=\"margin:0\"><div style=\"width:100px;height:40px;break-after:page\"></div>\
         <div style=\"width:100px;height:40px\"></div></body>",
    );
    assert_eq!(r.pages, 1);
}

#[test]
fn a_drawing_has_the_size_of_the_drawing() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"300\" height=\"160\"><rect width=\"10\" height=\"10\"/></svg>";
    assert_eq!(crop("svg", "svg", svg).size, (225.0, 120.0));
}
