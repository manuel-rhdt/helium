//! `target-counter()`, `target-counters()` and `target-text()`: the content of a table of
//! contents that reads what is further down in the document, tested through the layout dump.

mod common;

use serde_json::Value;

/// The text of every paragraph that the layout drew, in the order of the dump.
fn texts(name: &str, html: &str) -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("xref_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("xref_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|paragraph| {
            paragraph["pieces"].as_array().into_iter().flatten().filter_map(|piece| piece["text"].as_str()).collect()
        })
        .collect()
}

/// Whether a paragraph ends with `wanted` (the link text that comes before the content is
/// not of interest).
fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|text| text.ends_with(wanted))
}

#[test]
fn a_table_of_contents_reads_counters_text_and_pages_of_what_follows() {
    let texts = texts(
        "toc",
        r##"<style>
        body { counter-reset: h1 }
        h1 { counter-increment: h1; break-before: page }
        a::after { content: target-counter(attr(href), h1) " " target-text(attr(href)) " " target-counter(attr(href), page) }
        a { display: block }
        </style>
        <nav><a href="#a">.</a><a href="#b">.</a></nav>
        <h1 id="a">Alpha <em>one</em></h1><h1 id="b">Beta</h1>"##,
    );
    assert!(has(&texts, "1 Alpha one 2"), "{texts:?}");
    assert!(has(&texts, "2 Beta 3"), "{texts:?}");
}

#[test]
fn the_page_counter_follows_the_counter_of_the_page_rules() {
    let texts = texts(
        "page_counter",
        r##"<style>
        @page :first { counter-reset: page 10 }
        h1 { break-before: page }
        a::after { content: target-counter(attr(href), page, lower-roman) }
        </style>
        <a href="#a">.</a><h1 id="a">Alpha</h1>"##,
    );
    // the first page is 11, so the chapter on the second page is 12
    assert!(has(&texts, "xii"), "{texts:?}");
}

#[test]
fn a_table_of_contents_that_grows_moves_what_it_points_at() {
    // 60 entries fill more than one page, so the chapter is on page 3, not page 2 as the first
    // layout (with no numbers yet, and so shorter entries) finds
    let links: String = (0..60).map(|_| r##"<a href="#a">.</a>"##).collect();
    let texts = texts(
        "settles",
        &format!(
            r##"<style>
            a {{ display: block; margin: 8px 0 }}
            a::after {{ content: "page " target-counter(attr(href), page) }}
            h1 {{ break-before: page }}
            </style>{links}<h1 id="a">Alpha</h1>"##
        ),
    );
    let numbers: Vec<_> = texts.iter().filter(|text| text.contains("page ")).collect();
    assert_eq!(numbers.len(), 60);
    assert!(numbers.iter().all(|text| **text == *numbers[0]), "{numbers:?}");
    assert!(!numbers[0].ends_with("page 2"), "the list is longer than one page");
}

#[test]
fn nested_counters_are_joined_and_a_missing_target_is_empty() {
    let texts = texts(
        "nested",
        r##"<style>
        ol { counter-reset: n } li { counter-increment: n; display: block } a { display: block }
        a::after { content: "[" target-counters(attr(href), n, ".") "]" }
        .gone::after { content: "{" target-counter(attr(href), n) "}" }
        </style>
        <a href="#x">.</a><a class="gone" href="#nope">.</a>
        <ol><li><ol><li>a</li><li id="x">b</li></ol></li></ol>"##,
    );
    assert!(has(&texts, "[1.2]"), "{texts:?}");
    assert!(has(&texts, "{}"), "{texts:?}");
}
