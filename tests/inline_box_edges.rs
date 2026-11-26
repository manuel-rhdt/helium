//! The margin, the border and the padding at the ends of an inline box take room on the line.

mod common;

fn check_layout(name: &str, html: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("inline_box_edges__{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg(format!("--font-dir={}", common::wpt_fonts().display()))
        .arg("--check-layout")
        .arg(&input)
        .arg(dir.join(format!("inline_box_edges__{name}.pdf")))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn horizontal_edges_widen_the_line() {
    // Ahem: every character is 10 pixels wide
    check_layout(
        "widths",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font: 10px Ahem }
  div { display: inline-block }
  span { padding: 0 3px; margin: 0 4px 0 5px; border: solid 2px }
</style>
<div data-expected-width="60">abcdef</div><br>
<div data-expected-width="79">ab<span>cd</span>ef</div><br>
<div data-expected-width="80">ab<span style="padding: 0; border: 0; margin: 0 10px">cd</span>ef</div>"#,
    );
}

#[test]
fn the_edges_of_nested_boxes_add_up() {
    check_layout(
        "nested",
        r#"<!DOCTYPE html>
<style>body { margin: 0; font: 10px Ahem } div { display: inline-block }</style>
<div data-expected-width="50">a<span style="padding: 0 5px"><span style="padding: 0 5px">b</span></span>c</div>"#,
    );
}

#[test]
fn an_inline_box_that_wraps_has_edges_at_its_ends_only() {
    // "ab cd" wraps after "ab ": the padding is in front of "ab" and behind "cd"
    check_layout(
        "wrap",
        r#"<!DOCTYPE html>
<style>body { margin: 0; font: 10px Ahem } span { padding: 0 10px }</style>
<div style="width: 50px" data-expected-height="24"><span>ab cd</span></div>"#,
    );
}
