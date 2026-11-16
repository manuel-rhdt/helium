//! CSS multi-column layout: a container that is not split over pages and whose content is block
//! flow, with balanced columns.
//!
//! https://www.w3.org/TR/css-multicol-1/
//!
//! The pages are 400px by 600px without margins. The container is a `div` with the id `m` that
//! is 400px wide unless a test says otherwise, and the items are `div`s of 20px height with the
//! ids `i1`, `i2` and so on. All lengths are CSS px. See `docs/pagination.md`.

mod common;

use serde_json::Value;

/// One laid out box: the tag, the id, the page and the border box (x, y, width, height in px).
#[derive(Debug)]
struct Item {
    tag: String,
    id: String,
    page: u64,
    rect: [f64; 4],
}

/// A filled rectangle: the page and the box in px.
#[derive(Debug)]
struct Rect {
    page: u64,
    rect: [f64; 4],
    color: [u64; 3],
}

struct Layout {
    items: Vec<Item>,
    rects: Vec<Rect>,
    /// What the program logged at the level `warn`.
    log: String,
}

fn layout(name: &str, html: &str) -> Layout {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("multicol__{}.html", name));
    let output = dir.join(format!("multicol__{}.pdf", name));
    let dump = dir.join(format!("multicol__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(&dump)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{}", log);
    // what the paginator counts for a page is what the page has (test 15, for every test)
    assert!(
        !log.contains("high, but the paginator counted"),
        "the log has a warning about the height of a page:\n{log}"
    );
    let value: Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    fn walk(node: &Value, items: &mut Vec<Item>) {
        if node["type"] == "element" && node["border_box"].is_object() {
            let b = &node["border_box"];
            let get = |key: &str| b[key].as_f64().unwrap();
            items.push(Item {
                tag: node["tag"].as_str().unwrap().to_owned(),
                id: node["id"].as_str().unwrap_or("").to_owned(),
                page: node["page"].as_u64().unwrap_or(0),
                rect: [get("x"), get("y"), get("width"), get("height")],
            });
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, items);
        }
    }
    let mut items = vec![];
    walk(&value["document"], &mut items);
    let rects = value["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rect| {
            let get = |key: &str| rect[key].as_f64().unwrap();
            let color = |index: usize| rect["color"][index].as_u64().unwrap();
            Rect {
                page: rect["page"].as_u64().unwrap(),
                rect: [get("x"), get("y"), get("width"), get("height")],
                color: [color(0), color(1), color(2)],
            }
        })
        .collect();
    Layout { items, rects, log }
}

impl Layout {
    /// The box with the id.
    fn get(&self, id: &str) -> &Item {
        let found: Vec<&Item> = self.items.iter().filter(|item| item.id == id).collect();
        assert_eq!(found.len(), 1, "there is not exactly one box with the id {id}: {:?}", self.items);
        found[0]
    }

    /// The rectangles of a colour, by page, then top to bottom and left to right.
    fn filled(&self, color: [u64; 3]) -> Vec<&Rect> {
        let mut rects: Vec<&Rect> = self.rects.iter().filter(|rect| rect.color == color).collect();
        rects.sort_by(|a, b| {
            (a.page, a.rect[1], a.rect[0]).partial_cmp(&(b.page, b.rect[1], b.rect[0])).unwrap()
        });
        rects
    }
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 0.02,
        "{what} is {actual}, expected {expected}"
    );
}

/// Asserts the left edge, the top edge and the width of a box.
fn assert_at(layout: &Layout, id: &str, x: f64, y: f64, width: f64) {
    let item = layout.get(id);
    let [ax, ay, aw, _] = item.rect;
    assert_close(ax, x, &format!("x of {id}"));
    assert_close(ay, y, &format!("y of {id}"));
    assert_close(aw, width, &format!("width of {id}"));
}

fn assert_height(layout: &Layout, id: &str, height: f64) {
    assert_close(layout.get(id).rect[3], height, &format!("height of {id}"));
}

/// A document with the pages of the tests.
fn doc(style: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html><style>@page {{ width: 400px; height: 600px; margin: 0 }}
         body {{ margin: 0 }} {style}</style><body>{body}</body>"
    )
}

/// `count` items of 20px, with the ids `i{first}` and so on.
fn items(first: usize, count: usize) -> String {
    (first..first + count)
        .map(|number| format!("<div class=i id=i{number} style=\"height:20px\"></div>"))
        .collect()
}

/// A container with the style, and `count` items.
fn container(style: &str, count: usize) -> String {
    format!("<div id=m style=\"{style}\">{}</div>", items(1, count))
}

#[test]
fn the_count_gives_the_columns_their_width() {
    // (400 - 2 * 10) / 3 = 126.667 for the width of a column
    let layout = layout("count", &doc("", &container("column-count: 3; column-gap: 10px", 6)));
    let width = 380.0 / 3.0;
    assert_height(&layout, "m", 40.0);
    for (id, column, row) in [("i1", 0, 0), ("i2", 0, 1), ("i3", 1, 0), ("i4", 1, 1), ("i5", 2, 0), ("i6", 2, 1)] {
        assert_at(&layout, id, column as f64 * (width + 10.0), row as f64 * 20.0, width);
    }
    assert_at(&layout, "m", 0.0, 0.0, 400.0);
}

#[test]
fn the_width_gives_the_number_of_columns() {
    // floor((400 + 20) / (100 + 20)) = 3 columns of (400 - 2 * 20) / 3 = 120
    let layout = layout("width", &doc("", &container("column-width: 100px; column-gap: 20px", 3)));
    assert_at(&layout, "i1", 0.0, 0.0, 120.0);
    assert_at(&layout, "i2", 140.0, 0.0, 120.0);
    assert_at(&layout, "i3", 280.0, 0.0, 120.0);
    assert_height(&layout, "m", 20.0);
}

#[test]
fn the_count_is_at_most_what_fits_in_the_width() {
    // min(5, floor(400 / 150)) = 2 columns of 200
    let layout = layout(
        "both",
        &doc("", &container("column-count: 5; column-width: 150px; column-gap: 0", 4)),
    );
    assert_at(&layout, "i1", 0.0, 0.0, 200.0);
    assert_at(&layout, "i2", 0.0, 20.0, 200.0);
    assert_at(&layout, "i3", 200.0, 0.0, 200.0);
    assert_at(&layout, "i4", 200.0, 20.0, 200.0);
    assert_height(&layout, "m", 40.0);
}

