//! Flexbox and grid layout, which only the Taffy layout engine supports.

mod common;

use serde_json::Value;
use std::process::Output;

fn run(name: &str, html: &str, args: &[&str]) -> Output {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("flex_grid__{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    common::helium()
        .unwrap()
        .args(["--layout-engine", "taffy"])
        .args(args)
        .arg(&input)
        .arg(
            dir.join("wpt-pdfs")
                .join(format!("flex_grid__{}.pdf", name)),
        )
        .output()
        .unwrap()
}

fn check_layout(name: &str, html: &str) {
    let output = run(name, html, &["--check-layout"]);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn dump(name: &str, html: &str) -> Value {
    let output = run(name, html, &["--dump-layout", "-"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The element with the given id in the document tree of a layout dump.
fn element<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| element(child, id))
}

fn page_of(dump: &Value, id: &str) -> u64 {
    element(&dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))["page"]
        .as_u64()
        .unwrap()
}

// `data-offset-*` are relative to the offset parent, which is the body here.
#[test]
fn flex_row() {
    check_layout(
        "flex_row",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #row { display: flex; width: 300px; }
  #row div { height: 20px; }
  #a { width: 50px; }
  #b { flex-grow: 1; }
  #c { flex: 2; }
</style>
<div id="row" data-expected-height="20">
  <div id="a" data-offset-x="0" data-expected-width="50"></div>
  <div id="b" data-offset-x="50" data-expected-width="83.33"></div>
  <div id="c" data-offset-x="133.33" data-expected-width="166.67"></div>
</div>"#,
    );
}

#[test]
fn flex_column_with_gap() {
    check_layout(
        "flex_column",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #col { display: flex; flex-direction: column; width: 100px; gap: 10px; }
  #col div { height: 30px; }
</style>
<div id="col" data-expected-height="110">
  <div data-expected-width="100" data-offset-y="0"></div>
  <div data-offset-y="40"></div>
  <div data-offset-y="80"></div>
</div>"#,
    );
}

#[test]
fn flex_alignment() {
    check_layout(
        "flex_alignment",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #center { display: flex; justify-content: center; align-items: center; width: 200px; height: 100px; }
  #center div { width: 40px; height: 20px; }
  #end { display: flex; justify-content: space-between; width: 200px; }
  #end div { width: 40px; height: 20px; }
</style>
<div id="center">
  <div data-offset-x="80" data-offset-y="40"></div>
</div>
<div id="end">
  <div data-offset-x="0" data-offset-y="100"></div>
  <div data-offset-x="160" data-offset-y="100"></div>
</div>"#,
    );
}

#[test]
fn inline_children_of_a_flex_container_are_items() {
    check_layout(
        "flex_inline_items",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #row { display: flex; width: 300px; }
  span { width: 100px; height: 10px; }
</style>
<div id="row">
  <span data-offset-x="0" data-expected-width="100" data-expected-height="10"></span>
  <span data-offset-x="100" data-expected-width="100"></span>
</div>"#,
    );
}

#[test]
fn text_in_a_flex_container() {
    // The text becomes an anonymous flex item; it must not take over the padding of the
    // container.
    check_layout(
        "flex_text",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #row { display: flex; padding: 10px; width: 300px; }
  #item { width: 50px; height: 10px; }
</style>
<div id="row">text<div id="item" data-offset-y="10"></div></div>"#,
    );
}

#[test]
fn grid_tracks_and_placement() {
    check_layout(
        "grid",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #grid { display: grid; grid-template-columns: 100px 1fr 2fr; column-gap: 10px; row-gap: 5px; width: 400px; }
  #grid div { height: 25px; }
  #span { grid-column: 1 / 3; }
  #second-row { grid-row: 3; grid-column: span 3; }
</style>
<div id="grid" data-expected-height="85">
  <div data-offset-x="0" data-expected-width="100"></div>
  <div data-offset-x="110" data-expected-width="93.33"></div>
  <div data-offset-x="213.33" data-expected-width="186.67"></div>
  <div id="span" data-offset-x="0" data-offset-y="30" data-expected-width="203.33"></div>
  <div id="second-row" data-offset-x="0" data-offset-y="60" data-expected-width="400"></div>
</div>"#,
    );
}

/// A flex container that does not fit on the rest of a page is moved to the next page as a
/// whole, with all of its items.
#[test]
fn flex_container_is_not_broken_across_pages() {
    let dump = dump(
        "flex_page_break",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #filler { height: 850px; }
  #row { display: flex; flex-wrap: wrap; width: 200px; }
  #row div { width: 100px; height: 60px; }
</style>
<div id="filler"></div>
<div id="row"><div id="a"></div><div id="b"></div><div id="c"></div><div id="d"></div></div>"#,
    );
    assert_eq!(page_of(&dump, "filler"), 0);
    for id in ["row", "a", "b", "c", "d"] {
        assert_eq!(
            page_of(&dump, id),
            1,
            "#{} should be on the second page",
            id
        );
    }
}

/// A flex container that is taller than a page cannot be kept on one page. It must still be
/// laid out.
#[test]
fn flex_container_taller_than_a_page() {
    let dump = dump(
        "flex_tall",
        r#"<!DOCTYPE html>
<style>
  body { margin: 0; }
  #col { display: flex; flex-direction: column; }
  #col div { height: 400px; }
</style>
<div id="col"><div id="a"></div><div id="b"></div><div id="c"></div></div>"#,
    );
    assert!(element(&dump["document"], "c").is_some());
}
