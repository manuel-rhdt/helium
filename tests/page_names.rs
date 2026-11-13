//! Named pages (`page`, `@page name`) and the margins of left and right pages.

mod common;

use serde_json::Value;

/// 300pt pages, so a page is 400px wide; the margins are 40pt, which is 53.33px.
const PAGE: &str = "@page { width: 300pt; height: 300pt; margin: 50pt 40pt; \
    @top-center { content: \"HEAD\" } @bottom-center { content: counter(page) } }";

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir).unwrap();
    let input = dir.join(format!("page_names_{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("page_names_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The text of the page, as `(is_margin_box, x, number of glyphs)` for every run.
fn runs(dump: &Value, page: u64) -> Vec<(bool, f64, usize)> {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|run| run["page"] == page)
        .map(|run| {
            let ops = run["ops"].as_array().unwrap();
            let glyphs = ops
                .iter()
                .filter(|op| op["op"] == "kerned")
                .map(|op| {
                    op["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|item| item.is_array())
                        .map(|item| item.as_array().unwrap().len())
                        .sum::<usize>()
                })
                .sum();
            (ops[0]["op"] == "artifact", run["x"].as_f64().unwrap(), glyphs)
        })
        .collect()
}

fn page_count(dump: &Value) -> u64 {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["page"].as_u64().unwrap() + 1)
        .max()
        .unwrap()
}

/// Whether the page has the header (the four letters of "HEAD").
fn has_header(dump: &Value, page: u64) -> bool {
    runs(dump, page)
        .iter()
        .any(|&(margin_box, _, glyphs)| margin_box && glyphs == 4)
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.5
}

const STYLE: &str = "body { margin: 0 } p { margin: 0 }";

#[test]
fn a_change_of_the_page_name_breaks_the_page() {
    let dump = dump(
        "break",
        &format!(
            "<style>{PAGE} {STYLE} .c {{ page: chapter }}</style>\
             <p>a</p><p>b</p><div class=c><p>c</p></div><p>d</p>"
        ),
    );
    // a and b share a page, c is on a page of its own, and so is d
    assert_eq!(page_count(&dump), 3);
}

#[test]
fn the_same_page_name_does_not_break_the_page() {
    let dump = dump(
        "same",
        &format!(
            "<style>{PAGE} {STYLE} p {{ page: chapter }}</style><p>a</p><p>b</p><p>c</p>"
        ),
    );
    assert_eq!(page_count(&dump), 1);
}

#[test]
fn page_names_are_inherited_by_children() {
    let dump = dump(
        "inherit",
        &format!(
            "<style>{PAGE} {STYLE} .c {{ page: chapter }}</style>\
             <div class=c><p>a</p><div><p>b</p></div></div>"
        ),
    );
    assert_eq!(page_count(&dump), 1);
}

#[test]
fn auto_is_the_name_of_the_parent() {
    // `auto` does not reset the name: the paragraphs stay on one page
    let dump = dump(
        "auto",
        &format!(
            "<style>{PAGE} {STYLE} .c {{ page: chapter }} .d {{ page: auto }}</style>\
             <div class=c><p>a</p><div class=d><p>b</p></div></div>"
        ),
    );
    assert_eq!(page_count(&dump), 1);
}

#[test]
fn a_child_can_have_another_name() {
    let dump = dump(
        "other",
        &format!(
            "<style>{PAGE} {STYLE} .c {{ page: chapter }} .d {{ page: appendix }}</style>\
             <div class=c><p>a</p><div class=d><p>b</p></div></div>"
        ),
    );
    assert_eq!(page_count(&dump), 2);
}

#[test]
fn rules_for_a_page_name_apply_to_those_pages_only() {
    let dump = dump(
        "rules",
        &format!(
            "<style>{PAGE} {STYLE} @page chapter {{ @top-center {{ content: none }} }} \
             .c {{ page: chapter }}</style><p>a</p><div class=c><p>b</p></div><p>c</p>"
        ),
    );
    assert!(has_header(&dump, 0));
    assert!(!has_header(&dump, 1));
    assert!(has_header(&dump, 2));
}

#[test]
fn named_first_is_the_first_page_of_a_group() {
    // two chapters of two pages each: the pages that are the first of their chapter have no header
    let dump = dump(
        "group",
        &format!(
            "<style>{PAGE} {STYLE} @page chapter:first {{ @top-center {{ content: none }} }} \
             section {{ page: chapter; break-before: page }} p {{ break-after: page }}</style>\
             <section><p>a</p><p>b</p></section><section><p>c</p><p>d</p></section>"
        ),
    );
    assert_eq!(page_count(&dump), 4);
    assert!(!has_header(&dump, 0));
    assert!(has_header(&dump, 1));
    assert!(!has_header(&dump, 2));
    assert!(has_header(&dump, 3));
}

#[test]
fn page_rules_are_applied_by_specificity() {
    // the more specific rule wins although it comes first
    let dump = dump(
        "specificity",
        &format!(
            "<style>{PAGE} {STYLE} @page chapter {{ @top-center {{ content: none }} }} \
             @page {{ @top-center {{ content: \"HEAD\" }} }} \
             .c {{ page: chapter }}</style><div class=c><p>b</p></div>"
        ),
    );
    assert!(!has_header(&dump, 0));
}

#[test]
fn left_and_right_pages_have_their_own_margins() {
    // The inner margin is 60pt (80px), the outer one 20pt (26.67px): the same total as the
    // default of 40pt on both sides.
    let dump = dump(
        "mirrored",
        &format!(
            "<style>{PAGE} {STYLE} \
             @page :right {{ margin-left: 20pt; margin-right: 60pt }} \
             @page :left {{ margin-left: 60pt; margin-right: 20pt }} \
             p {{ break-after: page }}</style><p>a</p><p>b</p><p>c</p>"
        ),
    );
    let body_x = |page| {
        runs(&dump, page)
            .iter()
            .find(|run| !run.0)
            .map(|run| run.1)
            .unwrap()
    };
    // the first page is a right page
    assert!(close(body_x(0), 80.0 / 3.0), "{}", body_x(0));
    assert!(close(body_x(1), 80.0), "{}", body_x(1));
    assert!(close(body_x(2), 80.0 / 3.0), "{}", body_x(2));
}

#[test]
fn margin_boxes_follow_the_margins_of_the_page() {
    let dump = dump(
        "margin_boxes",
        &format!(
            "<style>{PAGE} {STYLE} \
             @page :right {{ margin-left: 20pt; margin-right: 60pt }} \
             @page :left {{ margin-left: 60pt; margin-right: 20pt }} \
             @page {{ @top-left {{ content: \"HEAD\" }} @top-center {{ content: none }} @bottom-center {{ content: none }} }} \
             p {{ break-after: page }}</style><p>a</p><p>b</p>"
        ),
    );
    let header_x = |page| {
        runs(&dump, page)
            .iter()
            .find(|run| run.0)
            .map(|run| run.1)
            .unwrap()
    };
    // left-aligned in the content area of the page
    assert!(close(header_x(0), 80.0 / 3.0), "{}", header_x(0));
    assert!(close(header_x(1), 80.0), "{}", header_x(1));
}

#[test]
fn margins_of_the_plain_rule_apply_to_all_pages() {
    let dump = dump(
        "plain",
        &format!("<style>{PAGE} {STYLE} p {{ break-after: page }}</style><p>a</p><p>b</p>"),
    );
    for page in 0..2 {
        let x = runs(&dump, page).iter().find(|run| !run.0).unwrap().1;
        assert!(close(x, 160.0 / 3.0), "{x}");
    }
}

#[test]
fn the_first_page_of_a_right_to_left_document_is_a_left_page() {
    let html = format!(
        "<html dir=rtl><style>{PAGE} {STYLE} \
         @page :right {{ margin-left: 20pt; margin-right: 60pt }} \
         @page :left {{ margin-left: 60pt; margin-right: 20pt }} \
         @page {{ @top-left {{ content: \"HEAD\" }} @top-center {{ content: none }} @bottom-center {{ content: none }} }} \
         p {{ break-after: page }}</style><body><p>a</p><p>b</p>"
    );
    let dump = dump("rtl_first_page", &html);
    let header_x = |page| {
        runs(&dump, page)
            .iter()
            .find(|run| run.0)
            .map(|run| run.1)
            .unwrap()
    };
    // the first page is on the left of the spread, with the binding on its right side
    assert!(close(header_x(0), 80.0), "{}", header_x(0));
    assert!(close(header_x(1), 80.0 / 3.0), "{}", header_x(1));
}
