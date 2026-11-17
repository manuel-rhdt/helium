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

const PAGE: &str = "@page { size: 200px 120px; margin: 20px } div { height: 60px } section { display: block }";

#[test]
fn the_page_counter_restarts_with_the_page_context() {
    // `@page body:first { counter-reset: page 0 }`: the first page of the body is page 1
    let pdf = render(
        "restart",
        &format!(
            "<style>{PAGE} @page {{ @bottom-center {{ content: counter(page) }} }}\
             @page front {{ -helium-page-label: counter(page, lower-roman) }}\
             @page body:first {{ counter-reset: page 0 }}\
             @page appendix:first {{ counter-reset: page 0 }}\
             @page appendix {{ -helium-page-label: \"A-\" counter(page) }}\
             .front {{ page: front }} .body {{ page: body }} .appendix {{ page: appendix }}</style>\
             <section class=front><div>1</div><div>2</div></section>\
             <section class=body><div>3</div><div>4</div></section>\
             <section class=appendix><div>5</div><div>6</div></section>"
        ),
    );
    assert_eq!(
        page_labels(&pdf),
        "/Nums [ 0 << /S /r /St 1 >> 2 << /S /D /St 1 >> 4 << /S /D /P (A-) /St 1 >> ]"
    );
}

#[test]
fn the_increment_of_the_page_counter_splits_the_ranges() {
    // CSS Paged Media 3, 6.1: all pages are even-numbered; the labels count by ones, so
    // every page is a range of its own
    let pdf = render("even", &format!("<style>{PAGE} @page {{ counter-increment: page 2 }}</style><div>1</div><div>2</div><div>3</div>"));
    assert_eq!(page_labels(&pdf), "/Nums [ 0 << /S /D /St 2 >> 1 << /S /D /St 4 >> 2 << /S /D /St 6 >> ]");
}

#[test]
fn numbers_below_one_are_written_out() {
    // a label has a positive number: the pages before it have the text of the number as a prefix
    let pdf = render(
        "negative",
        &format!("<style>{PAGE} @page :first {{ counter-reset: page -3 }}</style><div>1</div><div>2</div><div>3</div><div>4</div>"),
    );
    assert_eq!(
        page_labels(&pdf),
        "/Nums [ 0 << /P (-2) >> 1 << /P (-1) >> 2 << /P (0) >> 3 << /S /D /St 1 >> ]"
    );
}

#[test]
fn the_label_property_is_independent_of_the_margin_boxes() {
    // without a number in the margin box, the label is the one that the page says
    let pdf = render(
        "property",
        &format!("<style>{PAGE} @page {{ -helium-page-label: \"p. \" counter(page, upper-alpha) }}</style><div>1</div><div>2</div>"),
    );
    assert_eq!(page_labels(&pdf), "/Nums [ 0 << /S /A /P (p. ) /St 1 >> ]");
    // `none` is the number in decimal, whatever the footer prints
    let pdf = render(
        "none_label",
        &format!("<style>{PAGE} @page {{ -helium-page-label: none; @bottom-center {{ content: counter(page, lower-roman) }} }}</style><div>1</div><div>2</div>"),
    );
    assert_eq!(pdf.matches("/PageLabels").count(), 0);
}