#[test]
fn the_shorthand_sets_the_count_and_the_width() {
    // The gap is `normal`, which is 1em = 16px: min(2, floor((400 + 16) / (100 + 16))) = 2
    // columns of (400 - 16) / 2 = 192.
    let shorthand = layout("shorthand", &doc("", &container("columns: 2 100px", 4)));
    assert_at(&shorthand, "i1", 0.0, 0.0, 192.0);
    assert_at(&shorthand, "i3", 208.0, 0.0, 192.0);
    let longhands = layout("longhands", &doc("", &container("column-count: 2; column-width: 100px", 4)));
    for id in ["m", "i1", "i2", "i3", "i4"] {
        assert_eq!(shorthand.get(id).rect, longhands.get(id).rect, "{id}");
    }
    // one of the two values is enough, in either order
    let count = layout("shorthand_count", &doc("", &container("columns: 3; column-gap: 0", 3)));
    assert_at(&count, "i2", 400.0 / 3.0, 0.0, 400.0 / 3.0);
    let width = layout("shorthand_width", &doc("", &container("columns: 100px; column-gap: 0", 4)));
    assert_at(&width, "i4", 300.0, 0.0, 100.0);
    let reversed = layout("shorthand_reversed", &doc("", &container("columns: 100px 4; column-gap: 0", 4)));
    assert_at(&reversed, "i4", 300.0, 0.0, 100.0);
}

#[test]
fn auto_columns_are_an_ordinary_block() {
    for (name, style) in [("auto", "columns: auto"), ("auto_both", "column-count: auto; column-width: auto")] {
        let layout = layout(name, &doc("", &container(style, 3)));
        assert_height(&layout, "m", 60.0);
        for (index, id) in ["i1", "i2", "i3"].into_iter().enumerate() {
            assert_at(&layout, id, 0.0, index as f64 * 20.0, 400.0);
        }
    }
}

#[test]
fn a_gap_that_is_normal_is_one_em_in_columns_and_nothing_in_a_grid() {
    let columns = layout(
        "gap_normal",
        &doc("", &container("column-count: 2; column-gap: normal; font-size: 20px", 2)),
    );
    // (400 - 20) / 2 = 190
    assert_at(&columns, "i1", 0.0, 0.0, 190.0);
    assert_at(&columns, "i2", 210.0, 0.0, 190.0);
    // not set at all is `normal`, too
    let unset = layout("gap_unset", &doc("", &container("column-count: 2; font-size: 20px", 2)));
    assert_at(&unset, "i2", 210.0, 0.0, 190.0);
    // a grid keeps a `normal` gap of 0
    let grid = layout(
        "gap_grid",
        &doc("", &container("display: grid; grid-template-columns: 1fr 1fr; column-gap: normal; font-size: 20px", 2)),
    );
    assert_at(&grid, "i1", 0.0, 0.0, 200.0);
    assert_at(&grid, "i2", 200.0, 0.0, 200.0);
}

#[test]
fn the_columns_are_balanced() {
    // five items in two columns: three and two, so the container is 60px high
    let layout = layout("balance", &doc("", &container("column-count: 2; column-gap: 0", 5)));
    assert_height(&layout, "m", 60.0);
    for (id, x, y) in [("i1", 0.0, 0.0), ("i2", 0.0, 20.0), ("i3", 0.0, 40.0), ("i4", 200.0, 0.0), ("i5", 200.0, 20.0)] {
        assert_at(&layout, id, x, y, 200.0);
    }
}

#[test]
fn a_box_that_avoids_a_break_is_not_split_if_the_columns_are_as_high_without_it() {
    // A (20), B (20), C (40, `break-inside: avoid`), D (20): the least height at which the
    // content fits is 60, with [A, B] and [C, D].
    let layout = layout(
        "avoid",
        &doc(
            "",
            "<div id=m style=\"column-count: 2; column-gap: 0\">
             <div id=a style=\"height:20px\"></div><div id=b style=\"height:20px\"></div>
             <div id=c style=\"height:40px; break-inside: avoid\"></div><div id=d style=\"height:20px\"></div></div>",
        ),
    );
    assert_height(&layout, "m", 60.0);
    assert_at(&layout, "a", 0.0, 0.0, 200.0);
    assert_at(&layout, "b", 0.0, 20.0, 200.0);
    assert_at(&layout, "c", 200.0, 0.0, 200.0);
    assert_at(&layout, "d", 200.0, 40.0, 200.0);
}

#[test]
fn a_box_with_content_that_avoids_a_break_inside_stays_in_one_column() {
    // Without `break-inside: avoid-column` the balanced columns would cut C between its two items:
    // [A, B, c1] and [c2, D], 60px high. The columns are 60px high with the avoid, too, but
    // C stays whole: [A, B] and [C, D].
    let body = |style: &str| {
        format!(
            "<div id=m style=\"column-count: 2; column-gap: 0\">
             <div id=a style=\"height:20px\"></div><div id=b style=\"height:20px\"></div>
             <div id=c style=\"{style}\"><div id=c1 style=\"height:20px\"></div><div id=c2 style=\"height:20px\"></div></div>
             <div id=d style=\"height:20px\"></div></div>"
        )
    };
    let split = layout("avoid_content_off", &doc("", &body("")));
    assert_height(&split, "m", 60.0);
    assert_at(&split, "c1", 0.0, 40.0, 200.0);
    assert_at(&split, "c2", 200.0, 0.0, 200.0);
    let whole = layout("avoid_content", &doc("", &body("break-inside: avoid-column")));
    assert_height(&whole, "m", 60.0);
    assert_at(&whole, "b", 0.0, 20.0, 200.0);
    assert_at(&whole, "c1", 200.0, 0.0, 200.0);
    assert_at(&whole, "c2", 200.0, 20.0, 200.0);
    assert_at(&whole, "d", 200.0, 40.0, 200.0);
}

