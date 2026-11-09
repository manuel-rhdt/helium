//! Tests for the `color` property, expressed against the layout dump.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("color__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The values of all `color` ops in drawing order.
fn text_colors(dump: &Value) -> Vec<Value> {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|run| run["ops"].as_array().unwrap())
        .filter(|op| op["op"] == "color")
        .map(|op| op["value"].clone())
        .collect()
}

#[test]
fn text_color_is_applied_and_inherited() {
    let dump = dump(
        "inherit",
        r#"<p style="color: #ff0000">red <span>still red</span></p>"#,
    );
    let colors = text_colors(&dump);
    assert!(!colors.is_empty());
    assert!(colors.iter().all(|c| *c == colors[0]), "{:?}", colors);
    assert!(colors[0].to_string().contains("255"), "{:?}", colors[0]);
}

#[test]
fn nested_element_overrides_color() {
    let dump = dump(
        "override",
        r#"<p style="color: #ff0000">a <span style="color: #0000ff">b</span> c</p>"#,
    );
    // red, blue, red again
    assert_eq!(text_colors(&dump).len(), 3, "{:?}", text_colors(&dump));
}

/// Assert that `actual` (an `[r, g, b, a]` array from the dump) is `expected` within `tolerance`.
fn assert_color(actual: &Value, expected: [u8; 4], tolerance: i32, what: &str) {
    let channels = actual.as_array().unwrap();
    for (i, want) in expected.iter().enumerate() {
        let got = channels[i].as_u64().unwrap() as i32;
        assert!(
            (got - *want as i32).abs() <= tolerance,
            "{what}: channel {i} is {got}, expected {want} ({actual:?})"
        );
    }
}

#[test]
fn modern_colour_spaces_render_as_srgb() {
    // all of these are sRGB red
    let spaces = [
        ("oklch", "oklch(62.8% 0.2577 29.23)"),
        ("oklab", "oklab(62.8% 0.2249 0.1258)"),
        ("lab", "lab(54.288 80.798 69.879)"),
        ("lch", "lch(54.288 106.824 40.855)"),
        ("color-srgb", "color(srgb 1 0 0)"),
        ("color-xyz", "color(xyz-d65 0.4124 0.2126 0.0193)"),
    ];
    for (name, value) in spaces {
        let dump = dump(name, &format!(r#"<p style="color: {value}">x</p>"#));
        let colors = text_colors(&dump);
        assert_eq!(colors.len(), 1, "{name}: {colors:?}");
        assert_color(&colors[0], [255, 0, 0, 255], 2, name);
    }
}

#[test]
fn border_defaults_to_current_color() {
    let dump = dump(
        "border",
        r#"<div style="color: #00ff00; border: 2px solid; width: 10px; height: 10px"></div>"#,
    );
    let rects = dump["paint"]["rects"].as_array().unwrap();
    assert_eq!(rects.len(), 4);
    for rect in rects {
        assert_eq!(rect["color"], serde_json::json!([0, 255, 0, 255]));
    }
}
