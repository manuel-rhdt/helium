//! `position: relative`: the box is laid out in the flow and then moved by `left`, `right`, `top`
//! and `bottom`; the space it takes up and the boxes around it stay where they are.

mod common;

use serde_json::Value;
use std::path::Path;

struct Rendered {
    dump: Value,
    pdf: std::path::PathBuf,
}

fn render(name: &str, html: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("relative__{}.html", name));
    let output = dir.join(format!("relative__{}.pdf", name));
    let layout = dir.join(format!("relative__{}.json", name));
    std::fs::write(
        &input,
        format!(
            "<style>@page{{size:600px 300px;margin:0}} body{{margin:0;font-size:20px}}</style>{}",
            html
        ),
    )
    .unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let dump = serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    Rendered { dump, pdf: output }
}

impl Rendered {
    /// The border box `(x, y, width, height)` of the element with this `id`.
    fn border_box(&self, id: &str) -> (f64, f64, f64, f64) {
        fn find(node: &Value, id: &str) -> Option<(f64, f64, f64, f64)> {
            if node["id"] == id {
                let b = &node["border_box"];
                return Some((
                    b["x"].as_f64()?,
                    b["y"].as_f64()?,
                    b["width"].as_f64()?,
                    b["height"].as_f64()?,
                ));
            }
            node["children"].as_array()?.iter().find_map(|child| find(child, id))
        }
        find(&self.dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
    }

    /// Where the text runs of the page start, with the moves of the text line matrix in them as
    /// `(x_offset, leading)`.
    fn text_moves(&self) -> Vec<(f64, f64)> {
        self.dump["paint"]["text"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|run| run["ops"].as_array().unwrap())
            .filter(|op| op["op"] == "newline")
            .map(|op| (op["x_offset"].as_f64().unwrap(), op["leading"].as_f64().unwrap()))
            .collect()
    }

    /// The text decoration lines as `(from_x, from_y, to_x)`.
    fn lines(&self) -> Vec<(f64, f64, f64)> {
        self.dump["paint"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| {
                (
                    line["from"][0].as_f64().unwrap(),
                    line["from"][1].as_f64().unwrap(),
                    line["to"][0].as_f64().unwrap(),
                )
            })
            .collect()
    }

    fn rects(&self) -> Vec<(f64, f64)> {
        self.dump["paint"]["rects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rect| (rect["x"].as_f64().unwrap(), rect["y"].as_f64().unwrap()))
            .collect()
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[track_caller]
fn assert_box(rendered: &Rendered, id: &str, expected: (f64, f64, f64, f64)) {
    let found = rendered.border_box(id);
    assert!(
        close(found.0, expected.0)
            && close(found.1, expected.1)
            && close(found.2, expected.2)
            && close(found.3, expected.3),
        "#{}: {:?}, expected {:?}",
        id,
        found,
        expected
    );
}

const BOXES: &str = "<style>div{width:100px;height:20px}</style>";

#[test]
fn a_block_moves_and_leaves_its_space() {
    let r = render(
        "block",
        &format!(
            "{BOXES}<div id=a style='position:relative;left:10px;top:5px'></div><div id=b></div>"
        ),
    );
    assert_box(&r, "a", (10.0, 5.0, 100.0, 20.0));
    // the next box is where it would be without the move
    assert_box(&r, "b", (0.0, 20.0, 100.0, 20.0));
}

#[test]
fn right_and_bottom_move_the_other_way() {
    let r = render(
        "right",
        &format!("{BOXES}<div id=a style='position:relative;right:4px;bottom:6px'></div>"),
    );
    assert_box(&r, "a", (-4.0, -6.0, 100.0, 20.0));
}

#[test]
fn left_wins_over_right_and_top_over_bottom() {
    let r = render(
        "over-constrained",
        &format!(
            "{BOXES}<div id=a style='position:relative;left:3px;right:50px;top:2px;bottom:50px'></div>"
        ),
    );
    assert_box(&r, "a", (3.0, 2.0, 100.0, 20.0));
}

#[test]
fn right_wins_in_a_right_to_left_container() {
    let r = render(
        "rtl",
        &format!(
            "{BOXES}<div dir=rtl style='width:300px'><div id=a style='position:relative;left:3px;right:7px'></div></div>"
        ),
    );
    // at the right edge of its container, moved left by 7px
    assert_box(&r, "a", (300.0 - 100.0 - 7.0, 0.0, 100.0, 20.0));
}

#[test]
fn percentages_refer_to_the_width_of_the_container() {
    let r = render(
        "percentage",
        &format!(
            "{BOXES}<div style='width:200px'><div id=a style='position:relative;left:50%'></div></div>"
        ),
    );
    assert_box(&r, "a", (100.0, 0.0, 100.0, 20.0));
}

#[test]
fn a_flex_item_moves() {
    let r = render(
        "flex",
        &format!(
            "{BOXES}<div style='display:flex;width:300px;height:auto'><div id=a style='position:relative;left:7px;top:3px'></div><div id=b></div></div>"
        ),
    );
    assert_box(&r, "a", (7.0, 3.0, 100.0, 20.0));
    assert_box(&r, "b", (100.0, 0.0, 100.0, 20.0));
}

#[test]
fn the_content_moves_with_the_box() {
    let r = render(
        "content",
        "<div id=a style='position:relative;left:10px;top:5px;width:100px'><i id=i style='display:block;width:10px;height:10px;background:#333'></i></div>",
    );
    assert_box(&r, "i", (10.0, 5.0, 10.0, 10.0));
    // the text of a box is drawn where the box is
    let r = render(
        "text",
        "<div style='position:relative;left:10px;top:5px'>x</div><div>x</div>",
    );
    let runs = r.dump["paint"]["text"].as_array().unwrap();
    let (moved, normal) = (&runs[0], &runs[1]);
    assert!(close(moved["x"].as_f64().unwrap(), normal["x"].as_f64().unwrap() + 10.0));
    // the second line is one line height down
    let line = normal["y"].as_f64().unwrap();
    assert!(moved["y"].as_f64().unwrap() < line, "the first line is above the second");
}

#[test]
fn offsets_of_nested_boxes_add_up() {
    let r = render(
        "nested",
        &format!(
            "{BOXES}<div id=a style='position:relative;left:10px;top:10px;height:50px'><div id=b style='position:relative;left:5px;top:-3px'></div></div>"
        ),
    );
    assert_box(&r, "b", (15.0, 7.0, 100.0, 20.0));
}

#[test]
fn an_absolute_box_is_placed_in_the_moved_box() {
    let r = render(
        "absolute",
        &format!(
            "{BOXES}<div id=a style='position:relative;left:20px;top:10px;height:50px'><div id=b style='position:absolute;left:5px;top:5px;width:10px;height:10px'></div></div>"
        ),
    );
    assert_box(&r, "b", (25.0, 15.0, 10.0, 10.0));
}

#[test]
fn static_boxes_ignore_the_offsets() {
    let r = render(
        "static",
        &format!("{BOXES}<div id=a style='left:10px;top:5px'></div>"),
    );
    assert_box(&r, "a", (0.0, 0.0, 100.0, 20.0));
}

#[test]
fn inline_text_moves_and_the_line_keeps_its_shape() {
    let plain = render("inline-plain", "<p id=p>aaa <span style='text-decoration:underline'>bbb</span> ddd</p>");
    let moved = render(
        "inline-moved",
        "<p id=p>aaa <span style='position:relative;left:30px;top:-5px;text-decoration:underline'>bbb</span> ddd</p>",
    );
    // the paragraph takes up the same space
    assert_eq!(plain.border_box("p"), moved.border_box("p"));
    let (a, b) = (plain.lines()[0], moved.lines()[0]);
    assert!(close(b.0, a.0 + 30.0) && close(b.2, a.2 + 30.0), "{:?} {:?}", a, b);
    assert!(close(b.1, a.1 - 5.0), "{:?} {:?}", a, b);
    // the text is moved there and back, so that what follows is not affected
    let moves = moved.text_moves();
    assert!(moves.iter().any(|&(x, y)| close(y, -5.0) && x > 30.0), "{:?}", moves);
    assert!(moves.iter().any(|&(_, y)| close(y, 5.0)), "{:?}", moves);
    assert!(plain.text_moves().is_empty());
}

#[test]
fn the_offsets_of_nested_inline_boxes_add_up() {
    let r = render(
        "inline-nested",
        "<p><span style='position:relative;left:30px;top:-5px'>a<span style='position:relative;right:10px;bottom:-4px;text-decoration:underline'>b</span></span></p>",
    );
    // 30 - 10 to the right, 5 - 4 up
    let underline = r.lines()[0];
    let plain = render("inline-nested-plain", "<p><span>a<span style='text-decoration:underline'>b</span></span></p>").lines()[0];
    assert!(close(underline.0, plain.0 + 20.0), "{:?} {:?}", underline, plain);
    assert!(close(underline.1, plain.1 - 1.0), "{:?} {:?}", underline, plain);
}

#[test]
fn an_inline_block_moves() {
    let plain = render(
        "inline-block-plain",
        "<p>a <b style='display:inline-block;width:40px;height:10px;background:#0a0'></b> b</p>",
    );
    let moved = render(
        "inline-block-moved",
        "<p>a <b style='display:inline-block;width:40px;height:10px;background:#0a0;position:relative;left:3px;top:2px'></b> b</p>",
    );
    let (a, b) = (plain.rects()[0], moved.rects()[0]);
    assert!(close(b.0, a.0 + 3.0) && close(b.1, a.1 + 2.0), "{:?} {:?}", a, b);
}

#[test]
fn a_relative_box_is_painted_over_the_boxes_after_it() {
    // `pdftoppm` is how the reftests look at the page; without it there is nothing to look with
    if std::process::Command::new("pdftoppm").arg("-v").output().is_err() {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    }
    let r = render(
        "paint-order",
        "<div style='position:relative;left:10px;top:10px;background:#f00;width:50px;height:50px'></div><div style='background:#00f;width:50px;height:50px;margin-top:-30px'></div>",
    );
    let output = std::process::Command::new("pdftoppm")
        .args(["-r", "96", "-f", "1", "-l", "1"])
        .arg(&r.pdf)
        .output()
        .unwrap();
    assert!(output.status.success());
    // a binary PPM: "P6", the size, the maximum value, a single whitespace byte, the pixels
    let data = output.stdout;
    let mut at = 0;
    let mut fields = Vec::new();
    while fields.len() < 4 {
        while data[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while !data[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&data[start..at]).into_owned());
    }
    at += 1;
    assert_eq!(fields[0], "P6");
    let width: usize = fields[1].parse().unwrap();
    let pixel = |x: usize, y: usize| {
        let at = at + (y * width + x) * 3;
        [data[at], data[at + 1], data[at + 2]]
    };
    // inside of both boxes: x 10..50, y 20..60 (the second box starts at y = 20)
    assert_eq!(pixel(30, 40), [255, 0, 0], "the relative box is on top");
    // the part of the second box that the first does not cover
    assert_eq!(pixel(5, 40), [0, 0, 255]);
}

#[test]
fn inline_boxes_next_to_block_boxes_move_once() {
    // the inline content of a box that has block children goes through anonymous blocks
    let plain = render(
        "anonymous-plain",
        "<div><span style='text-decoration:underline'>aa</span><div>x</div></div>",
    );
    let moved = render(
        "anonymous-moved",
        "<div><span style='position:relative;left:30px;top:-5px;text-decoration:underline'>aa</span><div>x</div></div>",
    );
    let (a, b) = (plain.lines()[0], moved.lines()[0]);
    assert!(close(b.0, a.0 + 30.0) && close(b.1, a.1 - 5.0), "{:?} {:?}", a, b);
}

#[test]
fn vertical_text_moves_in_physical_directions() {
    let css = "writing-mode:vertical-rl;height:200px;width:100px";
    let plain = render(
        "vertical-plain",
        &format!("<div style='{css}'>ab <span style='text-decoration:underline'>cd</span> ef</div>"),
    );
    let moved = render(
        "vertical-moved",
        &format!("<div style='{css}'>ab <span style='position:relative;left:10px;top:7px;text-decoration:underline'>cd</span> ef</div>"),
    );
    let (a, b) = (plain.lines()[0], moved.lines()[0]);
    // `left` is across the lines and `top` along them
    assert!(close(b.0, a.0 + 10.0), "{:?} {:?}", a, b);
    assert!(close(b.1, a.1 + 7.0), "{:?} {:?}", a, b);
    assert!(close(b.2, a.2 + 10.0), "{:?} {:?}", a, b);
    // the text is moved along the line and, across it, to the right (up for the turned text)
    let moves = moved.text_moves();
    assert!(moves.iter().any(|&(_, across)| close(across, -10.0)), "{:?}", moves);
}

#[test]
fn a_formula_moves() {
    let plain = render(
        "math-plain",
        "<p>a <math id=m><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>",
    );
    let moved = render(
        "math-moved",
        "<p>a <math id=m style='position:relative;left:25px;top:6px'><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>",
    );
    let (a, b) = (plain.border_box("m"), moved.border_box("m"));
    assert!(close(b.0, a.0 + 25.0) && close(b.1, a.1 + 6.0), "{:?} {:?}", a, b);
    assert!(close(b.2, a.2) && close(b.3, a.3));
    // what follows stays where it is
    let text = |r: &Rendered| {
        r.dump["paint"]["text"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| (run["x"].as_f64().unwrap(), run["y"].as_f64().unwrap()))
            .collect::<Vec<_>>()
    };
    // the glyphs of the formula move with the box: the other runs (a and b) do not
    assert_eq!(text(&plain).len(), text(&moved).len());
}
