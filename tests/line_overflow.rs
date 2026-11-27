//! Lines of left-aligned text must not be wider than their box. Only justified text may shrink
//! its spaces to squeeze a word onto a line.

mod common;

fn check_layout(name: &str, html: &str) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("line_overflow__{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::umbruch()
        .unwrap()
        .args(["--check-layout"])
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("line_overflow__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// "The quick brown fox jumps" is about 190px wide in the built-in sans-serif at 16px, so it does not fit into 185px.
/// The spaces of left-aligned text do not shrink, so "jumps" moves to the next line: four
/// lines of 20px instead of three.
#[test]
fn left_aligned_text_does_not_cross_the_right_edge() {
    check_layout(
        "left",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font-family: sans-serif; font-size: 16px; }
  div { width: 185px; line-height: 20px; background: #fc6; }
</style>
<div data-expected-height="80">The quick brown fox jumps over the lazy dog and keeps running far away</div>
"#,
    );
}

#[test]
fn inline_block_text_does_not_cross_the_right_edge() {
    check_layout(
        "inline_block",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font-family: sans-serif; font-size: 16px; }
  div { display: inline-block; max-width: 185px; line-height: 20px; background: #fc6; }
</style>
<div data-expected-height="80">The quick brown fox jumps over the lazy dog and keeps running far away</div>
"#,
    );
}

/// Thai has no spaces between words. The line breaker finds the words with a dictionary, so a
/// sentence wraps inside its box instead of forming one line that sticks out of it.
#[cfg(feature = "embedded-fonts")]
#[test]
fn thai_text_wraps_inside_its_box() {
    check_layout(
        "thai",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; font-size: 16px; }
  div { width: 120px; line-height: 20px; background: #fc6; }
</style>
<div lang="th" data-expected-height="100">ภาษาไทยเป็นภาษาที่ไม่มีช่องว่างระหว่างคำและต้องใช้พจนานุกรมในการตัดคำ</div>
"#,
    );
}
