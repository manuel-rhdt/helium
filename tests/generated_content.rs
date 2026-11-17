//! Tests for `::before`, `::after`, `::marker`, `content` and the counters.
//!
//! They read the layout dump (`--dump-layout`): the text runs of the document in order, with the
//! text of the generated content in their place, and the marker of every list item before its
//! content. The strings are exact (spaces included) and no other tool is needed.

mod common;

use serde_json::Value;

/// The markers and text runs of the document, in document order.
fn runs(name: &str, html: &str) -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("generated_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let result = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("generated_{name}.pdf")))
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let dump: Value = serde_json::from_slice(&result.stdout).unwrap();

    fn walk(node: &Value, out: &mut Vec<String>) {
        for key in ["marker", "text"] {
            if let Some(text) = node[key].as_str().filter(|text| !text.is_empty()) {
                out.push(text.to_owned());
            }
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = vec![];
    walk(&dump["document"], &mut out);
    out
}

macro_rules! check {
    ($name:expr, $html:expr, [$($expected:expr),* $(,)?]) => {
        assert_eq!(runs($name, $html), [$($expected),*] as [&str; _]);
    };
}

#[test]
fn before_and_after_wrap_the_content() {
    check!(
        "wrap",
        r#"<style>p::before { content: "[" } p::after { content: "]" }</style><p>a</p>"#,
        ["[", "a", "]"]
    );
}

#[test]
fn single_colon_pseudo_elements_work() {
    check!("single", r#"<style>p:before { content: "<" } p:after { content: ">" }</style><p>a</p>"#, ["<", "a", ">"]);
}

#[test]
fn content_normal_and_none_generate_nothing() {
    check!(
        "none",
        r#"<style>p::before { content: none } p::after { content: normal } p::before { color: red }</style><p>a</p>"#,
        ["a"]
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
        ["1. ", "A", "1.a ", "x", "1.b ", "y", "2. ", "B", "2.a ", "z"]
    );
}

#[test]
fn nested_counters_are_joined() {
    check!(
        "nested",
        r#"<style>ol { counter-reset: item } li { counter-increment: item }
        li { list-style: none } li::before { content: counters(item, ".") ": " }</style>
        <ol><li>a<ol><li>b</li><li>c</li></ol></li><li>d</li></ol>"#,
        ["1: ", "a", "1.1: ", "b", "1.2: ", "c", "2: ", "d"]
    );
}

#[test]
fn counter_set_and_negative_increment() {
    check!(
        "set",
        r#"<style>div::before { content: counter(n) } div { counter-increment: n -2 }
        .s { counter-set: n 10 }</style><div>a</div><div class="s">b</div><div>c</div>"#,
        ["-2", "a", "10", "b", "8", "c"]
    );
}

#[test]
fn list_item_counter_follows_start_and_value() {
    check!(
        "listitem",
        r#"<style>li { list-style: none } li::before { content: counter(list-item) ":" }</style>
        <ol start="5"><li>a</li><li>b</li><li value="9">c</li><li>d</li></ol>"#,
        ["5:", "a", "6:", "b", "9:", "c", "10:", "d"]
    );
}

#[test]
fn list_markers_are_numbered_by_the_counter() {
    check!(
        "markers",
        r#"<ol start="3"><li>a</li><li>b</li></ol><ul><li>c</li></ul>"#,
        ["3.", "a", "4.", "b", "•", "c"]
    );
}

#[test]
fn attr_and_quotes() {
    check!(
        "attr",
        r#"<style>a::after { content: " (" attr(data-n) ")" }</style><a data-n="7">x</a>"#,
        ["x", " (7)"]
    );
    check!("quotes", r#"<p><q>a <q>b</q></q></p>"#, ["“", "a ", "‘", "b", "’", "”"]);
    check!("quotes_custom", r#"<style>q { quotes: "«" "»" }</style><p><q>a</q></p>"#, ["«", "a", "»"]);
}

#[test]
fn attr_falls_back_to_a_string() {
    check!(
        "attr_fallback",
        r#"<style>p::before { content: attr(data-t, "none") ": " }</style>
        <p data-t="given">a</p><p>b</p>"#,
        ["given: ", "a", "none: ", "b"]
    );
}

#[test]
fn elements_without_a_box_do_not_count() {
    check!(
        "hidden",
        r#"<style>p { counter-increment: n } p::before { content: counter(n) } .h { display: none }</style>
        <p>a</p><p class="h">b</p><p>c</p>"#,
        ["1", "a", "2", "c"]
    );
}

#[test]
fn generated_content_does_not_take_part_in_selectors() {
    check!(
        "selectors",
        r#"<style>p::before { content: "x" } p:empty::after { content: "E" } span:first-child { color: red }
        </style><p></p><p><span>s</span></p>"#,
        ["x", "E", "x", "s"]
    );
}

#[test]
fn display_none_and_replaced_elements_have_no_pseudo_elements() {
    check!(
        "replaced",
        r#"<style>img::before, br::after, .h::before { content: "!" }
        .h { display: none }</style><p>a<img width="1" height="1"><br>b</p><div class="h">c</div>"#,
        ["a", "b"]
    );
}

#[test]
fn marker_content_replaces_the_marker() {
    check!(
        "marker_content",
        r#"<style>.s::marker { content: "→ " } .n::marker { content: none }
        .c::marker { content: counter(list-item, upper-roman) ") " }</style>
        <ul><li class="s">a</li><li class="n">b</li></ul><ol><li class="c">c</li><li class="c">d</li></ol>"#,
        ["→ ", "a", "b", "I) ", "c", "II) ", "d"]
    );
}

#[test]
fn marker_counters_belong_to_the_item() {
    check!(
        "marker_counters",
        r#"<style>
        ol { counter-reset: n }
        li::marker { counter-increment: n; content: counters(n, ":") "> " }
        .r::marker { counter-reset: n }
        </style><ol><li>a</li><li class="r">b</li><li>c</li></ol>"#,
        ["1> ", "a", "1:1> ", "b", "2> ", "c"]
    );
}

#[test]
fn marker_rules_do_not_reach_generated_list_items() {
    // the marker of a `::before` that is a list item is the default one
    check!(
        "marker_pseudo",
        r#"<style>::marker { content: "m;" } li::before { display: list-item; content: "b" }</style>
        <ul><li>a</li></ul>"#,
        ["m;", "•", "b", "a"]
    );
}