#[test]
fn a_forced_break_ends_a_column() {
    // the third of four items starts the second column
    let layout_a = layout(
        "forced",
        &doc("#i3 { break-before: column }", &container("column-count: 2; column-gap: 0", 4)),
    );
    assert_height(&layout_a, "m", 40.0);
    assert_at(&layout_a, "i2", 0.0, 20.0, 200.0);
    assert_at(&layout_a, "i3", 200.0, 0.0, 200.0);
    assert_at(&layout_a, "i4", 200.0, 20.0, 200.0);

    // A forced break takes away the freedom to balance: [1] and [2, 3, 4, 5], 80px high.
    let layout_b = layout(
        "forced_unbalanced",
        &doc("#i2 { break-before: column }", &container("column-count: 2; column-gap: 0", 5)),
    );
    assert_height(&layout_b, "m", 80.0);
    assert_at(&layout_b, "i1", 0.0, 0.0, 200.0);
    assert_at(&layout_b, "i2", 200.0, 0.0, 200.0);
    assert_at(&layout_b, "i5", 200.0, 60.0, 200.0);

    // `break-after` on the first item is the same break
    let layout_c = layout(
        "forced_after",
        &doc("#i1 { break-after: column }", &container("column-count: 2; column-gap: 0", 5)),
    );
    assert_height(&layout_c, "m", 80.0);
    assert_at(&layout_c, "i2", 200.0, 0.0, 200.0);
}

#[test]
fn a_break_that_is_to_be_avoided_is_not_taken_if_another_is_possible() {
    // `break-before: avoid-column` on the fourth of four items avoids [1, 2, 3] and [4]. The
    // columns [1, 2] and [3, 4] are the balanced ones anyway, so nothing changes.
    let layout_a = layout(
        "avoid_before_fourth",
        &doc("#i4 { break-before: avoid-column }", &container("column-count: 2; column-gap: 0", 4)),
    );
    assert_height(&layout_a, "m", 40.0);
    assert_at(&layout_a, "i2", 0.0, 20.0, 200.0);
    assert_at(&layout_a, "i3", 200.0, 0.0, 200.0);
    // On the third item it avoids [1, 2] and [3, 4]. [1] and [2, 3, 4] and [1, 2, 3] and [4] are
    // both 60px high; of two ways that are as high, the first column is the fuller one.
    let layout_b = layout(
        "avoid_before_third",
        &doc("#i3 { break-before: avoid-column }", &container("column-count: 2; column-gap: 0", 4)),
    );
    assert_height(&layout_b, "m", 60.0);
    assert_at(&layout_b, "i3", 0.0, 40.0, 200.0);
    assert_at(&layout_b, "i4", 200.0, 0.0, 200.0);
    // `avoid-page` is not about columns
    let layout_c = layout(
        "avoid_page",
        &doc("#i3 { break-before: avoid-page }", &container("column-count: 2; column-gap: 0", 4)),
    );
    assert_height(&layout_c, "m", 40.0);
    assert_at(&layout_c, "i3", 200.0, 0.0, 200.0);
}

#[test]
fn breaks_of_a_level_that_does_not_apply_are_ignored() {
    // A forced page break in a container is a column break, too, and it ends the fragment: the
    // container continues on the next page, with its content balanced there
    // (https://www.w3.org/TR/css-break-3/#forced-breaks).
    let page = layout(
        "page_break_in_columns",
        &doc("#i2 { break-before: page }", &container("column-count: 2; column-gap: 0", 4)),
    );
    assert_eq!(page.get("i1").page, 0);
    assert_height(&page, "m", 20.0);
    assert_eq!(page.get("i2").page, 1);
    assert_at(&page, "i2", 0.0, 0.0, 200.0);
    assert_at(&page, "i3", 0.0, 20.0, 200.0);
    assert_at(&page, "i4", 200.0, 0.0, 200.0);
    // a forced column break outside of a container with columns is like `auto`
    let outside = layout(
        "column_break_outside",
        &doc("#i2 { break-before: column } #i3 { break-after: column }", &items(1, 4)),
    );
    for id in ["i1", "i2", "i3", "i4"] {
        assert_eq!(outside.get(id).page, 0, "{id}");
    }
    assert_at(&outside, "i4", 0.0, 60.0, 400.0);
}

/// Text that fills about 20 lines in a column of 190px.
fn words() -> String {
    "the quick brown fox jumps over the lazy dog ".repeat(14)
}

#[test]
fn the_lines_of_a_paragraph_are_broken_for_the_width_of_a_column() {
    let style = "p { margin: 0; line-height: 20px; font-size: 16px; orphans: 1; widows: 1;
                 background: rgb(0, 128, 0) }";
    let text = words();
    let columns = layout(
        "text",
        &doc(
            style,
            &format!("<div id=m style=\"column-count: 2; column-gap: 20px\"><p id=p>{text}</p></div>"),
        ),
    );
    // the same paragraph in a block as wide as a column, and in one as wide as the page
    let narrow = layout("text_190", &doc(style, &format!("<div style=\"width: 190px\"><p id=p>{text}</p></div>")));
    let wide = layout("text_400", &doc(style, &format!("<div><p id=p>{text}</p></div>")));
    let lines_190 = narrow.get("p").rect[3] / 20.0;
    let lines_400 = wide.get("p").rect[3] / 20.0;
    assert!(lines_190 >= 16.0 && lines_190 != lines_400, "{lines_190} lines for 190px, {lines_400} for 400px");

    let green = columns.filled([0, 128, 0]);
    assert_eq!(green.len(), 2, "one background per column: {green:?}");
    assert_close(green[0].rect[0], 0.0, "x of the first column");
    assert_close(green[1].rect[0], 210.0, "x of the second column");
    for rect in &green {
        assert_close(rect.rect[2], 190.0, "width of a column");
    }
    assert_close(green[0].rect[1], green[1].rect[1], "top of the second column");
    assert!((green[0].rect[3] - green[1].rect[3]).abs() <= 20.0 + 0.01, "{green:?}");
    // the lines of both columns are the lines of the paragraph at 190px
    assert_close((green[0].rect[3] + green[1].rect[3]) / 20.0, lines_190, "lines in both columns");
    // balanced: the first column has the half, rounded up
    assert_close(green[0].rect[3] / 20.0, (lines_190 / 2.0).ceil(), "lines in the first column");
    assert_height(&columns, "m", green[0].rect[3].max(green[1].rect[3]));
}

