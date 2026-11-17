//! Tests for `::before`, `::after`, `content` and the counters, read from the text of the PDF.

mod common;

/// The text that the layout draws, in order, with the glyphs mapped back to characters by the
/// PDF (needs `pdftotext`; the tests are skipped without it).
fn text(name: &str, html: &str) -> Option<String> {
    if std::process::Command::new("pdftotext").arg("-v").output().is_err() {
        return None;
    }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("generated_{name}.html"));
    let output = dir.join(format!("generated_{name}.pdf"));
    std::fs::write(&input, html).unwrap();
    let result = common::helium().unwrap().arg(&input).arg(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let extracted = std::process::Command::new("pdftotext").arg(&output).arg("-").output().unwrap();
    let text = String::from_utf8_lossy(&extracted.stdout).into_owned();
    // one line per paragraph, whatever the font does to the spaces
    Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

macro_rules! check {
    ($name:expr, $html:expr, $expected:expr) => {
        if let Some(text) = text($name, $html) {
            assert_eq!(text, $expected);
        }
    };
}

#[test]
fn before_and_after_wrap_the_content() {
    check!(
        "wrap",
        r#"<style>p::before { content: "[" } p::after { content: "]" }</style><p>a</p>"#,
        "[a]"
    );
}

#[test]
fn single_colon_pseudo_elements_work() {
    check!("single", r#"<style>p:before { content: "<" } p:after { content: ">" }</style><p>a</p>"#, "<a>");
}

#[test]
fn content_normal_and_none_generate_nothing() {
    check!(
        "none",
        r#"<style>p::before { content: none } p::after { content: normal } p::before { color: red }</style><p>a</p>"#,
        "a"
    );
}

#[test]
fn counters_number_headings() {
    check!(
        "headings",
        r#"<style>
        body { counter-reset: chapter }
        h1 { counter-increment: chapter; counter-reset: section }
        h2 { counter-increment: section }
        h1::before { content: counter(chapter) ". " }
        h2::before { content: counter(chapter) "." counter(section, lower-alpha) " " }
        </style><h1>A</h1><h2>x</h2><h2>y</h2><h1>B</h1><h2>z</h2>"#,
        "1. A 1.a x 1.b y 2. B 2.a z"
    );
}

#[test]
fn nested_counters_are_joined() {
    check!(
        "nested",
        r#"<style>ol { counter-reset: item } li { counter-increment: item }
        li { list-style: none } li::before { content: counters(item, ".") ": " }</style>
        <ol><li>a<ol><li>b</li><li>c</li></ol></li><li>d</li></ol>"#,
        "1: a 1.1: b 1.2: c 2: d"
    );
}

#[test]
fn counter_set_and_negative_increment() {
    check!(
        "set",
        r#"<style>div::before { content: counter(n) } div { counter-increment: n -2 }
        .s { counter-set: n 10 }</style><div>a</div><div class="s">b</div><div>c</div>"#,
        "-2a 10b 8c"
    );
}

#[test]
fn list_item_counter_follows_start_and_value() {
    check!(
        "listitem",
        r#"<style>li { list-style: none } li::before { content: counter(list-item) ":" }</style>
        <ol start="5"><li>a</li><li>b</li><li value="9">c</li><li>d</li></ol>"#,
        "5:a 6:b 9:c 10:d"
    );
}

#[test]
fn attr_and_quotes() {
    check!(
        "attr",
        r#"<style>a::after { content: " (" attr(data-n) ")" }</style><a data-n="7">x</a>"#,
        "x (7)"
    );
    check!("quotes", r#"<p><q>a <q>b</q></q></p>"#, "“a ‘b’”");
    check!(
        "quotes_custom",
        r#"<style>q { quotes: "«" "»" }</style><p><q>a</q></p>"#,
        "«a»"
    );
}

#[test]
fn elements_without_a_box_do_not_count() {
    check!(
        "hidden",
        r#"<style>p { counter-increment: n } p::before { content: counter(n) } .h { display: none }</style>
        <p>a</p><p class="h">b</p><p>c</p>"#,
        "1a 2c"
    );
}

#[test]
fn generated_content_does_not_take_part_in_selectors() {
    check!(
        "selectors",
        r#"<style>p::before { content: "x" } p:empty::after { content: "E" } span:first-child { color: red }
        </style><p></p><p><span>s</span></p>"#,
        "xE xs"
    );
}

#[test]
fn display_none_and_replaced_elements_have_no_pseudo_elements() {
    check!(
        "replaced",
        r#"<style>img::before, br::after, .h::before { content: "!" }
        .h { display: none }</style><p>a<img width="1" height="1"><br>b</p><div class="h">c</div>"#,
        "a b"
    );
}

#[test]
fn reversed_lists_count_down() {
    check!(
        "reversed",
        r#"<style>li { list-style: none } li::before { content: counter(list-item) ": " }</style>
        <ol reversed><li>a<li>b<li>c</ol>
        <ol reversed start="9"><li>d<li>e</ol>
        <ol reversed><li>f<li value="20">g<li>h</ol>"#,
        "3: a 2: b 1: c 9: d 8: e 21: f 20: g 19: h"
    );
}

#[test]
fn reversed_counters_start_from_their_scope() {
    check!(
        "reversed_counter",
        r#"<style>div { counter-reset: reversed(n) } p { counter-increment: n -1 } p::before { content: counter(n) ". " }
        p.skip { display: none }</style>
        <div><p>a</p><p class="skip">b</p><p>c</p><p>d</p></div>"#,
        "3. a 2. c 1. d"
    );
}

#[test]
fn counters_of_a_sibling_stay_in_scope() {
    check!(
        "siblings",
        r#"<style>span::before { content: counters(c, ".") ";" } .r { counter-reset: c } span { counter-increment: c }</style>
        <p><span class="r"></span><span></span></p>"#,
        "1;2;"
    );
}
