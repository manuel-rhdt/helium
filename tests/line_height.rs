//! Line box heights, per CSS 2.1 §10.8: the pitch of a paragraph is `line-height × font-size`,
//! with no lower bound, and a line box starts with the strut of the block container.

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
        .arg(
            dir.join("wpt-pdfs")
                .join(format!("line_height__{}.pdf", name)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The content height of the element with the given `id`, in px.
fn content_height(dump: &Value, id: &str) -> f64 {
    fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
        if node.get("id").and_then(Value::as_str) == Some(id) {
            return Some(node);
        }
        node.get("children")?.as_array()?.iter().find_map(|c| find(c, id))
    }
    let element = find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {id}"));
    element["content_box"]["height"].as_f64().unwrap()
}

/// A page whose `body` sets the font, with paragraphs of forced line breaks so the line count is
/// independent of the available width.
fn page(size: f64, line_height: f64, paragraphs: &str) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>
@page {{ width: 300pt; height: 300pt; margin: 20pt }}
body {{ font-family: "Liberation Serif", serif; font-size: {size}pt; line-height: {line_height}; margin: 0 }}
p {{ margin: 0 }}
</style></head><body>{paragraphs}</body></html>"#
    )
}

/// 1 pt is 4/3 px.
fn pt(px: f64) -> f64 {
    px * 3.0 / 4.0
}

#[test]
fn line_pitch_is_line_height_times_font_size() {
    // A line-height smaller than 1 shrinks the line box below the font's natural height; CSS has
    // no minimum, so 0.6 must give a pitch of 0.6 em (the glyphs may overlap).
    for (size, line_height) in [(6.0, 1.2), (6.0, 0.6), (10.0, 1.2), (10.0, 2.0), (16.0, 1.5)] {
        let pitch_pt = size * line_height;
        let html = page(size, line_height, r#"<p id="p">alpha<br>beta<br>gamma</p>"#);
        let name = format!("pitch_{size}_{line_height}");
        let height = pt(content_height(&dump(&name, &html), "p"));
        let expected = 3.0 * pitch_pt;
        assert!(
            (height - expected).abs() < 0.5,
            "font-size {size}pt, line-height {line_height}: three lines measure {height:.2}pt, \
             expected {expected:.2}pt"
        );
    }
}

#[test]
fn empty_line_is_one_pitch_tall() {
    let size = 10.0;
    let line_height = 1.5;
    let pitch_pt = size * line_height;

    let two = pt(content_height(
        &dump(
            "empty_line_2",
            &page(size, line_height, r#"<p id="p">alpha<br>beta</p>"#),
        ),
        "p",
    ));
    let three = pt(content_height(
        &dump(
            "empty_line_3",
            &page(size, line_height, r#"<p id="p">alpha<br><br>beta</p>"#),
        ),
        "p",
    ));

    assert!(
        (two - 2.0 * pitch_pt).abs() < 0.5,
        "two lines measure {two:.2}pt, expected {:.2}pt",
        2.0 * pitch_pt
    );
    assert!(
        ((three - two) - pitch_pt).abs() < 0.5,
        "an empty line adds {:.2}pt, expected {:.2}pt",
        three - two,
        pitch_pt
    );
}

#[test]
fn empty_block_is_one_pitch_tall() {
    // A block with no inline content still gets one line box, whose height is the strut: the
    // block's own font and line-height.
    let size = 10.0;
    let line_height = 1.2;
    let pitch_pt = size * line_height;
    let height = pt(content_height(
        &dump(
            "empty_block",
            &page(size, line_height, r#"<p id="p"></p>"#),
        ),
        "p",
    ));
    assert!(
        (height - pitch_pt).abs() < 0.5,
        "an empty block measures {height:.2}pt, expected one pitch of {pitch_pt:.2}pt"
    );
}