#[test]
fn a_rule_is_drawn_in_the_middle_of_the_gap() {
    let red = [255, 0, 0];
    let layout_a = layout(
        "rule",
        &doc("", &container("column-count: 2; column-gap: 20px; column-rule: 2px solid rgb(255, 0, 0)", 4)),
    );
    let rules = layout_a.filled(red);
    assert_eq!(rules.len(), 1, "{rules:?}");
    // the first column ends at 190, the gap is from 190 to 210
    assert_close(rules[0].rect[0], 199.0, "x of the rule");
    assert_close(rules[0].rect[1], 0.0, "top of the rule");
    assert_close(rules[0].rect[2], 2.0, "width of the rule");
    assert_close(rules[0].rect[3], 40.0, "height of the rule");
    assert_eq!(rules[0].page, 0);

    // the longhands
    let longhands = layout(
        "rule_longhands",
        &doc(
            "",
            &container(
                "column-count: 2; column-gap: 20px; column-rule-width: 2px; column-rule-style: solid;
                 column-rule-color: rgb(255, 0, 0)",
                4,
            ),
        ),
    );
    assert_eq!(longhands.filled(red).len(), 1);
    assert_close(longhands.filled(red)[0].rect[0], 199.0, "x of the rule of the longhands");

    // three columns of (400 - 2 * 20) / 3 = 120: two rules, in the gaps from 120 and 260
    let layout_b = layout(
        "rule_three",
        &doc("", &container("column-count: 3; column-gap: 20px; column-rule: 2px solid rgb(255, 0, 0)", 6)),
    );
    let rules = layout_b.filled(red);
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert_close(rules[0].rect[0], 129.0, "x of the first rule");
    assert_close(rules[1].rect[0], 269.0, "x of the second rule");
    for rule in &rules {
        assert_close(rule.rect[3], 40.0, "height of a rule");
    }

    // no style, no rule
    let none = layout(
        "rule_none",
        &doc(
            "",
            &container("column-count: 2; column-gap: 20px; column-rule: 2px solid rgb(255, 0, 0); column-rule-style: none", 4),
        ),
    );
    assert!(none.filled(red).is_empty());

    // a gap between two columns that have content only: two items fill two of three columns
    let empty = layout(
        "rule_empty_column",
        &doc("", &container("column-count: 3; column-gap: 20px; column-rule: 2px solid rgb(255, 0, 0)", 2)),
    );
    let rules = empty.filled(red);
    assert_eq!(rules.len(), 1, "{rules:?}");
    assert_close(rules[0].rect[0], 129.0, "x of the rule");
}

#[test]
fn the_content_after_the_container_follows_it_like_a_block() {
    let layout_a = layout(
        "around",
        &doc(
            "p { margin: 0; height: 20px }",
            &format!(
                "<p id=before></p>{}<p id=after style=\"margin-top: 10px\"></p>",
                container("column-count: 2; column-gap: 0", 4)
            ),
        ),
    );
    assert_at(&layout_a, "before", 0.0, 0.0, 400.0);
    assert_at(&layout_a, "m", 0.0, 20.0, 400.0);
    assert_height(&layout_a, "m", 40.0);
    // 20 + 40 + 10
    assert_at(&layout_a, "after", 0.0, 70.0, 400.0);

    // the padding and the border are around the columns
    let layout_b = layout(
        "edges",
        &doc(
            "",
            &container(
                "box-sizing: border-box; padding: 10px; border: 5px solid black; column-count: 2; column-gap: 10px",
                4,
            ),
        ),
    );
    assert_at(&layout_b, "m", 0.0, 0.0, 400.0);
    // 400 - 2 * 15 = 370, (370 - 10) / 2 = 180
    assert_at(&layout_b, "i1", 15.0, 15.0, 180.0);
    assert_at(&layout_b, "i2", 15.0, 35.0, 180.0);
    assert_at(&layout_b, "i3", 205.0, 15.0, 180.0);
    assert_height(&layout_b, "m", 70.0);
}

#[test]
fn a_container_that_does_not_fit_in_the_rest_of_the_page_moves_to_the_next_page() {
    // 500px of content, then a container of 200px (20 items in two columns), where 100px are left
    let layout_a = layout(
        "pages",
        &doc(
            "",
            &format!(
                "<div id=before style=\"height: 500px\"></div>{}",
                container("column-count: 2; column-gap: 0; break-inside: avoid", 20)
            ),
        ),
    );
    assert_height(&layout_a, "m", 200.0);
    assert_eq!(layout_a.get("before").page, 0);
    assert_eq!(layout_a.get("m").page, 1);
    assert_at(&layout_a, "m", 0.0, 0.0, 400.0);
    assert_eq!(layout_a.get("i1").page, 1);
    assert_eq!(layout_a.get("i20").page, 1);
    assert_at(&layout_a, "i20", 200.0, 180.0, 200.0);

    // a container that fits is on the page
    let layout_b = layout(
        "pages_fit",
        &doc(
            "",
            &format!(
                "<div id=before style=\"height: 300px\"></div>{}",
                container("column-count: 2; column-gap: 0", 20)
            ),
        ),
    );
    assert_eq!(layout_b.get("m").page, 0);
    assert_at(&layout_b, "m", 0.0, 300.0, 400.0);

    // a container that is taller than a page is split over pages: the first fragment is as high
    // as the page, and the content after it is not lost
    let layout_c = layout(
        "pages_tall",
        &doc(
            "",
            &format!(
                "{}<div id=after style=\"height: 20px\"></div>",
                container("column-count: 2; column-gap: 0", 70)
            ),
        ),
    );
    assert_height(&layout_c, "m", 600.0);
    assert_eq!(layout_c.get("m").page, 0);
    assert!(layout_c.get("after").page >= 1);
    assert_eq!(layout_c.get("i70").page, 1);
}

#[test]
fn one_column_is_a_container_with_columns() {
    let layout = layout("one", &doc("", &container("column-count: 1; column-gap: 10px", 3)));
    assert_at(&layout, "m", 0.0, 0.0, 400.0);
    assert_at(&layout, "i1", 0.0, 0.0, 400.0);
    assert_at(&layout, "i3", 0.0, 40.0, 400.0);
    assert_height(&layout, "m", 60.0);
}

