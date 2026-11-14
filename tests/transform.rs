//! The CSS `transform` property.

mod common;

use std::path::Path;

fn render(name: &str, html: &str) -> (String, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("transform__{}.html", name));
    let output = dir.join(format!("transform__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    (String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned(), log)
}

const PAGE: &str = "<style>@page{width:300pt;height:300pt;margin:20pt} body{margin:0}</style>";

#[test]
fn a_translation_moves_the_box_and_its_text() {
    let (pdf, log) = render(
        "translate",
        &format!("{PAGE}<div style=\"transform:translate(10px,20px);width:100px;background:#c00\">text</div>"),
    );
    assert!(!log.contains("\"transform"), "{}", log);
    // 10 x 20 pixels are 7.5 pt to the right and 15 pt down (the PDF y axis points up)
    let at = pdf.find("1 0 0 1 7.5 -15 cm").unwrap_or_else(|| panic!("{}", pdf));
    // the background and the text are inside of it, and it ends before the next content
    let rest = &pdf[at..];
    let end = rest.find("ET Q").expect("the end of the transformation");
    let inside = &rest[..end];
    assert!(inside.contains(" re"), "{}", inside);
    assert!(inside.contains("Tj") || inside.contains("TJ"), "{}", inside);
}

#[test]
fn the_origin_is_the_center_and_can_be_changed() {
    let html = |extra: &str| {
        format!(
            "{PAGE}<div style=\"transform:scale(2);{extra}width:100px;height:50px;background:#00c\"></div>"
        )
    };
    let (center, _) = render("origin_center", &html(""));
    let (corner, _) = render("origin_corner", &html("transform-origin:left top;"));
    // the box is at (20pt, 20pt) from the top left corner of the page, 75 x 37.5 pt: a scale by 2
    // around its center (57.5, 261.25 from the bottom) or around its top left corner (20, 280)
    assert!(center.contains("2 0 0 2 -57.5 -261.25 cm"), "{}", center);
    assert!(corner.contains("2 0 0 2 -20 -280 cm"), "{}", corner);
}

#[test]
fn rotations_and_nesting() {
    let (pdf, _) = render(
        "nested",
        &format!(
            "{PAGE}<div style=\"transform:rotate(90deg);width:100px;height:50px\">\
               <div style=\"transform:translateX(10px);width:20px;height:20px;background:#0c0\"></div></div>"
        ),
    );
    // a quarter turn: cosine 0 and sine 1 (the sign of the second one depends on the y axis)
    assert!(pdf.contains("0 -1 1 0 ") || pdf.contains("0 1 -1 0 "), "{}", pdf);
    // two transformations, one inside of the other
    assert_eq!(pdf.matches(" cm").count() >= 2, true);
}

#[test]
fn an_inline_box_is_not_transformed() {
    let (pdf, _) = render(
        "inline",
        &format!("{PAGE}<p>a <span style=\"transform:rotate(30deg)\">b</span> c</p>"),
    );
    assert!(!pdf.contains("0.87"), "{}", pdf);
}

#[test]
fn none_and_unknown_functions_do_nothing() {
    let (pdf, log) = render(
        "none",
        &format!("{PAGE}<div style=\"transform:none\">a</div><div style=\"transform:wobble(3)\">b</div>"),
    );
    assert!(!pdf.contains("1 0 0 1 7.5"), "{}", pdf);
    assert!(log.contains("transform"), "{}", log);
}
