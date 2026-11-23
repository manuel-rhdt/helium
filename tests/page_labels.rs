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
fn auto_labels_follow_the_style_of_the_one_counter() {
    let pdf = render(
        "ranges",
        &format!("{STYLE}<div class=front>1</div><div class=front>2</div><div>3</div><div>4</div><div class=appendix>5</div>"),
    );
    assert_eq!(pdf.matches("/PageLabels").count(), 1);
    // front matter in roman numerals from the first page, the body in decimal from the third
    // page (its number), the appendix in letters from the fifth; the "A-" of the footer is text
    // around the counter, not a prefix
    assert_eq!(page_labels(&pdf), "/Nums [ 0 << /S /r /St 1 >> 2 << /S /D /St 3 >> 4 << /S /A /St 5 >> ]");
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
fn the_labels_follow_the_page_context() {
    // the labels change with the style and the prefix of the page; the counter goes on counting
    let pdf = render(
        "restart",
        &format!(
            "<style>{PAGE} @page {{ @bottom-center {{ content: counter(page) }} }}\
             @page front {{ -helium-page-label: counter(page, lower-roman) }}\
             @page appendix {{ -helium-page-label: \"A-\" counter(page) }}\
             .front {{ page: front }} .body {{ page: body }} .appendix {{ page: appendix }}</style>\
             <section class=front><div>1</div><div>2</div></section>\
             <section class=body><div>3</div><div>4</div></section>\
             <section class=appendix><div>5</div><div>6</div></section>"
        ),
    );
    assert_eq!(
        page_labels(&pdf),
        "/Nums [ 0 << /S /r /St 1 >> 2 << /S /D /St 3 >> 4 << /S /D /P (A-) /St 5 >> ]"
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

fn labels_of(name: &str, css: &str) -> String {
    let pdf = render(name, &format!("<style>{PAGE} {css}</style><div>1</div><div>2</div><div>3</div><div>4</div>"));
    if pdf.contains("/PageLabels") { page_labels(&pdf) } else { "none".to_string() }
}

#[test]
fn auto_needs_exactly_one_counter() {
    // the number is in the header and in the footer, in different styles: nothing to follow
    assert_eq!(
        labels_of("two", "@page { @top-right { content: counter(page, lower-roman) } @bottom-center { content: counter(page) } }"),
        "none"
    );
    // no number
    assert_eq!(labels_of("no_number", "@page { @bottom-center { content: \"Draft\" } }"), "none");
    // the one counter can be in any margin box, and the total of pages is another counter
    assert_eq!(
        labels_of("header", "@page { @top-left { content: counter(page, lower-roman) \" / \" counter(pages) } }"),
        "/Nums [ 0 << /S /r /St 1 >> ]"
    );
    // two counters in one margin box
    assert_eq!(
        labels_of("both", "@page { @bottom-center { content: counter(page, lower-roman) \" (\" counter(page) \")\" } }"),
        "none"
    );
}

#[test]
fn text_around_the_counter_is_not_a_prefix() {
    // a running title on the right pages must not become part of the labels
    assert_eq!(
        labels_of(
            "title",
            "@page :left { @bottom-left { content: counter(page) } } @page :right { @bottom-right { content: \"My Book \\2014  \" counter(page) } }"
        ),
        "none"
    );
    assert_eq!(
        labels_of("page_of", "@page { @bottom-center { content: \"Page \" counter(page, upper-roman) \" of \" counter(pages) } }"),
        "/Nums [ 0 << /S /R /St 1 >> ]"
    );
}

#[test]
fn even_and_odd_pages_can_differ() {
    // the same style on left and right pages: one range, whatever the position
    assert_eq!(
        labels_of(
            "same",
            "@page :left { @bottom-left { content: counter(page, lower-roman) } } @page :right { @bottom-right { content: counter(page, lower-roman) } }"
        ),
        "/Nums [ 0 << /S /r /St 1 >> ]"
    );
    // other styles: the labels follow the pages, a range for every page
    assert_eq!(
        labels_of(
            "differ",
            "@page :left { @bottom-left { content: counter(page, lower-roman) } } @page :right { @bottom-right { content: counter(page) } }"
        ),
        "/Nums [ 0 << /S /D /St 1 >> 1 << /S /r /St 2 >> 2 << /S /D /St 3 >> 3 << /S /r /St 4 >> ]"
    );
}