#[test]
fn a_container_in_a_column_does_not_panic() {
    let inner = format!("<div id=inner style=\"column-count: 2; column-gap: 0\">{}</div>", items(1, 6));
    let layout = layout(
        "nested",
        &doc(
            "",
            &format!(
                "<div id=m style=\"column-count: 2; column-gap: 0\">
                 <div id=x style=\"height: 20px\"></div>{inner}
                 <div id=y style=\"height: 20px\"></div><div id=z style=\"height: 40px\"></div></div>"
            ),
        ),
    );
    for id in ["m", "inner", "x", "y", "z", "i1", "i6"] {
        layout.get(id);
    }
    // the inner container is as wide as a column
    assert_close(layout.get("inner").rect[2], 200.0, "width of the inner container");
    assert_close(layout.get("i1").rect[2], 100.0, "width of an item of the inner container");
}

#[test]
fn other_properties_of_columns_are_accepted() {
    // `column-fill` is parsed, `column-span` is parsed and ignored: neither changes the layout
    // of the container, and nothing is reported
    let plain = layout("plain", &doc("", &container("column-count: 2; column-gap: 0", 4)));
    let fill = layout(
        "fill",
        &doc("", &container("column-count: 2; column-gap: 0; column-fill: balance", 4)),
    );
    let span = layout(
        "span",
        &doc("#i2 { column-span: all }", &container("column-count: 2; column-gap: 0", 4)),
    );
    for other in [&fill, &span] {
        for id in ["m", "i1", "i2", "i3", "i4"] {
            assert_eq!(plain.get(id).rect, other.get(id).rect, "{id}");
        }
        assert!(!other.log.contains("column"), "{}", other.log);
    }
    let auto = layout("fill_auto", &doc("", &container("column-count: 2; column-gap: 0; column-fill: auto", 4)));
    assert!(!auto.log.contains("column"), "{}", auto.log);
}

#[test]
fn a_definite_height_keeps_the_height_of_the_container() {
    // the content is balanced within the height: 6 items in 2 columns are 3 and 3 (60px) in a
    // container of 100px
    let layout = layout("definite", &doc("", &container("column-count: 2; column-gap: 0; height: 100px", 6)));
    assert_height(&layout, "m", 100.0);
    assert_at(&layout, "i3", 0.0, 40.0, 200.0);
    assert_at(&layout, "i4", 200.0, 0.0, 200.0);
}

#[test]
fn content_that_does_not_fit_in_the_columns_overflows_below_the_last_one() {
    // 10 items need 100px in two columns, but the container is 60px high: the first column has
    // 3 items and the last one takes the rest, below the container
    let layout = layout("overflow", &doc("", &container("column-count: 2; column-gap: 0; height: 60px", 10)));
    assert_height(&layout, "m", 60.0);
    assert_at(&layout, "i3", 0.0, 40.0, 200.0);
    assert_at(&layout, "i4", 200.0, 0.0, 200.0);
    assert_at(&layout, "i10", 200.0, 120.0, 200.0);
}

#[test]
fn a_link_in_a_column_is_where_the_text_is() {
    // the clickable area of the link is in the second column
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("multicol__link.html");
    let output = dir.join("multicol__link.pdf");
    std::fs::write(
        &input,
        doc(
            "",
            "<div style=\"column-count: 2; column-gap: 0\">
             <div style=\"height: 20px; break-after: column\">first</div><div style=\"height: 20px\"><a href=\"https://example.com/\">link</a></div></div>",
        ),
    )
    .unwrap();
    let out = common::helium().unwrap().args(["--no-system-fonts"]).arg(&input).arg(&output).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pdf = common::expand_object_streams(&std::fs::read(&output).unwrap());
    let text = String::from_utf8_lossy(&pdf);
    let link = text.find("/Subtype /Link").expect("the PDF has a link annotation");
    // (the keys of the dictionary are in no particular order)
    let window = &text[link.saturating_sub(400)..(link + 400).min(text.len())];
    let start = window.find("/Rect").expect("the annotation has a rectangle");
    let rect: Vec<f64> = window[start..]
        .split(['[', ']'])
        .nth(1)
        .unwrap()
        .split_whitespace()
        .map(|number| number.parse().unwrap())
        .collect();
    // in points, from the left edge of the page: the second column starts at 200px = 150pt
    assert!(rect[0] >= 150.0 - 0.5 && rect[2] <= 300.0 + 0.5, "{rect:?}");
}

// Cases that go beyond the list above.

#[test]
fn a_container_with_one_paragraph_of_text_is_split_into_columns() {
    let style = "#m { line-height: 20px; font-size: 16px; orphans: 1; widows: 1; background: rgb(0, 128, 0) }
                 #r { line-height: 20px; font-size: 16px; width: 190px }";
    let text = words();
    let columns = layout(
        "inline",
        &doc(style, &format!("<div id=m style=\"column-count: 2; column-gap: 20px\">{text}</div>")),
    );
    let narrow = layout("inline_190", &doc(style, &format!("<div id=r>{text}</div>")));
    let lines = narrow.get("r").rect[3] / 20.0;
    // the container is as high as the first column, which has the half of the lines, rounded up
    assert_height(&columns, "m", (lines / 2.0).ceil() * 20.0);
    // and it is one box: one background
    assert_eq!(columns.filled([0, 128, 0]).len(), 1);
}

#[test]
fn a_container_with_padding_and_border_that_has_text_is_as_high_as_its_columns_and_edges() {
    let style = "#m { line-height: 20px; font-size: 16px; orphans: 1; widows: 1; padding: 5px; border: 3px solid black }
                 #r { line-height: 20px; font-size: 16px; width: 186px }";
    let text = words();
    // 400 - 16 = 384 for the columns, (384 - 12) / 2 = 186 for a column
    let columns = layout(
        "inline_edges",
        &doc(style, &format!("<div id=m style=\"column-count: 2; column-gap: 12px\">{text}</div>")),
    );
    let narrow = layout("inline_edges_186", &doc(style, &format!("<div id=r>{text}</div>")));
    let lines = narrow.get("r").rect[3] / 20.0;
    assert_height(&columns, "m", (lines / 2.0).ceil() * 20.0 + 16.0);
}

