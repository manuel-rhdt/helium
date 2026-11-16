//! Boxes that are split across pages, expressed against the layout dump.

mod common;

use serde_json::Value;

fn rects(name: &str, html: &str) -> Vec<Value> {
    dump(name, html)["paint"]["rects"].as_array().unwrap().clone()
}

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
                .join(format!("fragmentation__{}.pdf", name)),
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

fn element<'a>(dump: &'a Value, id: &str) -> &'a Value {
    fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
        if node["id"] == id {
            return Some(node);
        }
        node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|child| find(child, id))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element #{id}"))
}

/// The page of an element and the bottom edge of its border box.
fn page_and_bottom(dump: &Value, id: &str) -> (u64, f64) {
    let el = element(dump, id);
    let b = &el["border_box"];
    (
        el["page"].as_u64().unwrap(),
        f(b, "y") + f(b, "height"),
    )
}

/// 300pt pages with 20pt margins: the content area ends at 280pt = 373.33px.
const SMALL_PAGE: &str = "@page { width: 300pt; height: 300pt; margin: 20pt } body { margin: 0 }";
const PAGE_BOTTOM: f64 = 280.0 * 4.0 / 3.0 + 0.5;

#[test]
fn table_taller_than_a_page_breaks_between_rows() {
    let rows: String = (0..60)
        .map(|i| format!("<tr id=r{i}><td>row {i}</td><td>x</td></tr>"))
        .collect();
    let html = format!(
        "<style>{SMALL_PAGE}</style><table>{rows}</table><table>{rows}</table><p id=after>AFTER</p>"
    );
    let dump = dump("long_table", &html);

    let mut last_page = 0;
    for i in 0..60 {
        let (page, bottom) = page_and_bottom(&dump, &format!("r{i}"));
        assert!(page >= last_page, "row {i} is on an earlier page");
        assert!(bottom <= PAGE_BOTTOM, "row {i} overflows page {page}: {bottom}");
        last_page = page;
    }
    assert!(last_page >= 3, "60 rows need several pages, got {}", last_page + 1);
    let (after_page, after_bottom) = page_and_bottom(&dump, "after");
    assert!(after_page > last_page);
    assert!(after_bottom <= PAGE_BOTTOM);
}

#[test]
fn table_does_not_break_inside_a_row_span() {
    // Every row is 33px, so 10 rows fit on a page and the page would break between rows 9 and
    // 10, but the cell that spans them must stay in one piece.
    let rows: String = (0..30)
        .map(|i| match i {
            9 => format!("<tr id=r{i}><td>{i}</td><td id=span rowspan=2>span</td></tr>"),
            10 => format!("<tr id=r{i}><td>{i}</td></tr>"),
            _ => format!("<tr id=r{i}><td>{i}</td><td>x</td></tr>"),
        })
        .collect();
    let html = format!(
        "<style>{SMALL_PAGE} tr {{ height: 33px }} table {{ border-spacing: 0 }}</style><table>{rows}</table>"
    );
    let dump = dump("table_row_span", &html);
    let (span_page, span_bottom) = page_and_bottom(&dump, "span");
    let (r10_page, _) = page_and_bottom(&dump, "r10");
    assert_eq!(span_page, r10_page);
    assert!(span_bottom <= PAGE_BOTTOM, "the spanning cell overflows: {span_bottom}");
}

#[test]
fn content_after_an_unbreakable_box_taller_than_a_page_is_kept() {
    let html = format!(
        "<style>{SMALL_PAGE}</style><p id=before>before</p>\
         <div style=\"display: flex; height: 600px\">tall</div><p id=after>AFTER</p>"
    );
    let dump = dump("tall_monolithic", &html);
    let (before_page, _) = page_and_bottom(&dump, "before");
    let (after_page, after_bottom) = page_and_bottom(&dump, "after");
    assert!(after_page > before_page);
    assert!(after_bottom <= PAGE_BOTTOM);
}
