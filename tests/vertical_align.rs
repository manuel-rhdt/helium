//! `vertical-align` of inline boxes: raised and lowered boxes, boxes aligned with the edges of the
//! line, and the height of the line they make. The font is Ahem, whose ascent is 0.8 em, descent
//! 0.2 em and x-height 0.8 em.

mod common;

use serde_json::Value;
use std::path::Path;

struct Rendered {
    dump: Value,
}

fn render(name: &str, body: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("vertical_align__{}.html", name));
    let output = dir.join(format!("vertical_align__{}.pdf", name));
    let layout = dir.join(format!("vertical_align__{}.json", name));
    std::fs::write(
        &input,
        format!(
            "<style>@page{{size:500px 300px;margin:0}} body{{margin:0;font:20px/1 Ahem}} p{{margin:0}} \
             .s{{font-size:10px;line-height:10px}}</style>{}",
            body
        ),
    )
    .unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--font-dir"])
        .arg(common::wpt_fonts())
        .arg("--dump-layout")
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Rendered { dump: serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap() }
}

impl Rendered {
    fn height(&self, id: &str) -> f64 {
        fn find(node: &Value, id: &str) -> Option<f64> {
            if node["id"] == id {
                return node["border_box"]["height"].as_f64();
            }
            node["children"].as_array()?.iter().find_map(|child| find(child, id))
        }
        find(&self.dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
    }

    /// How far each piece of text that is moved is moved down, in order: the second operand of
    /// the moves there and back (`(x, leading)`: the first ones of each pair).
    fn drops(&self) -> Vec<f64> {
        let moves: Vec<f64> = self.dump["paint"]["text"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|run| run["ops"].as_array().unwrap())
            .filter(|op| op["op"] == "newline")
            .map(|op| op["leading"].as_f64().unwrap())
            .collect();
        // every move is followed by the move back
        moves.chunks(2).map(|pair| pair[0]).collect()
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[track_caller]
fn assert_drops(rendered: &Rendered, expected: &[f64]) {
    let drops = rendered.drops();
    assert!(
        drops.len() == expected.len() && drops.iter().zip(expected).all(|(a, b)| close(*a, *b)),
        "{:?}, expected {:?}",
        drops,
        expected
    );
}

#[test]
fn baseline_changes_nothing() {
    let r = render("baseline", "<p id=p>x<span class=s>x</span>x</p>");
    assert!(close(r.height("p"), 20.0));
    assert!(r.drops().is_empty());
}

#[test]
fn super_raises_by_a_third_of_the_font_size_and_sub_lowers_by_a_fifth() {
    let r = render(
        "sub-super",
        "<p id=p>x<span class=s style='vertical-align:super'>x</span><span class=s style='vertical-align:sub'>x</span></p>",
    );
    assert_drops(&r, &[-20.0 / 3.0, 4.0]);
    // the lowered box sticks out below the strut (2 + 4 > 4)
    assert!(close(r.height("p"), 22.0), "{}", r.height("p"));
}

#[test]
fn lengths_and_percentages_raise() {
    let r = render(
        "length",
        "<p id=p>x<span class=s style='vertical-align:5px'>x</span><span class=s style='vertical-align:-3px'>x</span><span class=s style='vertical-align:50%'>x</span><span class=s style='vertical-align:1em'>x</span></p>",
    );
    // 50% of the line-height of the box (10px), 1em of its font
    assert_drops(&r, &[-5.0, 3.0, -5.0, -10.0]);
}

#[test]
fn a_lowered_box_makes_the_line_higher() {
    let r = render("lowered", "<p id=p>x<span style='vertical-align:-30px;font-size:10px;line-height:10px'>x</span></p>");
    // below the baseline: 2 (descent) + 30
    assert!(close(r.height("p"), 16.0 + 32.0), "{}", r.height("p"));
}

#[test]
fn middle_and_the_text_keywords_use_the_font_of_the_parent() {
    let r = render(
        "keywords",
        "<p id=p>x<span class=s style='vertical-align:middle'>x</span><span class=s style='vertical-align:text-top'>x</span><span class=s style='vertical-align:text-bottom'>x</span></p>",
    );
    // middle: the middle of the box (6/2 above the baseline) at half the x-height (8) above it
    // text-top: the top of the box (8) at the ascent of the parent (16)
    // text-bottom: the bottom of the box (2 below) at the descent of the parent (4 below)
    assert_drops(&r, &[-5.0, -8.0, 2.0]);
    assert!(close(r.height("p"), 20.0));
}

#[test]
fn top_and_bottom_align_with_the_edges_of_the_line() {
    let r = render(
        "edges",
        "<p id=p>x<span class=s style='vertical-align:top'>x</span><span class=s style='vertical-align:bottom'>x</span></p>",
    );
    // the line is 20 high with the baseline 16 from the top: the top of the box (8 above its
    // baseline) is at the top (8 up), its bottom (2 below) at the bottom (2 down)
    assert_drops(&r, &[-8.0, 2.0]);
    assert!(close(r.height("p"), 20.0));
}

#[test]
fn a_box_that_is_higher_than_the_line_stretches_it() {
    let r = render(
        "tall",
        "<p id=p>x<span style='vertical-align:top;font-size:10px;line-height:50px'>x</span></p>",
    );
    assert!(close(r.height("p"), 50.0), "{}", r.height("p"));
    let r = render(
        "tall-bottom",
        "<p id=p>x<span style='vertical-align:bottom;font-size:10px;line-height:50px'>x</span></p>",
    );
    assert!(close(r.height("p"), 50.0), "{}", r.height("p"));
}

#[test]
fn the_alignments_of_nested_boxes_add_up() {
    let r = render(
        "nested",
        "<p id=p>x<span class=s style='vertical-align:5px'>a<span style='vertical-align:3px'>x</span></span></p>",
    );
    // the text of the outer box and the one of the inner box (8 up)
    assert_drops(&r, &[-5.0, -8.0]);
}

#[test]
fn an_inline_block_is_aligned_by_its_baseline_and_its_edges() {
    let r = render(
        "inline-block",
        "<p id=p>x<span id=a style='display:inline-block;width:10px;height:10px;vertical-align:top'></span>\
         <span id=b style='display:inline-block;width:10px;height:10px;vertical-align:bottom'></span>\
         <span id=c style='display:inline-block;width:10px;height:10px;vertical-align:8px'></span></p>",
    );
    let (mut ys, mut ids) = (vec![], vec![]);
    fn collect(node: &Value, ys: &mut Vec<f64>, ids: &mut Vec<String>) {
        if let Some(id) = node["id"].as_str().filter(|id| ["a", "b", "c"].contains(id)) {
            ys.push(node["border_box"]["y"].as_f64().unwrap());
            ids.push(id.to_owned());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            collect(child, ys, ids);
        }
    }
    collect(&r.dump["document"], &mut ys, &mut ids);
    // The third box is raised by 8 and is 10 high: it needs 18 above the baseline, so the line is
    // 18 + 4 = 22 high. The first box is at the top, the second at the bottom (22 - 10), the
    // third has its bottom edge (the baseline of an empty inline-block) 8 above the baseline.
    assert_eq!(ids, ["a", "b", "c"]);
    assert!(close(ys[0], 0.0), "{:?}", ys);
    assert!(close(ys[1], 12.0), "{:?}", ys);
    assert!(close(ys[2], 18.0 - 8.0 - 10.0), "{:?}", ys);
}

#[test]
fn sub_and_sup_elements_are_raised_and_lowered() {
    let r = render("elements", "<p id=p>x<sup>x</sup><sub>x</sub></p>");
    let drops = r.drops();
    assert_eq!(drops.len(), 2, "{:?}", drops);
    assert!(drops[0] < 0.0 && drops[1] > 0.0, "{:?}", drops);
}

#[test]
fn vertical_text_raises_to_the_right() {
    // in vertical text the glyphs are turned: up is to the right
    let r = render(
        "vertical",
        "<div style='writing-mode:vertical-rl;height:200px;width:100px'>a<span class=s style='vertical-align:5px;text-decoration:underline'>b</span></div>",
    );
    let line = &r.dump["paint"]["lines"][0];
    let plain = render(
        "vertical-plain",
        "<div style='writing-mode:vertical-rl;height:200px;width:100px'>a<span class=s style='text-decoration:underline'>b</span></div>",
    );
    let plain_line = &plain.dump["paint"]["lines"][0];
    assert!(close(line["from"][0].as_f64().unwrap(), plain_line["from"][0].as_f64().unwrap() + 5.0));
    assert!(close(line["from"][1].as_f64().unwrap(), plain_line["from"][1].as_f64().unwrap()));
}

/// The y coordinate of the baseline of the text run `n` of the page.
fn text_y(r: &Rendered, n: usize) -> f64 {
    r.dump["paint"]["text"][n]["y"].as_f64().unwrap()
}

const TABLE: &str = "<style>table{border-spacing:0;font-size:10px;line-height:10px} td{padding:0;width:30px}</style>";

#[test]
fn the_content_of_a_cell_is_in_the_middle_by_default() {
    let r = render(
        "cell-default",
        &format!("{TABLE}<table><tr><td>x</td><td style='height:50px'>x</td></tr></table>"),
    );
    // the first cell is 50 high, its line 10 (baseline at 8): centered, 20 down
    assert!(close(text_y(&r, 0), 8.0 + 20.0), "{}", text_y(&r, 0));
    assert!(close(text_y(&r, 1), 8.0 + 20.0), "{}", text_y(&r, 1));
}

#[test]
fn cells_can_be_aligned_to_the_top_and_the_bottom() {
    let r = render(
        "cell-edges",
        &format!(
            "{TABLE}<table><tr><td style='vertical-align:top'>x</td><td style='vertical-align:bottom'>x</td>\
             <td style='height:50px'></td></tr></table>"
        ),
    );
    assert!(close(text_y(&r, 0), 8.0), "{}", text_y(&r, 0));
    assert!(close(text_y(&r, 1), 8.0 + 40.0), "{}", text_y(&r, 1));
}

#[test]
fn a_row_or_row_group_gives_its_alignment_to_the_cells() {
    let r = render(
        "cell-inherit",
        &format!(
            "{TABLE}<table><tbody style='vertical-align:bottom'><tr><td>x</td><td style='height:50px'></td></tr>\
             <tr style='vertical-align:top'><td style='height:30px'>x</td></tr></tbody></table>"
        ),
    );
    assert!(close(text_y(&r, 0), 8.0 + 40.0), "{}", text_y(&r, 0));
    // the second row is at 50
    assert!(close(text_y(&r, 1), 50.0 + 8.0), "{}", text_y(&r, 1));
}

#[test]
fn blocks_in_a_cell_move_together() {
    let r = render(
        "cell-blocks",
        &format!(
            "{TABLE}<table><tr><td><div id=a>x</div><div id=b>x</div></td><td style='height:60px'></td></tr></table>"
        ),
    );
    // 20 of content in 60: centered, 20 down
    assert!(close(r.top("a"), 20.0), "{}", r.top("a"));
    assert!(close(r.top("b"), 30.0), "{}", r.top("b"));
}

impl Rendered {
    fn top(&self, id: &str) -> f64 {
        fn find(node: &Value, id: &str) -> Option<f64> {
            if node["id"] == id {
                return node["border_box"]["y"].as_f64();
            }
            node["children"].as_array()?.iter().find_map(|child| find(child, id))
        }
        find(&self.dump["document"], id).unwrap_or_else(|| panic!("no element #{}", id))
    }
}
