//! `writing-mode`: vertical text in a box of its own (an orthogonal flow), expressed against the
//! layout dump.

mod common;

use serde_json::{Value, json};

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("writing_mode__{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("writing_mode__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn f(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap()
}

/// The rects that are filled with `color`.
fn rects_with(dump: &Value, color: [u8; 3]) -> Vec<Value> {
    let color = json!([color[0], color[1], color[2], 255]);
    dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["color"] == color)
        .cloned()
        .collect()
}

/// The matrices of the text lines, in order: `[a, b, c, d, e, f]`.
fn text_matrices(dump: &Value) -> Vec<[f64; 6]> {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|paragraph| paragraph["ops"].as_array().unwrap().iter())
        .filter(|op| op["op"] == "transform")
        .map(|op| {
            let m = op["matrix"].as_array().unwrap();
            std::array::from_fn(|i| m[i].as_f64().unwrap())
        })
        .collect()
}

const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const RED: [u8; 3] = [255, 0, 0];

const STYLE: &str = "@page { size: 600px 400px; margin: 0 } body { margin: 10px; font: 16px/20px sans-serif }";

#[test]
fn the_axes_of_a_vertical_box_are_turned() {
    // `width` and `height` are physical: the lines are as long as the box is high
    let dump = dump(
        "axes",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-rl; width: 120px; height: 200px; padding: 6px; border: 2px solid; background: #0f0 }}
            </style><div class="v">one two three four five six seven eight nine ten</div>"#
        ),
    );
    let boxes = rects_with(&dump, GREEN);
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!((f(&boxes[0], "width"), f(&boxes[0], "height")), (136.0, 216.0));
}

#[test]
fn lines_run_down_and_stack_from_right_to_left_in_vertical_rl() {
    let dump = dump(
        "rl",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-rl; width: 100px; height: 100px; background: #0f0 }}
            </style><div class="v">aaaa bbbb cccc dddd eeee ffff</div>"#
        ),
    );
    let lines = text_matrices(&dump);
    assert!(lines.len() >= 3, "{lines:?}");
    // the text is turned a quarter clockwise: its x axis points down
    for line in &lines {
        assert_eq!(&line[..4], &[0.0, -1.0, 1.0, 0.0]);
    }
    // every line starts at the top of the box, and the next line is one line height to the left
    assert!(lines.windows(2).all(|pair| pair[0][5] == pair[1][5]));
    assert!(lines.windows(2).all(|pair| (pair[0][4] - pair[1][4] - 20.0).abs() < 0.01), "{lines:?}");
}

#[test]
fn lines_stack_from_left_to_right_in_vertical_lr() {
    let dump = dump(
        "lr",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-lr; width: 100px; height: 100px; background: #0f0 }}
            </style><div class="v">aaaa bbbb cccc dddd eeee ffff</div>"#
        ),
    );
    let lines = text_matrices(&dump);
    assert!(lines.len() >= 3, "{lines:?}");
    // the glyphs are turned the same way as in vertical-rl, but the next line is on the right
    assert!(lines.iter().all(|line| line[..4] == [0.0, -1.0, 1.0, 0.0]));
    assert!(lines.windows(2).all(|pair| (pair[1][4] - pair[0][4] - 20.0).abs() < 0.01), "{lines:?}");
}

