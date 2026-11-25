//! The first page of the document has its own margins.

mod common;

/// One laid out box: the tag, the page and the border box (x, y, width, height in points).
#[derive(Debug)]
struct Item {
    tag: String,
    page: u64,
    rect: [f64; 4],
}

fn layout(name: &str, html: &str) -> (Vec<Item>, String) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("page_groups__{}.html", name));
    let output = dir.join(format!("page_groups__{}.pdf", name));
    let dump = dir.join(format!("page_groups__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::umbruch()
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
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    fn walk(node: &serde_json::Value, items: &mut Vec<Item>) {
        if node["type"] == "element" && node["border_box"].is_object() {
            let b = &node["border_box"];
            let get = |key: &str| b[key].as_f64().unwrap() * 0.75;
            items.push(Item {
                tag: node["tag"].as_str().unwrap().to_owned(),
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
    (items, log)
}

fn boxes<'a>(items: &'a [Item], tag: &str) -> Vec<&'a Item> {
    items.iter().filter(|item| item.tag == tag).collect()
}

fn assert_rect(item: &Item, x: f64, y: f64, width: f64) {
    let [ax, ay, aw, _] = item.rect;
    for (actual, expected, what) in [(ax, x, "x"), (ay, y, "y"), (aw, width, "width")] {
        assert!(
            (actual - expected).abs() < 0.1,
            "{what} of {item:?} is {actual}, expected {expected}"
        );
    }
}

#[test]
fn the_first_page_has_its_own_margins() {
    // the page is 300pt wide and has 50pt margins, the first page has 10pt margins
    let (items, _) = layout(
        "first",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         .box { height: 40pt; }</style>
         <div class=\"box\"></div><p style=\"break-before: page\">second page</p>",
    );
    assert_rect(boxes(&items, "div")[0], 10.0, 10.0, 280.0);
    // the second page has the margins of the document
    let p = boxes(&items, "p")[0];
    assert_eq!(p.page, 1);
    assert_rect(p, 50.0, 62.0, 200.0);
}

#[test]
fn a_named_page_can_have_no_margins() {
    let (items, _) = layout(
        "named",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page cover { margin: 0; }
         .cover { page: cover; height: 100pt; }</style>
         <div class=\"cover\">Cover</div><p>text</p><p>more</p>",
    );
    assert_rect(boxes(&items, "div")[0], 0.0, 0.0, 300.0);
    let p = boxes(&items, "p");
    assert_eq!(p[0].page, 1);
    assert_rect(p[0], 50.0, 50.0 + 12.0, 200.0);
}

#[test]
fn only_the_first_page_has_the_margins_of_the_first_page() {
    // three boxes of 120pt: two fit on the first page (280pt of content), the third is on the
    // second page, which is not the first page of the document and has its margins
    let (items, _) = layout(
        "group",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         .box { height: 120pt; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>
         <p style=\"break-before: page\">after the group</p>",
    );
    let divs = boxes(&items, "div");
    assert_eq!(divs.iter().map(|d| d.page).collect::<Vec<_>>(), [0, 0, 1]);
    assert_rect(divs[0], 10.0, 10.0, 280.0);
    assert_rect(divs[1], 10.0, 130.0, 280.0);
    assert_rect(divs[2], 50.0, 50.0, 200.0);
    let p = boxes(&items, "p")[0];
    assert_eq!(p.page, 2);
    assert_rect(p, 50.0, 62.0, 200.0);
}

#[test]
fn the_first_page_has_another_width_without_a_forced_break() {
    let (items, log) = layout(
        "unbroken",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         .box { height: 40pt; }</style><div class=\"box\"></div><p>flows on</p>",
    );
    // the first page is laid out for its own margins
    assert_rect(boxes(&items, "div")[0], 10.0, 10.0, 280.0);
    assert!(!log.contains("forced page break"), "{}", log);
}

#[test]
fn margins_of_the_document_are_not_touched_without_other_pages() {
    let (items, log) = layout(
        "plain",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } .box { height: 40pt; }</style>
         <div class=\"box\"></div><p style=\"break-before: page\">second</p>",
    );
    assert_rect(boxes(&items, "div")[0], 50.0, 50.0, 200.0);
    assert_rect(boxes(&items, "p")[0], 50.0, 62.0, 200.0);
    assert!(!log.contains("forced page break"), "{}", log);
}

#[test]
fn the_pages_have_their_own_height() {
    // the first page has a content area of 280pt, the others of 200pt: two boxes of 130pt fit on
    // the first page, and only one on each following page
    let (items, _) = layout(
        "heights",
        "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :first { margin: 10pt; }
         .box { height: 130pt; }</style>
         <div class=\"box\"></div><div class=\"box\"></div><div class=\"box\"></div>",
    );
    let pages: Vec<u64> = boxes(&items, "div").iter().map(|d| d.page).collect();
    assert_eq!(pages, [0, 0, 1], "{items:?}");
}

const CHAPTERS: &str = "<style>@page { size: 300pt 300pt; margin: 50pt; } @page :nth(1 of chapter) { margin-top: 10pt; }
    section { page: chapter; break-before: page } .box { height: 150pt; }</style>
    <section><div class=\"box\"></div><div class=\"box\"></div></section>
    <section><div class=\"box\"></div><div class=\"box\"></div></section>";

#[test]
fn the_first_page_of_a_page_group_has_its_own_margins() {
    // the content area is 200pt high, so a box of 150pt fills a page; the first page of every
    // chapter (a page group) has a margin of 10pt on top, the others of 50pt
    let (items, log) = layout("group_margins", CHAPTERS);
    let divs = boxes(&items, "div");
    assert_eq!(divs.iter().map(|d| d.page).collect::<Vec<_>>(), [0, 1, 2, 3], "{log}");
    let tops: Vec<f64> = divs.iter().map(|d| d.rect[1]).collect();
    for (actual, expected) in tops.iter().zip([10.0, 50.0, 10.0, 50.0]) {
        assert!((actual - expected).abs() < 0.1, "{tops:?}");
    }
}

#[test]
fn only_the_first_page_of_a_group_is_known_where_the_pages_are_chosen() {
    // the size and the margins follow `:nth(1 of name)` only: that the second page of a group
    // has other margins would change how many boxes fit before the pages are chosen. The rule is
    // for the margin boxes and the counters of the page.
    let (items, _) = layout(
        "group_later",
        &CHAPTERS.replace("@page :nth(1 of chapter) { margin-top: 10pt; }", "@page :nth(2 of chapter) { margin-top: 10pt; }"),
    );
    let tops: Vec<f64> = boxes(&items, "div").iter().map(|d| d.rect[1]).collect();
    assert!(tops.iter().all(|top| (top - 50.0).abs() < 0.1), "{tops:?}");
}
