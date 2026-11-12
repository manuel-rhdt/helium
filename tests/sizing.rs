//! `box-sizing`, `min-width`, `max-width`, `min-height` and `max-height`, which only the Taffy
//! layout engine supports. `data-expected-width` and `data-expected-height` are border box sizes.

mod common;

use std::process::Output;

fn run(name: &str, html: &str) -> Output {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("sizing__{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    common::helium()
        .unwrap()
        .args(["--layout-engine", "taffy", "--check-layout"])
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("sizing__{}.pdf", name)))
        .output()
        .unwrap()
}

fn check_layout(name: &str, html: &str) {
    let output = run(name, html);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn box_sizing_content_box_is_the_default() {
    check_layout(
        "content_box_default",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  div { width: 100px; height: 20px; padding: 10px; border: 5px solid black; }
</style>
<div data-expected-width="130" data-expected-height="50"></div>
"#,
    );
}

#[test]
fn box_sizing_border_box_includes_padding_and_border() {
    check_layout(
        "border_box",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  div { box-sizing: border-box; width: 100px; height: 50px; padding: 10px; border: 5px solid black; }
</style>
<div data-expected-width="100" data-expected-height="50"></div>
"#,
    );
}

#[test]
fn box_sizing_explicit_content_box_and_keyword_reset() {
    check_layout(
        "content_box_explicit",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  div { box-sizing: border-box; width: 100px; height: 20px; padding: 10px; }
  #a { box-sizing: content-box; }
  #b { box-sizing: initial; }
</style>
<div id="a" data-expected-width="120" data-expected-height="40"></div>
<div id="b" data-expected-width="120" data-expected-height="40"></div>
"#,
    );
}

#[test]
fn box_sizing_is_not_inherited() {
    check_layout(
        "box_sizing_not_inherited",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  #outer { box-sizing: border-box; width: 200px; height: 20px; }
  #inner { width: 100px; height: 10px; padding: 10px; }
</style>
<div id="outer"><div id="inner" data-expected-width="120" data-expected-height="30"></div></div>
"#,
    );
}

#[test]
fn min_and_max_width() {
    check_layout(
        "min_max_width",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  div { height: 10px; }
  #minw { width: 50px; min-width: 120px; }
  #maxw { width: 300px; max-width: 150px; }
  #auto_max { max-width: 250px; }
  #auto_min { width: auto; min-width: 500px; }
  #pct_max { width: 50%; max-width: 120px; }
  #pct_min { width: 10%; min-width: 25%; }
  #min_wins { width: 10px; min-width: 100px; max-width: 50px; }
  #none { width: 300px; max-width: none; }
</style>
<div id="minw" data-expected-width="120"></div>
<div id="maxw" data-expected-width="150"></div>
<div id="auto_max" data-expected-width="250"></div>
<div id="auto_min" data-expected-width="500"></div>
<div id="pct_max" data-expected-width="120"></div>
<div id="pct_min" data-expected-width="100"></div>
<div id="min_wins" data-expected-width="100"></div>
<div id="none" data-expected-width="300"></div>
"#,
    );
}

#[test]
fn min_and_max_height() {
    check_layout(
        "min_max_height",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  #minh { height: 10px; min-height: 40px; }
  #maxh { height: 100px; max-height: 30px; }
  #grow { min-height: 60px; }
  #content { max-height: 15px; }
  #content span { display: block; height: 100px; }
  #min_wins { height: 10px; min-height: 40px; max-height: 20px; }
  #none { height: 70px; max-height: none; }
</style>
<div id="minh" data-expected-height="40"></div>
<div id="maxh" data-expected-height="30"></div>
<div id="grow" data-expected-height="60"></div>
<div id="content" data-expected-height="15"><span></span></div>
<div id="min_wins" data-expected-height="40"></div>
<div id="none" data-expected-height="70"></div>
"#,
    );
}

#[test]
fn min_and_max_size_follow_box_sizing() {
    check_layout(
        "min_max_box_sizing",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  div { padding: 10px; height: 10px; }
  /* the limits apply to the content box: 100 + 20 */
  #cb_max { width: 300px; max-width: 100px; }
  #cb_min { width: 10px; min-width: 100px; }
  /* the limits apply to the border box */
  #bb_max { box-sizing: border-box; width: 300px; max-width: 100px; }
  #bb_min { box-sizing: border-box; width: 10px; min-width: 100px; }
  #bb_min_h { box-sizing: border-box; min-height: 50px; }
</style>
<div id="cb_max" data-expected-width="120"></div>
<div id="cb_min" data-expected-width="120"></div>
<div id="bb_max" data-expected-width="100"></div>
<div id="bb_min" data-expected-width="100"></div>
<div id="bb_min_h" data-expected-height="50"></div>
"#,
    );
}

#[test]
fn min_and_max_size_in_flex_and_grid() {
    check_layout(
        "min_max_flex_grid",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  #flex { display: flex; }
  #flex div { height: 10px; }
  #f1 { flex: 1; max-width: 100px; }
  #f2 { width: 50px; flex-shrink: 1; min-width: 50px; }
  #grid { display: grid; grid-template-columns: 1fr 1fr; }
  #g1 { max-width: 80px; height: 10px; }
  #g2 { min-height: 40px; }
</style>
<div id="flex">
  <div id="f1" data-expected-width="100"></div>
  <div id="f2" data-expected-width="50"></div>
</div>
<div id="grid">
  <div id="g1" data-expected-width="80" data-expected-height="10"></div>
  <div id="g2" data-expected-width="200" data-expected-height="40"></div>
</div>
"#,
    );
}

#[test]
fn page_sized_columns_with_a_fixed_height_and_box_sizing() {
    // the layout of a two column page: a grid whose height is the page minus the header
    check_layout(
        "page_columns",
        r#"<!DOCTYPE html>
<style>
  @page { margin: 0; }
  body { margin: 0; width: 600px; }
  #header { height: 100px; }
  #cols { display: grid; grid-template-columns: 1fr 200px; height: 300px; }
  #main, #side { box-sizing: border-box; padding: 20px; min-height: 100px; }
</style>
<div id="header" data-expected-height="100"></div>
<div id="cols" data-expected-height="300">
  <div id="main" data-expected-width="400" data-expected-height="300"></div>
  <div id="side" data-expected-width="200" data-expected-height="300"></div>
</div>
"#,
    );
}

#[test]
fn tables_stay_border_box() {
    check_layout(
        "table_border_box",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; width: 400px; }
  table { border-collapse: separate; border-spacing: 0; width: 200px; padding: 10px; border: 5px solid black; }
</style>
<table data-expected-width="200"><tr><td>x</td></tr></table>
"#,
    );
}
