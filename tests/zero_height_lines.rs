//! A line whose font size is zero has no height, and must not crash the layout. `font-size: 0`
//! is used for spacers and to take the white space between inline boxes out of a line.

mod common;

fn check_layout(name: &str, html: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("zero_height_lines__{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::helium()
        .unwrap()
        .args(["--check-layout"])
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("zero_height_lines__{}.pdf", name)))
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
fn a_paragraph_with_a_font_size_of_zero_has_no_height() {
    check_layout(
        "paragraph",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font-family: sans-serif; font-size: 16px; }
  p { margin: 0; padding: 0; border: 0; }
</style>
<p data-expected-height="0" style="font-size: 0pt; line-height: 1.2">&nbsp;</p>
<p data-expected-height="20" style="line-height: 20px">text</p>
"#,
    );
}

/// The inline box of the span has no height either, so it does not make the line any higher
/// (with the `line-height` of 20px of the div it would: its box would be 20px high, centered on
/// its own baseline).
#[test]
fn an_inline_with_no_height_does_not_change_the_line() {
    check_layout(
        "inline",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font-family: sans-serif; font-size: 16px; }
  div { line-height: 20px; }
</style>
<div data-expected-height="20">before <span style="font-size: 0; line-height: 0">hidden</span> after</div>
"#,
    );
}
