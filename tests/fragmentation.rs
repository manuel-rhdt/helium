//! Boxes that are split across pages, expressed against the layout dump.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn rects(name: &str, html: &str) -> Vec<Value> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(
            dir.join("wpt-pdfs")
                .join(format!("fragmentation__{}.pdf", name)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["rects"].as_array().unwrap().clone()
}

fn f(rect: &Value, key: &str) -> f64 {
    rect[key].as_f64().unwrap()
}

/// A box with a background and a border that is taller than a page: 60 lines of 20px.
fn tall_box() -> String {
    let lines = "<div style=\"height: 20px\"></div>".repeat(60);
    format!(
        r#"<style>body {{ margin: 0 }}</style>
           <div style="border: 4px solid #ff0000; padding: 10px; background-color: #00ff00">{}</div>"#,
        lines
    )
}

#[test]
fn box_across_a_page_break_is_split() {
    let rects = rects("split", &tall_box());
    let on = |page: u64| {
        rects
            .iter()
            .filter(move |r| r["page"] == page)
            .collect::<Vec<_>>()
    };
    let (first, second) = (on(0), on(1));
    assert!(
        !first.is_empty() && !second.is_empty(),
        "the box should be painted on both pages"
    );
    assert!(on(2).is_empty());

    let red = serde_json::json!([255, 0, 0, 255]);
    let horizontal = |rs: &[&Value]| {
        rs.iter()
            .filter(|r| r["color"] == red && f(r, "width") > f(r, "height"))
            .count()
    };
    // the top border is only on the first page and the bottom border only on the last
    assert_eq!(horizontal(&first), 1);
    assert_eq!(horizontal(&second), 1);
    let top = first
        .iter()
        .find(|r| r["color"] == red && f(r, "width") > f(r, "height"))
        .unwrap();
    let bottom = second
        .iter()
        .find(|r| r["color"] == red && f(r, "width") > f(r, "height"))
        .unwrap();
    assert!(
        f(top, "y") < 200.0,
        "the top border should be at the top of the first fragment"
    );
    assert!(
        f(bottom, "y") > f(top, "y"),
        "the bottom border ends the second fragment"
    );

    // the side borders and the background continue across the break
    let vertical = |rs: &[&Value]| {
        rs.iter()
            .filter(|r| r["color"] == red && f(r, "height") > f(r, "width"))
            .count()
    };
    assert_eq!(vertical(&first), 2);
    assert_eq!(vertical(&second), 2);
}

#[test]
fn first_fragment_reaches_the_bottom_of_the_page_and_the_next_starts_at_the_top() {
    let rects = rects("extent", &tall_box());
    let green = serde_json::json!([0, 255, 0, 255]);
    let backgrounds: Vec<&Value> = rects.iter().filter(|r| r["color"] == green).collect();
    assert_eq!(backgrounds.len(), 2);
    // A4 with 2.5cm margins: the content area is 933.5px high
    let page_bottom = 94.488 + 933.543;
    let (first, second) = (backgrounds[0], backgrounds[1]);
    assert!((f(first, "y") + f(first, "height") - page_bottom).abs() < 0.01);
    assert!((f(second, "y") - 94.488).abs() < 0.01);
    assert_eq!(first["page"], 0);
    assert_eq!(second["page"], 1);
}

#[test]
fn box_on_a_single_page_is_not_split() {
    let rects = rects(
        "single",
        r#"<div style="border: 4px solid #ff0000; padding: 10px"><p>short</p></div>"#,
    );
    assert_eq!(rects.len(), 4);
}