#[test]
fn a_container_in_a_flex_item_and_in_a_wrapper_has_its_columns_with_others_around_it() {
    // Several containers: the boxes around them are laid out again, so the columns are, too
    let first = "<div id=m0 style=\"column-count: 2; column-gap: 0\"><div class=i></div></div>";
    let flex = "<div style=\"display: flex\" id=fl><div id=flexitem style=\"column-count: 2; column-gap: 0; width: 200px\">
                <div class=i id=f1></div><div class=i id=f2></div><div class=i id=f3></div></div>
                <div class=i id=other style=\"width: 100px\"></div></div>";
    let last = "<div id=wrap><p id=h style=\"height: 20px; margin: 0\"></p>
                <div id=m3 style=\"column-count: 2; column-gap: 10px\"><div class=i id=r1></div><div class=i id=r2></div></div></div>
                <p style=\"height: 20px; margin: 0\"></p>";
    let layout = layout("around_flex", &doc(".i { height: 20px }", &format!("{first}{flex}{last}")));
    assert_at(&layout, "f1", 0.0, 20.0, 100.0);
    assert_at(&layout, "f2", 0.0, 40.0, 100.0);
    assert_at(&layout, "f3", 100.0, 20.0, 100.0);
    assert_height(&layout, "flexitem", 40.0);
    assert_at(&layout, "other", 200.0, 20.0, 100.0);
    assert_at(&layout, "r1", 0.0, 80.0, 195.0);
    assert_at(&layout, "r2", 205.0, 80.0, 195.0);
}

#[test]
fn a_container_in_a_column_has_columns_of_its_own() {
    // [x (20), inner (40)] and [y (20)]: the columns are 60 and 20 high. [x] and [inner, y] would
    // be 20 and 60: the first column is the fuller one.
    let layout = layout(
        "nested_positions",
        &doc(
            "",
            "<div id=m style=\"column-count: 2; column-gap: 0\">
             <div id=x style=\"height: 20px\"></div>
             <div id=inner style=\"column-count: 2; column-gap: 0\">
             <div id=n1 style=\"height: 20px\"></div><div id=n2 style=\"height: 20px\"></div>
             <div id=n3 style=\"height: 20px\"></div><div id=n4 style=\"height: 20px\"></div></div>
             <div id=y style=\"height: 20px\"></div></div>",
        ),
    );
    assert_height(&layout, "m", 60.0);
    assert_at(&layout, "x", 0.0, 0.0, 200.0);
    assert_at(&layout, "inner", 0.0, 20.0, 200.0);
    assert_height(&layout, "inner", 40.0);
    assert_at(&layout, "n1", 0.0, 20.0, 100.0);
    assert_at(&layout, "n2", 0.0, 40.0, 100.0);
    assert_at(&layout, "n3", 100.0, 20.0, 100.0);
    assert_at(&layout, "n4", 100.0, 40.0, 100.0);
    assert_at(&layout, "y", 200.0, 0.0, 200.0);
}

#[test]
fn a_table_in_a_column_is_one_box() {
    let layout = layout(
        "table",
        &doc(
            "table { border-spacing: 0 } td { padding: 0; height: 20px }",
            "<div id=m style=\"column-count: 2; column-gap: 0\"><div id=a style=\"height: 20px\"></div>
             <table id=t style=\"width: 100%\"><tr><td id=c1></td></tr><tr><td id=c2></td></tr></table>
             <div id=b style=\"height: 20px\"></div></div>",
        ),
    );
    // [a, table] and [b]: the table is not cut between its rows
    assert_at(&layout, "a", 0.0, 0.0, 200.0);
    assert_at(&layout, "t", 0.0, 20.0, 200.0);
    assert_at(&layout, "c2", 0.0, 40.0, 200.0);
    assert_at(&layout, "b", 200.0, 0.0, 200.0);
}

#[test]
fn the_first_column_of_a_container_from_right_to_left_is_at_the_right() {
    let layout = layout(
        "rtl",
        &doc(
            "",
            &container("column-count: 2; column-gap: 20px; direction: rtl; column-rule: 2px solid rgb(255, 0, 0)", 4),
        ),
    );
    assert_at(&layout, "i1", 210.0, 0.0, 190.0);
    assert_at(&layout, "i2", 210.0, 20.0, 190.0);
    assert_at(&layout, "i3", 0.0, 0.0, 190.0);
    assert_at(&layout, "i4", 0.0, 20.0, 190.0);
    assert_close(layout.filled([255, 0, 0])[0].rect[0], 199.0, "x of the rule");
}

#[test]
fn a_vertical_flow_has_no_columns_and_does_not_panic() {
    let layout = layout(
        "vertical",
        &doc("", "<div id=m style=\"column-count: 2; writing-mode: vertical-rl; height: 100px\"><div class=i id=v1 style=\"width: 20px\"></div></div>"),
    );
    layout.get("m");
    layout.get("v1");
}

#[test]
fn the_colour_and_the_gap_can_be_relative() {
    // `currentcolor` is the text colour, and a percentage is of the width: 10% of 400 = 40
    let layout = layout(
        "relative",
        &doc(
            "",
            &container(
                "column-count: 2; column-gap: 10%; color: rgb(0, 0, 255); column-rule: 4px solid currentcolor",
                4,
            ),
        ),
    );
    assert_at(&layout, "i1", 0.0, 0.0, 180.0);
    assert_at(&layout, "i3", 220.0, 0.0, 180.0);
    let rules = layout.filled([0, 0, 255]);
    assert_eq!(rules.len(), 1);
    assert_close(rules[0].rect[0], 198.0, "x of the rule");
    assert_close(rules[0].rect[2], 4.0, "width of the rule");
}

