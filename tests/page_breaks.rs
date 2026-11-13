//! Page breaks: `break-before`, `break-after`, `break-inside`, their CSS 2 names, `orphans` and
//! `widows`, expressed against the layout dump.
//!
//! The pages are 200px high and the text has a line height of 20px, so ten lines fit on a page.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, body: &str) -> Value {
    let html = format!(
        r#"<!DOCTYPE html><html><head><style>
        @page {{ width: 300px; height: 200px; margin: 0 }}
        body, p, h2, div {{ margin: 0; padding: 0 }}
        body {{ font: 16px/20px sans-serif }}
        {}
        </style></head><body>{}</body></html>"#,
        "", body
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("page_breaks__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("page_breaks__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A paragraph of `lines` lines with a background of the given colour.
fn lines(color: &str, lines: usize, style: &str) -> String {
    let text = vec!["line"; lines].join("<br>");
    format!(r#"<p style="background-color: {}; {}">{}</p>"#, color, style, text)
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const RED_CSS: &str = "#ff0000";
const GREEN_CSS: &str = "#00ff00";
const BLUE_CSS: &str = "#0000ff";

/// The pages and heights of the pieces of the background of one box: one piece per page.
fn pieces(dump: &Value, color: [u8; 3]) -> Vec<(u64, f64)> {
    let color = json!([color[0], color[1], color[2], 255]);
    let mut pieces: Vec<(u64, f64)> = dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| rect["color"] == color)
        .map(|rect| (rect["page"].as_u64().unwrap(), rect["height"].as_f64().unwrap()))
        .collect();
    pieces.sort_by(|a, b| a.partial_cmp(b).unwrap());
    pieces
}

/// The lines on each page: `[(page, number of lines)]`.
fn lines_per_page(dump: &Value, color: [u8; 3]) -> Vec<(u64, usize)> {
    pieces(dump, color)
        .into_iter()
        .map(|(page, height)| (page, (height / 20.0).round() as usize))
        .collect()
}

fn pages(dump: &Value, color: [u8; 3]) -> Vec<u64> {
    pieces(dump, color).into_iter().map(|(page, _)| page).collect()
}

#[test]
fn paragraphs_break_between_lines_by_default() {
    let dump = dump(
        "default",
        &format!("{}{}", lines(RED_CSS, 4, ""), lines(GREEN_CSS, 8, "")),
    );
    // 4 + 6 lines fit on the first page; the other two lines of the second paragraph follow
    assert_eq!(lines_per_page(&dump, GREEN), [(0, 6), (1, 2)]);
}

#[test]
fn orphans_keep_the_first_lines_of_a_paragraph_together() {
    // one line of the second paragraph would fit on the first page
    let body = |style: &str| format!("{}{}", lines(RED_CSS, 9, ""), lines(GREEN_CSS, 5, style));
    let default = dump("orphans_default", &body(""));
    assert_eq!(lines_per_page(&default, GREEN), [(1, 5)]);
    let relaxed = dump("orphans_one", &body("orphans: 1"));
    assert_eq!(lines_per_page(&relaxed, GREEN), [(0, 1), (1, 4)]);
}

#[test]
fn widows_keep_the_last_lines_of_a_paragraph_together() {
    // two lines fit on the first page and one would be left for the second
    let body = |style: &str| format!("{}{}", lines(RED_CSS, 8, ""), lines(GREEN_CSS, 3, style));
    let default = dump("widows_default", &body(""));
    // neither 2/1 (widows) nor 1/2 (orphans) is allowed, so the paragraph moves
    assert_eq!(lines_per_page(&default, GREEN), [(1, 3)]);
    let relaxed = dump("widows_one", &body("widows: 1"));
    assert_eq!(lines_per_page(&relaxed, GREEN), [(0, 2), (1, 1)]);
}

#[test]
fn widows_move_lines_to_the_next_page_instead_of_moving_the_paragraph() {
    // ten of the 12 lines fit on a page, leaving 2 for the second one; with widows: 5 the second
    // page needs three more lines
    let dump = dump("widows_pull", &lines(RED_CSS, 12, "widows: 5"));
    // the part on the first page reaches the end of the page; the last part is as tall as its
    // lines
    assert_eq!(lines_per_page(&dump, RED).last(), Some(&(1, 5)));
}

#[test]
fn orphans_and_widows_are_inherited() {
    let dump = dump(
        "inherited",
        &format!(
            r#"<div style="orphans: 1">{}{}</div>"#,
            lines(RED_CSS, 9, ""),
            lines(GREEN_CSS, 5, "")
        ),
    );
    assert_eq!(lines_per_page(&dump, GREEN), [(0, 1), (1, 4)]);
}

#[test]
fn a_paragraph_too_short_for_orphans_and_widows_is_not_split() {
    // 2 + 2 > 3 lines: there is no good break inside the paragraph
    let dump = dump(
        "short",
        &format!("{}{}", lines(RED_CSS, 9, ""), lines(GREEN_CSS, 3, "")),
    );
    assert_eq!(pages(&dump, GREEN), [1]);
}

#[test]
fn break_inside_avoid_moves_the_box_to_the_next_page() {
    let body = |style: &str| format!("{}{}", lines(RED_CSS, 8, ""), lines(GREEN_CSS, 5, style));
    let default = dump("inside_auto", &body(""));
    assert_eq!(lines_per_page(&default, GREEN), [(0, 2), (1, 3)]);
    let avoided = dump("inside_avoid", &body("break-inside: avoid"));
    assert_eq!(lines_per_page(&avoided, GREEN), [(1, 5)]);
    let legacy = dump("inside_legacy", &body("page-break-inside: avoid"));
    assert_eq!(lines_per_page(&legacy, GREEN), [(1, 5)]);
    let page = dump("inside_avoid_page", &body("break-inside: avoid-page"));
    assert_eq!(lines_per_page(&page, GREEN), [(1, 5)]);
}

#[test]
fn break_inside_avoid_applies_to_all_descendants() {
    let dump = dump(
        "inside_nested",
        &format!(
            r#"{}<div style="break-inside: avoid">{}{}</div>"#,
            lines(RED_CSS, 6, ""),
            lines(GREEN_CSS, 3, ""),
            lines(BLUE_CSS, 3, "")
        ),
    );
    // the whole div (6 lines) does not fit behind the first paragraph
    assert_eq!(pages(&dump, GREEN), [1]);
    assert_eq!(pages(&dump, BLUE), [1]);
}

#[test]
fn boxes_that_are_taller_than_a_page_are_broken_anyway() {
    let dump = dump("inside_too_tall", &lines(RED_CSS, 15, "break-inside: avoid"));
    let total: usize = lines_per_page(&dump, RED).iter().map(|&(_, n)| n).sum();
    assert_eq!(total, 15);
    assert_eq!(pages(&dump, RED), [0, 1]);
}

#[test]
fn forced_breaks_before_and_after() {
    let before = dump(
        "before_page",
        &format!("{}{}", lines(RED_CSS, 2, ""), lines(GREEN_CSS, 2, "break-before: page")),
    );
    assert_eq!((pages(&before, RED), pages(&before, GREEN)), (vec![0], vec![1]));
    let after = dump(
        "after_page",
        &format!("{}{}", lines(RED_CSS, 2, "break-after: page"), lines(GREEN_CSS, 2, "")),
    );
    assert_eq!((pages(&after, RED), pages(&after, GREEN)), (vec![0], vec![1]));
    for value in ["always", "left", "right", "recto", "verso"] {
        let dump = dump(
            &format!("before_{}", value),
            &format!(
                "{}{}",
                lines(RED_CSS, 2, ""),
                lines(GREEN_CSS, 2, &format!("break-before: {}", value))
            ),
        );
        assert_eq!(pages(&dump, GREEN), [1], "break-before: {}", value);
    }
}

#[test]
fn the_css_2_names_force_breaks() {
    let dump = dump(
        "legacy",
        &format!(
            "{}{}{}",
            lines(RED_CSS, 2, "page-break-after: always"),
            lines(GREEN_CSS, 2, "page-break-before: always"),
            lines(BLUE_CSS, 2, "")
        ),
    );
    // the two breaks next to each other are a single break
    assert_eq!(pages(&dump, RED), [0]);
    assert_eq!(pages(&dump, GREEN), [1]);
    assert_eq!(pages(&dump, BLUE), [1]);
}

#[test]
fn the_css_2_names_and_the_new_names_share_the_cascade() {
    let later_new = dump(
        "cascade_new",
        &format!(
            "{}{}",
            lines(RED_CSS, 2, ""),
            lines(GREEN_CSS, 2, "page-break-before: always; break-before: auto")
        ),
    );
    assert_eq!(pages(&later_new, GREEN), [0]);
    let later_old = dump(
        "cascade_old",
        &format!(
            "{}{}",
            lines(RED_CSS, 2, ""),
            lines(GREEN_CSS, 2, "break-before: auto; page-break-before: always")
        ),
    );
    assert_eq!(pages(&later_old, GREEN), [1]);
}

#[test]
fn break_after_avoid_keeps_a_heading_with_the_next_box() {
    let body = |style: &str| {
        format!(
            r#"{}<h2 style="{}">heading</h2>{}"#,
            lines(RED_CSS, 9, ""),
            style,
            lines(GREEN_CSS, 3, "")
        )
    };
    // the heading is the tenth line of the first page, the text starts on the second
    let default = dump("heading_default", &body("background-color: #0000ff"));
    assert_eq!(pages(&default, BLUE), [0]);
    let avoided = dump(
        "heading_avoid",
        &body("background-color: #0000ff; break-after: avoid"),
    );
    assert_eq!(pages(&avoided, BLUE), [1]);
    assert_eq!(pages(&avoided, GREEN), [1]);
    let legacy = dump(
        "heading_legacy",
        &body("background-color: #0000ff; page-break-after: avoid"),
    );
    assert_eq!(pages(&legacy, BLUE), [1]);
}

#[test]
fn break_before_avoid_keeps_a_box_with_the_previous_one() {
    let body = |style: &str| {
        format!(
            "{}{}{}",
            lines(RED_CSS, 4, ""),
            lines(GREEN_CSS, 6, "break-inside: avoid"),
            lines(BLUE_CSS, 2, style)
        )
    };
    // the third paragraph starts on the second page; the second can stay on the first
    let default = dump("before_avoid_default", &body(""));
    assert_eq!((pages(&default, GREEN), pages(&default, BLUE)), (vec![0], vec![1]));
    let avoided = dump("before_avoid", &body("break-before: avoid"));
    assert_eq!((pages(&avoided, GREEN), pages(&avoided, BLUE)), (vec![1], vec![1]));
}

#[test]
fn break_values_of_a_first_or_last_child_apply_to_the_parent() {
    let first_child = dump(
        "propagate_before",
        &format!(
            r#"{}<div>{}</div>"#,
            lines(RED_CSS, 2, ""),
            lines(GREEN_CSS, 2, "break-before: page")
        ),
    );
    assert_eq!(pages(&first_child, GREEN), [1]);
    let last_child = dump(
        "propagate_after",
        &format!(
            r#"<div>{}</div>{}"#,
            lines(RED_CSS, 2, "break-after: page"),
            lines(GREEN_CSS, 2, "")
        ),
    );
    assert_eq!(pages(&last_child, GREEN), [1]);
}

#[test]
fn a_forced_break_wins_over_avoid() {
    let dump = dump(
        "forced_wins",
        &format!(
            "{}{}",
            lines(RED_CSS, 2, "break-after: avoid"),
            lines(GREEN_CSS, 2, "break-before: page")
        ),
    );
    assert_eq!(pages(&dump, GREEN), [1]);
}

#[test]
fn forced_breaks_still_happen_inside_of_boxes_that_avoid_breaks() {
    let dump = dump(
        "forced_inside_avoid",
        &format!(
            r#"<div style="break-inside: avoid">{}{}</div>"#,
            lines(RED_CSS, 2, ""),
            lines(GREEN_CSS, 2, "break-before: page")
        ),
    );
    assert_eq!((pages(&dump, RED), pages(&dump, GREEN)), (vec![0], vec![1]));
}

#[test]
fn invalid_break_values_are_ignored() {
    let dump = dump(
        "invalid",
        &format!(
            "{}{}",
            lines(RED_CSS, 2, ""),
            lines(GREEN_CSS, 2, "break-before: sometimes; page-break-before: recto")
        ),
    );
    // `recto` is not valid for the CSS 2 name; the declaration is dropped
    assert_eq!(pages(&dump, GREEN), [0]);
}
