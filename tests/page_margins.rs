//! `@page` margins: the `pc` unit, negative values and `auto`.

mod common;

use serde_json::Value;

/// 300pt pages, so a page is 400px wide.
const PAGE: &str = "@page { width: 300pt; height: 300pt;";

/// A paragraph without margins of its own, so that its box starts exactly at the page margin.
const BODY: &str = r#"<p id=p style="margin: 0">x</p>"#;

fn dump(name: &str, css: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("page_margins_{}.html", name));
    std::fs::write(
        &input,
        format!(r#"<style>{} body {{ margin: 0 }}</style>{}"#, css, BODY),
    )
    .unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("page_margins_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The `content_box` of the element with the given `id`: `(x, y, width, height)`.
fn content_box(dump: &Value, id: &str) -> (f64, f64, f64, f64) {
    fn find(node: &Value, id: &str) -> Option<(f64, f64, f64, f64)> {
        if node["id"] == id && node["content_box"].is_object() {
            let b = &node["content_box"];
            let f = |key: &str| b[key].as_f64().unwrap();
            return Some((f("x"), f("y"), f("width"), f("height")));
        }
        node["children"]
            .as_array()
            .and_then(|children| children.iter().find_map(|child| find(child, id)))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no box for #{id}"))
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.5
}

#[test]
fn pica_page_margins_are_twelve_points() {
    let dump = dump("pica", &format!("{} margin: 1pc }}", PAGE));
    let (x, _, width, _) = content_box(&dump, "p");
    // 1pc = 12pt = 16px, so the content area is 400px - 2 * 16px wide
    assert!(close(x, 16.0), "content starts at x = {x}, expected 16");
    assert!(close(width, 368.0), "content is {width}px wide, expected 368");
}

#[test]
fn negative_page_margins_are_clamped_to_zero() {
    let dump = dump("negative", &format!("{} margin: -20pt }}", PAGE));
    let (x, _, width, _) = content_box(&dump, "p");
    assert!(close(x, 0.0), "content starts at x = {x}, expected 0");
    assert!(
        close(width, 400.0),
        "content is {width}px wide, expected the full page width"
    );
}

#[test]
fn auto_page_margins_keep_the_default() {
    // the user agent stylesheet sets a 2.5cm page margin, which `auto` must not override
    let default = dump("default", &format!("{} }}", PAGE));
    let (x, y, width, _) = content_box(&default, "p");
    assert!(close(x, 94.49), "default margin is {x}px, expected 94.49");

    for (name, margin) in [
        ("auto", "margin: auto"),
        ("auto-top", "margin-top: auto"),
        ("auto-shorthand", "margin: auto 0"),
    ] {
        let dump = dump(name, &format!("{} {} }}", PAGE, margin));
        let (auto_x, auto_y, auto_width, _) = content_box(&dump, "p");
        assert!(
            close(auto_x, x) && close(auto_y, y) && close(auto_width, width),
            "`{margin}` changed the content box to ({auto_x}, {auto_y}) {auto_width}px wide, \
             expected ({x}, {y}) {width}px wide"
        );
    }
}