#[test]
fn margins_at_the_top_of_a_column_go_and_the_ones_in_the_middle_stay() {
    // Four items of 20px with a margin of 10px above and below. The first column keeps the margin at
    // the start of the container: 10 + 20 + 10 + 20 = 60 (the margin at its end goes). The second
    // starts with its box: 20 + 10 + 20 = 50.
    let layout = layout(
        "margins",
        &doc(
            ".i { margin: 10px 0 }",
            "<div id=m style=\"column-count: 2; column-gap: 0\">
             <div class=i id=i1 style=\"height: 20px\"></div><div class=i id=i2 style=\"height: 20px\"></div>
             <div class=i id=i3 style=\"height: 20px\"></div><div class=i id=i4 style=\"height: 20px\"></div></div>",
        ),
    );
    assert_height(&layout, "m", 60.0);
    assert_at(&layout, "i1", 0.0, 10.0, 200.0);
    assert_at(&layout, "i2", 0.0, 40.0, 200.0);
    assert_at(&layout, "i3", 200.0, 0.0, 200.0);
    assert_at(&layout, "i4", 200.0, 30.0, 200.0);
}

#[test]
fn orphans_and_widows_keep_the_lines_of_a_paragraph_together_in_columns() {
    let style = "p { margin: 0; line-height: 20px; font-size: 16px; background: rgb(0, 128, 0) }";
    let lines = |count: usize| vec!["line"; count].join("<br>");
    // five lines are three and two
    let five = layout(
        "widows_five",
        &doc(style, &format!("<div id=m style=\"column-count: 2; column-gap: 0\"><p>{}</p></div>", lines(5))),
    );
    let green = five.filled([0, 128, 0]);
    assert_eq!(green.len(), 2);
    assert_close(green[0].rect[3], 60.0, "height of the first part");
    assert_close(green[1].rect[3], 40.0, "height of the second part");
    // three lines cannot be cut: two lines would be left on one of the columns, and the last
    // has to be one
    let three = layout(
        "widows_three",
        &doc(style, &format!("<div id=m style=\"column-count: 2; column-gap: 0\"><p>{}</p></div>", lines(3))),
    );
    assert_eq!(three.filled([0, 128, 0]).len(), 1);
    assert_height(&three, "m", 60.0);
}

#[test]
fn a_box_that_is_cut_between_columns_has_the_border_of_its_parts() {
    // a box of three items with a border and a padding of 5px is cut after its second item
    let layout = layout(
        "cut_box",
        &doc(
            "",
            "<div id=m style=\"column-count: 2; column-gap: 0\"><div id=b style=\"border: 5px solid rgb(0, 0, 255); padding: 5px; background: rgb(0, 128, 0)\">
             <div class=i style=\"height: 20px\"></div><div class=i style=\"height: 20px\"></div><div class=i style=\"height: 20px\"></div>
             <div class=i style=\"height: 20px\"></div></div></div>",
        ),
    );
    // 80px of content and 20px of edges are 100 in all, 50 in each column: the first part has the
    // top border and padding (10) and two items (40), the second two items and the bottom edges
    let green = layout.filled([0, 128, 0]);
    assert_eq!(green.len(), 2, "{green:?}");
    assert_close(green[0].rect[0], 0.0, "x of the first part");
    assert_close(green[1].rect[0], 200.0, "x of the second part");
    assert_close(green[1].rect[1], 0.0, "top of the second part");
    // no border at the cut: the first part has no bottom border and the second no top border
    let blue = layout.filled([0, 0, 255]);
    let horizontal: Vec<&&Rect> = blue.iter().filter(|rect| rect.rect[2] > 100.0).collect();
    assert_eq!(horizontal.len(), 2, "one top and one bottom border: {blue:?}");
}

/// The log of a box that cannot be split and is taller than a page: it overflows its page, and the
/// part of it that is below the page is not printed. A container that can be split over its columns
/// and pages does not lose content (see the tests below), so the box here is one item that is taller
/// than a page.
#[test]
fn a_box_taller_than_a_page_in_a_container_says_that_its_content_is_cut_off() {
    let layout = layout(
        "tall_warning",
        &doc("", "<div id=m style=\"column-count: 2\"><div id=tall style=\"height: 800px\"></div></div>"),
    );
    assert!(
        layout.log.contains("the content below the page is not printed"),
        "no warning about the cut content in the log:\n{}",
        layout.log
    );
    // the box is on the page, and it is as tall as it is
    assert_height(&layout, "tall", 800.0);
    assert_eq!(layout.get("tall").page, 0);
}

// A container that is split over pages. These tests describe what step 3 of the multi-column
// order of work in `docs/pagination.md` has to do. Today a container taller than a page is one
// box, balanced over its whole content: its columns are as tall as the content allows, and the
// part of them that is below the page is lost.

/// Asserts the page and the top left corner of a box.
fn assert_on(layout: &Layout, id: &str, page: u64, x: f64, y: f64) {
    let item = layout.get(id);
    assert_eq!(item.page, page, "page of {id}");
    assert_close(item.rect[0], x, &format!("x of {id}"));
    assert_close(item.rect[1], y, &format!("y of {id}"));
}

#[test]
fn no_content_of_a_tall_container_is_below_its_page() {
    // 100 items of 20px in two columns are 2000px of content on pages that are 600px high
    let layout = layout("tall_invariant", &doc("", &container("column-count: 2", 100)));
    for number in 1..=100 {
        let item = layout.get(&format!("i{number}"));
        assert!(
            item.rect[1] + item.rect[3] <= 600.0 + 0.01,
            "i{number} ends at {} on page {}, below the page",
            item.rect[1] + item.rect[3],
            item.page
        );
        assert!(item.page <= 1, "i{number} is on page {}, there should be two pages", item.page);
    }
}

#[test]
#[ignore = "x of the second column is 208 and not 200: the default column-gap is 1em = 16px (see the same test with column-gap: 0 below)"]
fn the_columns_of_every_page_are_filled_in_turn_with_column_fill_auto() {
    // a page holds 30 items per column and 60 per page
    let layout = layout(
        "fill_auto_pages",
        &doc(
            "",
            &format!(
                "{}<p id=after style=\"margin:0;height:20px\"></p>",
                container("column-count: 2; column-fill: auto", 100)
            ),
        ),
    );
    assert_on(&layout, "i1", 0, 0.0, 0.0);
    assert_on(&layout, "i30", 0, 0.0, 580.0);
    assert_on(&layout, "i31", 0, 200.0, 0.0);
    assert_on(&layout, "i60", 0, 200.0, 580.0);
    assert_on(&layout, "i61", 1, 0.0, 0.0);
    assert_on(&layout, "i90", 1, 0.0, 580.0);
    // the last page is not balanced: the second column has what the first one cannot hold
    assert_on(&layout, "i91", 1, 200.0, 0.0);
    assert_on(&layout, "i100", 1, 200.0, 180.0);
    // the first column of the last page is full, so the container is as high as the page
    assert_on(&layout, "after", 2, 0.0, 0.0);
}

