mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("labels_{name}.html"));
    let output = dir.join(format!("labels_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    common::helium().unwrap().arg("--no-compression").arg(&input).arg(&output).assert().success();
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

const STYLE: &str = "<style>@page { size: 200px 120px; margin: 20px; @bottom-center { content: counter(page) } }\
    @page front { @bottom-center { content: counter(page, lower-roman) } }\
    @page appendix { @bottom-center { content: \"A-\" counter(page, upper-alpha) } }\
    div { height: 60px } .front { page: front } .appendix { page: appendix }</style>";

#[test]
fn page_labels_follow_the_numbers_that_the_pages_print() {
    let pdf = render(
        "ranges",
        &format!("{STYLE}<div class=front>1</div><div class=front>2</div><div>3</div><div>4</div><div class=appendix>5</div>"),
    );
    assert_eq!(pdf.matches("/PageLabels").count(), 1);
    // front matter in roman numerals from the first page, the body in decimal from the third
    // page (its number), the appendix with a prefix from the fifth
    assert_eq!(
        page_labels(&pdf),
        "/Nums [ 0 << /S /r /St 1 >> 2 << /S /D /St 3 >> 4 << /S /A /P (A-) /St 5 >> ]"
    );
}

/// The `/Nums` array of the page labels, with the white space normalized.
fn page_labels(pdf: &str) -> String {
    let start = pdf.find("/PageLabels").unwrap();
    let end = start + pdf[start..].find("]").unwrap() + 1;
    let text = &pdf[start..end];
    text[text.find("/Nums").unwrap()..].split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn plain_page_numbers_need_no_labels() {
    let pdf = render("plain", &format!("{STYLE}<div>1</div><div>2</div><div>3</div>"));
    assert_eq!(pdf.matches("/PageLabels").count(), 0);
    let pdf = render("none", "<div>1</div>");
    assert_eq!(pdf.matches("/PageLabels").count(), 0);
}
