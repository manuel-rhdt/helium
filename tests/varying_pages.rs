//! Content that flows over pages of differing geometry: other margins on the first page, on
//! left and right pages, and other page sizes, without forced breaks around the content.
//!
//! See `docs/pagination.md`.

mod common;

use serde_json::Value;

/// One laid out box: the tag, the class, the page and the border box (x, y, width, height in
/// points).
#[derive(Debug)]
struct Item {
    tag: String,
    class: String,
    page: u64,
    rect: [f64; 4],
}

/// A filled rectangle: the page and the box in points.
#[derive(Debug)]
struct Rect {
    page: u64,
    rect: [f64; 4],
    color: [u64; 3],
}

struct Layout {
    items: Vec<Item>,
    rects: Vec<Rect>,
    log: String,
}

fn layout(name: &str, html: &str) -> Layout {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("varying_pages__{}.html", name));
    let output = dir.join(format!("varying_pages__{}.pdf", name));
    let dump = dir.join(format!("varying_pages__{}.json", name));
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
    let value: Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    fn walk(node: &Value, items: &mut Vec<Item>) {
        if node["type"] == "element" && node["border_box"].is_object() {
            let b = &node["border_box"];
            let get = |key: &str| b[key].as_f64().unwrap() * 0.75;
            items.push(Item {
                tag: node["tag"].as_str().unwrap().to_owned(),
                class: node["class"].as_str().unwrap_or("").to_owned(),
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
            let get = |key: &str| rect[key].as_f64().unwrap() * 0.75;
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
    fn boxes(&self, class: &str) -> Vec<&Item> {
        self.items.iter().filter(|item| item.class == class).collect()
    }

    fn pages(&self, class: &str) -> Vec<u64> {
        self.boxes(class).iter().map(|item| item.page).collect()
    }

    /// The rectangles of a colour, by page.
    fn filled(&self, color: [u64; 3]) -> Vec<&Rect> {
        let mut rects: Vec<&Rect> = self.rects.iter().filter(|rect| rect.color == color).collect();
        rects.sort_by_key(|rect| rect.page);
        rects
    }
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 0.1,
        "{what} is {actual}, expected {expected}"
    );
}

/// Asserts the left edge, the top edge and the width of a box, in points.
fn assert_rect(item: &Item, x: f64, y: f64, width: f64) {
    let [ax, ay, aw, _] = item.rect;
    assert_close(ax, x, &format!("x of {item:?}"));
    assert_close(ay, y, &format!("y of {item:?}"));
    assert_close(aw, width, &format!("width of {item:?}"));
}

/// Some words that fill `lines` lines of about 200pt.
fn text(lines: usize) -> String {
    "the quick brown fox jumps over the lazy dog ".repeat(lines * 2)
}

#[test]
fn content_flows_from_a_wide_first_page_to_narrower_pages() {
    // The first page has a content area of 280pt by 280pt, the others of 200pt by 200pt. Two
    // boxes of 120pt fit on the first page; the third is on the second page and as wide as it.
    let layout = layout(
        "first",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         body { margin: 0; } .box { height: 120pt; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>",
    );
    let boxes = layout.boxes("box");
    assert_eq!(layout.pages("box"), [0, 0, 1]);
    assert_rect(boxes[0], 10.0, 10.0, 280.0);
    assert_rect(boxes[1], 10.0, 130.0, 280.0);
    assert_rect(boxes[2], 50.0, 50.0, 200.0);
    assert!(!layout.log.contains("forced page break"), "{}", layout.log);
}

#[test]
fn a_page_after_the_first_of_forced_content_has_the_margins_of_the_document() {
    // The content in front of the forced break takes two pages. Only the first of them is the
    // first page of the document.
    let layout = layout(
        "first_forced",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         body { margin: 0; } .box { height: 120pt; } .after { height: 20pt; break-before: page; }
         </style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>
         <div class=\"after\"></div>",
    );
    let boxes = layout.boxes("box");
    assert_eq!(layout.pages("box"), [0, 0, 1]);
    assert_rect(boxes[1], 10.0, 130.0, 280.0);
    assert_rect(boxes[2], 50.0, 50.0, 200.0);
    let after = layout.boxes("after")[0];
    assert_eq!(after.page, 2);
    assert_rect(after, 50.0, 50.0, 200.0);
}

#[test]
fn left_and_right_pages_have_their_own_width() {
    // Right pages (the first, the third) have a content area of 240pt, left pages of 200pt.
    // The sums of the left and right margins differ, so moving the content sideways is not
    // enough.
    let layout = layout(
        "left_right_width",
        "<style>@page { size: 300pt 300pt; margin: 20pt; }
         @page :right { margin: 20pt 20pt 20pt 40pt; }
         @page :left { margin: 20pt 80pt 20pt 20pt; }
         body { margin: 0; } .box { height: 200pt; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>",
    );
    let boxes = layout.boxes("box");
    assert_eq!(layout.pages("box"), [0, 1, 2]);
    assert_rect(boxes[0], 40.0, 20.0, 240.0);
    assert_rect(boxes[1], 20.0, 20.0, 200.0);
    assert_rect(boxes[2], 40.0, 20.0, 240.0);
    assert!(!layout.log.contains("left and right pages differ"), "{}", layout.log);
}

#[test]
fn left_and_right_pages_have_their_own_height() {
    // Right pages have a content area that is 260pt high, left pages one of 100pt. Boxes of
    // 90pt: two fit on a right page, one on a left page.
    let layout = layout(
        "left_right_height",
        "<style>@page { size: 300pt 300pt; margin: 50pt; }
         @page :right { margin: 20pt 50pt; }
         @page :left { margin: 100pt 50pt; }
         body { margin: 0; } .box { height: 90pt; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>
         <div class=\"box\"></div><div class=\"box\"></div>",
    );
    let boxes = layout.boxes("box");
    assert_eq!(layout.pages("box"), [0, 0, 1, 2, 2]);
    assert_rect(boxes[0], 50.0, 20.0, 200.0);
    assert_rect(boxes[2], 50.0, 100.0, 200.0);
    assert_rect(boxes[3], 50.0, 20.0, 200.0);
    assert!(!layout.log.contains("left and right pages differ"), "{}", layout.log);
}

#[test]
fn a_box_that_spans_pages_has_the_width_of_each_page() {
    // The section is split over the first page and the second. Its boxes are as wide as the
    // content area of their page less the padding of the section.
    let layout = layout(
        "nested",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         body { margin: 0; } section { padding: 0 10pt; background: rgb(0, 0, 255); }
         .box { height: 120pt; }</style>
         <section><div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>
         </section>",
    );
    let boxes = layout.boxes("box");
    assert_eq!(layout.pages("box"), [0, 0, 1]);
    assert_rect(boxes[0], 20.0, 10.0, 260.0);
    assert_rect(boxes[1], 20.0, 130.0, 260.0);
    assert_rect(boxes[2], 60.0, 50.0, 180.0);
    // the background of the section is painted once per page, as wide as the page allows
    let fragments = layout.filled([0, 0, 255]);
    assert_eq!(fragments.len(), 2, "{fragments:?}");
    assert_eq!(fragments[0].page, 0);
    assert_close(fragments[0].rect[0], 10.0, "x of the first fragment");
    assert_close(fragments[0].rect[2], 280.0, "width of the first fragment");
    assert_close(fragments[0].rect[3], 240.0, "height of the first fragment");
    assert_eq!(fragments[1].page, 1);
    assert_close(fragments[1].rect[0], 50.0, "x of the second fragment");
    assert_close(fragments[1].rect[2], 200.0, "width of the second fragment");
    assert_close(fragments[1].rect[3], 120.0, "height of the second fragment");
}

#[test]
fn a_paragraph_is_broken_for_the_width_of_each_page() {
    // A paragraph that starts on the wide first page and goes on on the narrower second one.
    // Its lines on the second page are broken for the width of that page.
    let layout = layout(
        "paragraph",
        &format!(
            "<style>@page {{ size: 300pt 300pt; margin: 50pt; }} @page :first {{ margin: 10pt; }}
             body {{ margin: 0; font: 12pt/20pt sans-serif; }}
             p {{ margin: 0; background: rgb(0, 128, 0); orphans: 1; widows: 1; }}</style>
             <p>{}</p>",
            text(30)
        ),
    );
    let fragments = layout.filled([0, 128, 0]);
    assert!(fragments.len() >= 2, "the paragraph takes more than one page: {fragments:?}");
    assert_eq!(fragments[0].page, 0);
    assert_close(fragments[0].rect[0], 10.0, "x on the first page");
    assert_close(fragments[0].rect[2], 280.0, "width on the first page");
    // fourteen lines of 20pt fit on the first page
    assert_close(fragments[0].rect[3], 280.0, "height on the first page");
    for fragment in &fragments[1..] {
        assert_close(fragment.rect[0], 50.0, &format!("x on page {}", fragment.page));
        assert_close(fragment.rect[2], 200.0, &format!("width on page {}", fragment.page));
        assert!(
            fragment.rect[3] <= 200.0 + 0.1,
            "the lines on page {} fit its content area: {fragment:?}",
            fragment.page
        );
    }
}

#[test]
fn the_margin_in_front_of_a_break_does_not_count_against_the_page() {
    // The content area is 200px high. Two boxes of 90px with 20px between them fill it
    // exactly. The margin below the second box is at the break and takes no space.
    let layout = layout(
        "trailing_margin",
        "<style>@page { size: 400px 400px; margin: 100px; }
         body { margin: 0; } .box { height: 90px; margin-bottom: 20px; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>",
    );
    assert_eq!(layout.pages("box"), [0, 0, 1]);
    assert_rect(layout.boxes("box")[2], 75.0, 75.0, 150.0);
}

#[test]
fn a_named_page_has_its_own_size() {
    // The pages named `wide` are landscape: 400pt wide with a content area of 300pt.
    let layout = layout(
        "named_size",
        "<style>@page { size: 300pt 400pt; margin: 50pt; } @page wide { size: 400pt 300pt; }
         body { margin: 0; } .box { height: 120pt; } .wide { page: wide; height: 120pt; }</style>
         <div class=\"box\"></div><div class=\"wide\"></div><div class=\"wide\"></div>
         <div class=\"box\"></div>",
    );
    assert_eq!(layout.pages("box"), [0, 3]);
    assert_eq!(layout.pages("wide"), [1, 2]);
    assert_rect(layout.boxes("box")[0], 50.0, 50.0, 200.0);
    // the content area of a landscape page is 200pt high: one box of 120pt per page
    for wide in layout.boxes("wide") {
        assert_rect(wide, 50.0, 50.0, 300.0);
    }
    assert_rect(layout.boxes("box")[1], 50.0, 50.0, 200.0);
}

#[test]
fn a_break_to_a_right_page_leaves_a_left_page_empty() {
    // The first page is a right page. The box that asks for a right page is on the third page,
    // and the second page is empty.
    let layout = layout(
        "break_right",
        "<style>@page { size: 300pt 300pt; margin: 50pt; }
         body { margin: 0; } .box { height: 50pt; } .right { height: 50pt; break-before: right; }
         .left { height: 50pt; break-before: left; }</style>
         <div class=\"box\"></div><div class=\"right\"></div><div class=\"left\"></div>
         <div class=\"left\"></div>",
    );
    assert_eq!(layout.pages("box"), [0]);
    assert_eq!(layout.pages("right"), [2]);
    // a left page follows a right page directly; the next break to a left page skips a page
    assert_eq!(layout.pages("left"), [3, 5]);
}