#[test]
#[ignore = "x of the second column is 208 and not 200: the default column-gap is 1em = 16px (see the same test with column-gap: 0 below)"]
fn only_the_last_page_of_a_container_is_balanced() {
    // the columns are balanced by default: the pages before the last are full all the same
    let layout = layout(
        "balance_last_page",
        &doc(
            "",
            &format!(
                "{}<p id=after style=\"margin:0;height:20px\"></p>",
                container("column-count: 2", 100)
            ),
        ),
    );
    assert_on(&layout, "i30", 0, 0.0, 580.0);
    assert_on(&layout, "i31", 0, 200.0, 0.0);
    assert_on(&layout, "i60", 0, 200.0, 580.0);
    // 40 items are left for the last page: 20 in each column
    assert_on(&layout, "i61", 1, 0.0, 0.0);
    assert_on(&layout, "i80", 1, 0.0, 380.0);
    assert_on(&layout, "i81", 1, 200.0, 0.0);
    assert_on(&layout, "i100", 1, 200.0, 380.0);
    // the container ends where its balanced columns end
    assert_on(&layout, "after", 1, 0.0, 400.0);
}

#[test]
#[ignore = "x of the second column is 208 and not 200: the default column-gap is 1em = 16px (see the same test with column-gap: 0 below)"]
fn the_first_page_of_a_container_is_as_high_as_the_rest_of_the_page() {
    // 100px of content in front of the container leave 500px: 25 items per column
    let layout = layout(
        "first_fragment",
        &doc(
            "",
            &format!(
                "<div style=\"height:100px\"></div>{}",
                container("column-count: 2; column-fill: auto", 100)
            ),
        ),
    );
    assert_on(&layout, "i1", 0, 0.0, 100.0);
    assert_on(&layout, "i25", 0, 0.0, 580.0);
    assert_on(&layout, "i26", 0, 200.0, 100.0);
    assert_on(&layout, "i50", 0, 200.0, 580.0);
    // the next page has the full height again
    assert_on(&layout, "i51", 1, 0.0, 0.0);
    assert_on(&layout, "i80", 1, 0.0, 580.0);
}

#[test]
fn a_forced_page_break_in_a_container_ends_the_page() {
    // ten items fill part of the first column; the item with the break starts the next page
    let body = format!(
        "<div id=m style=\"column-count: 2; column-fill: auto\">{}\
         <div class=i id=i11 style=\"height:20px;break-before:page\"></div>{}</div>",
        items(1, 10),
        items(12, 9)
    );
    let layout = layout("forced_page", &doc("", &body));
    assert_on(&layout, "i10", 0, 0.0, 180.0);
    assert_on(&layout, "i11", 1, 0.0, 0.0);
    assert_on(&layout, "i20", 1, 0.0, 180.0);
}

// The three tests above that are ignored with the default `column-gap: normal` (1em = 16px, so a
// column is 192px wide and the second one starts at 208px, not at 200px as they assert), with a gap
// of 0, which is the arithmetic that they describe. Nothing else is different.

#[test]
fn the_columns_of_every_page_are_filled_in_turn_with_column_fill_auto_and_no_gap() {
    let layout = layout(
        "fill_auto_pages_no_gap",
        &doc(
            "",
            &format!(
                "{}<p id=after style=\"margin:0;height:20px\"></p>",
                container("column-count: 2; column-gap: 0; column-fill: auto", 100)
            ),
        ),
    );
    assert_on(&layout, "i1", 0, 0.0, 0.0);
    assert_on(&layout, "i30", 0, 0.0, 580.0);
    assert_on(&layout, "i31", 0, 200.0, 0.0);
    assert_on(&layout, "i60", 0, 200.0, 580.0);
    assert_on(&layout, "i61", 1, 0.0, 0.0);
    assert_on(&layout, "i90", 1, 0.0, 580.0);
    assert_on(&layout, "i91", 1, 200.0, 0.0);
    assert_on(&layout, "i100", 1, 200.0, 180.0);
    assert_on(&layout, "after", 2, 0.0, 0.0);
}

#[test]
fn only_the_last_page_of_a_container_is_balanced_with_no_gap() {
    let layout = layout(
        "balance_last_page_no_gap",
        &doc(
            "",
            &format!(
                "{}<p id=after style=\"margin:0;height:20px\"></p>",
                container("column-count: 2; column-gap: 0", 100)
            ),
        ),
    );
    assert_on(&layout, "i30", 0, 0.0, 580.0);
    assert_on(&layout, "i31", 0, 200.0, 0.0);
    assert_on(&layout, "i60", 0, 200.0, 580.0);
    assert_on(&layout, "i61", 1, 0.0, 0.0);
    assert_on(&layout, "i80", 1, 0.0, 380.0);
    assert_on(&layout, "i81", 1, 200.0, 0.0);
    assert_on(&layout, "i100", 1, 200.0, 380.0);
    assert_on(&layout, "after", 1, 0.0, 400.0);
}

#[test]
fn the_first_page_of_a_container_is_as_high_as_the_rest_of_the_page_with_no_gap() {
    let layout = layout(
        "first_fragment_no_gap",
        &doc(
            "",
            &format!(
                "<div style=\"height:100px\"></div>{}",
                container("column-count: 2; column-gap: 0; column-fill: auto", 100)
            ),
        ),
    );
    assert_on(&layout, "i1", 0, 0.0, 100.0);
    assert_on(&layout, "i25", 0, 0.0, 580.0);
    assert_on(&layout, "i26", 0, 200.0, 100.0);
    assert_on(&layout, "i50", 0, 200.0, 580.0);
    assert_on(&layout, "i51", 1, 0.0, 0.0);
    assert_on(&layout, "i80", 1, 0.0, 580.0);
}
