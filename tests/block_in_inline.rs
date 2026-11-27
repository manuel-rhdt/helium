//! A block-level box in an inline box: the inline box is broken around it.

mod common;

fn check_layout(name: &str, html: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("block_in_inline__{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--check-layout")
        .arg(&input)
        .arg(dir.join(format!("block_in_inline__{name}.pdf")))
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
fn a_block_in_a_span_is_a_box_between_two_lines() {
    check_layout(
        "span",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font: 20px/20px sans-serif }
  .inner { height: 30px; margin-bottom: 10px }
</style>
<div data-expected-height="100">
  <span>text<div class="inner" data-offset-y="20" data-expected-height="30"></div>
  </span><div style="height: 40px" data-offset-y="60" data-expected-height="40"></div>
</div>"#,
    );
}

#[test]
fn white_space_after_the_block_and_empty_spans_make_no_line() {
    check_layout(
        "empty",
        r#"<!DOCTYPE html>
<style>body { margin: 0; font: 20px/20px sans-serif } .inner { height: 30px }</style>
<div data-expected-height="30"> <span><div class="inner"></div></span> <span></span> </div>"#,
    );
}

#[test]
fn anonymous_blocks_keep_the_parent_line_height() {
    check_layout(
        "anonymous_strut",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font: 20px/40px sans-serif }
  span { font-size: 10px; line-height: 10px }
</style>
<div data-expected-height="90">
  <span>before</span><div style="height:10px" data-offset-y="40"></div><span>after</span>
</div>"#,
    );
}