#[test]
fn physical_margins_follow_the_physical_sides() {
    // `margin-left` is the margin after the box in vertical-rl, where blocks stack to the left
    let dump = dump(
        "margins",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-rl; width: 200px; height: 100px }}
            .a {{ margin: 0 0 0 30px; background: #00f; height: 100px }}
            .b {{ margin: 0; background: #f00; height: 100px }}
            </style><div class="v"><p class="a" style="width: 40px"></p><p class="b" style="width: 40px"></p></div>"#
        ),
    );
    let (a, b) = (&rects_with(&dump, BLUE)[0], &rects_with(&dump, RED)[0]);
    // a is at the right edge of the box, b is 30px to its left
    assert_eq!(f(a, "x") + f(a, "width"), 210.0);
    assert_eq!(f(a, "x") - (f(b, "x") + f(b, "width")), 30.0);
}

#[test]
fn horizontal_text_in_a_vertical_box_is_laid_out_horizontally() {
    let dump = dump(
        "orthogonal",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-rl; width: 200px; height: 100px }}
            .h {{ writing-mode: horizontal-tb; width: 80px; height: 40px; background: #0f0 }}
            </style><div class="v"><div class="h">horizontal</div></div>"#
        ),
    );
    let boxes = rects_with(&dump, GREEN);
    assert_eq!((f(&boxes[0], "width"), f(&boxes[0], "height")), (80.0, 40.0));
    // its text is not turned
    assert!(text_matrices(&dump).is_empty());
}

#[test]
fn an_inline_block_can_be_vertical() {
    let dump = dump(
        "inline_block",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ display: inline-block; writing-mode: vertical-rl; width: 60px; height: 120px; background: #0f0 }}
            </style>text <span class="v">vertical</span> text"#
        ),
    );
    let boxes = rects_with(&dump, GREEN);
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert_eq!((f(&boxes[0], "width"), f(&boxes[0], "height")), (60.0, 120.0));
    assert!(text_matrices(&dump).iter().any(|line| line[..4] == [0.0, -1.0, 1.0, 0.0]));
}

#[test]
fn cjk_characters_stand_upright_and_latin_ones_are_turned() {
    let dump = dump(
        "orientation",
        &format!(
            r#"<!DOCTYPE html><html lang="ja"><meta charset="utf-8"><style>{STYLE}
            .v {{ writing-mode: vertical-rl; width: 100px; height: 300px }}
            </style><div class="v">日本語 text 日本</div>"#
        ),
    );
    let matrices = text_matrices(&dump);
    let upright = matrices.iter().filter(|m| m[..4] == [1.0, 0.0, 0.0, 1.0]).count();
    // one text matrix for each of the five ideographs
    assert_eq!(upright, 5, "{matrices:?}");
    // the line and the turned text between the ideographs
    assert!(matrices.iter().any(|m| m[..4] == [0.0, -1.0, 1.0, 0.0]));
    // the upright characters follow each other down the line, one em apart
    let ys: Vec<f64> = matrices.iter().filter(|m| m[..4] == [1.0, 0.0, 0.0, 1.0]).map(|m| m[5]).collect();
    assert!((ys[1] - ys[0] - 16.0).abs() < 0.5, "{ys:?}");
}

#[test]
fn text_orientation_upright_stands_latin_characters_up() {
    let dump = dump(
        "upright",
        &format!(
            r#"<!DOCTYPE html><style>{STYLE}
            .v {{ writing-mode: vertical-rl; text-orientation: upright; width: 100px; height: 300px }}
            </style><div class="v">abc</div>"#
        ),
    );
    let matrices = text_matrices(&dump);
    assert_eq!(matrices.iter().filter(|m| m[..4] == [1.0, 0.0, 0.0, 1.0]).count(), 3, "{matrices:?}");
}

#[test]
fn a_vertical_box_is_not_split_between_pages() {
    // a block that fills most of the page, and a vertical box that does not fit below it
    let dump = dump(
        "pages",
        r#"<!DOCTYPE html><style>@page { size: 300px 200px; margin: 0 } body { margin: 0; font: 16px/20px sans-serif }
        .before { height: 120px }
        .v { writing-mode: vertical-rl; width: 100px; height: 100px; background: #0f0 }
        </style><div class="before"></div><div class="v">aaaa bbbb cccc</div>"#,
    );
    let boxes = rects_with(&dump, GREEN);
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    // the box moved as a whole to the second page, with its text
    assert_eq!(boxes[0]["page"], 1);
    let text = dump["paint"]["text"].as_array().unwrap();
    assert!(text.iter().all(|paragraph| paragraph["page"] == 1), "{text:?}");
    assert_eq!(f(&boxes[0], "y"), 0.0);
}
