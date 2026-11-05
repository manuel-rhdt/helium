//! The collapsing border model of tables (`border-collapse: collapse`), tested against the
//! layout dump: the boxes of the cells and the borders that are painted.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn dump(name: &str, style: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("table_borders_{}.html", name));
    let html = format!(
        r#"<html><head><style>body {{ font-family: "DejaVu Sans"; }} td {{ padding: 0 }} {}</style></head><body>{}</body></html>"#,
        style, body
    );
    std::fs::write(&input, html).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("table_borders_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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

/// `(x, y, width, height)` of the border box of an element.
fn border_box(dump: &Value, id: &str) -> (f64, f64, f64, f64) {
    let b = &element(dump, id)["border_box"];
    let get = |key: &str| b[key].as_f64().unwrap();
    (get("x"), get("y"), get("width"), get("height"))
}

/// The painted rectangles of the given color as `(x, y, width, height)`.
fn painted(dump: &Value, color: [u64; 3]) -> Vec<(f64, f64, f64, f64)> {
    dump["paint"]["rects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rect| {
            let c = rect["color"].as_array().unwrap();
            (0..3).all(|i| c[i].as_u64() == Some(color[i]))
        })
        .map(|rect| {
            let get = |key: &str| rect[key].as_f64().unwrap();
            (get("x"), get("y"), get("width"), get("height"))
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

const GRID: &str = r#"<table><tr><td id="a">a</td><td id="b">b</td></tr>
    <tr><td id="c">c</td><td id="d">d</td></tr></table>"#;

#[test]
fn adjacent_cells_share_their_borders() {
    let dump = dump(
        "grid",
        "table { border-collapse: collapse } td { border: 2px solid rgb(255, 0, 0) }",
        GRID,
    );
    let (ax, ay, aw, ah) = border_box(&dump, "a");
    let (bx, _, _, _) = border_box(&dump, "b");
    let (_, cy, _, _) = border_box(&dump, "c");
    // no spacing between the cells, and each has half of the shared border
    assert!(close(bx, ax + aw), "a ends at {}, b starts at {bx}", ax + aw);
    assert!(close(cy, ay + ah));
    assert!(close(element(&dump, "a")["border"]["right"].as_f64().unwrap(), 1.0));

    // 3 horizontal lines of 2 segments and 3 vertical lines of 2 segments, each painted once
    // with the full width
    let rects = painted(&dump, [255, 0, 0]);
    assert_eq!(rects.len(), 12, "{:?}", rects);
    for (_, _, width, height) in &rects {
        assert!(close(width.min(*height), 2.0), "{:?}", rects);
    }
    // the border between a and b is centered on their common edge
    let shared = rects
        .iter()
        .find(|&&(x, y, width, _)| close(x, bx - 1.0) && close(width, 2.0) && y < ay + 1.0)
        .expect("the border between a and b");
    assert!(shared.3 >= ah);
}

#[test]
fn the_wider_border_wins() {
    let dump = dump(
        "wider",
        "table { border-collapse: collapse; border: 6px solid rgb(0, 0, 255) }
         td { border: 2px solid rgb(255, 0, 0) }",
        &GRID.replace("<table>", r#"<table id="t">"#),
    );
    // the outer edges are the table's (4 sides of 2 segments), the inner ones the cells'
    assert_eq!(painted(&dump, [0, 0, 255]).len(), 8);
    assert_eq!(painted(&dump, [255, 0, 0]).len(), 4);
    // the table has half of its border outside of the cells, and no padding
    let (tx, _, _, _) = border_box(&dump, "t");
    let (ax, _, _, _) = border_box(&dump, "a");
    assert!(close(ax - tx, 3.0), "table at {tx}, first cell at {ax}");
}

#[test]
fn a_wider_row_border_wins_over_the_cell_borders() {
    let dump = dump(
        "row",
        "table { border-collapse: collapse }
         td { border: 2px solid rgb(255, 0, 0) }
         tr.first { border-bottom: 3px solid rgb(0, 128, 0) }",
        &GRID.replacen("<tr>", r#"<tr class="first">"#, 1),
    );
    // the line below the first row, painted by c and d on their top edges
    let green = painted(&dump, [0, 128, 0]);
    assert_eq!(green.len(), 2, "{:?}", green);
    let (_, cy, _, _) = border_box(&dump, "c");
    assert!(green.iter().all(|&(_, y, _, h)| close(y + h / 2.0, cy) && close(h, 3.0)));
}

#[test]
fn a_cell_border_wins_over_a_row_border_of_the_same_width() {
    let dump = dump(
        "row_tie",
        "table { border-collapse: collapse }
         td { border: 2px solid rgb(255, 0, 0) }
         tr.first { border-bottom: 2px solid rgb(0, 128, 0) }",
        &GRID.replacen("<tr>", r#"<tr class="first">"#, 1),
    );
    assert!(painted(&dump, [0, 128, 0]).is_empty());
}

#[test]
fn between_cells_the_one_to_the_left_or_top_wins() {
    let dump = dump(
        "tie",
        "table { border-collapse: collapse }
         td { border: 2px solid rgb(255, 0, 0) }
         #a { border-color: rgb(0, 128, 0) }
         #d { border-color: rgb(0, 0, 255) }",
        GRID,
    );
    // a wins all of its edges; d loses its top and left edges to b and c
    assert_eq!(painted(&dump, [0, 128, 0]).len(), 4);
    assert_eq!(painted(&dump, [0, 0, 255]).len(), 2);
}

#[test]
fn right_to_left_tables_prefer_the_cell_to_the_right() {
    let dump = dump(
        "rtl",
        "table { border-collapse: collapse }
         td { border: 2px solid rgb(255, 0, 0) }
         #a { border-color: rgb(0, 128, 0) }",
        r#"<table dir="rtl"><tr><td id="a">a</td><td id="b">b</td></tr></table>"#,
    );
    let (ax, _, _, _) = border_box(&dump, "a");
    let (bx, _, bw, _) = border_box(&dump, "b");
    assert!(close(ax, bx + bw), "a is right of b");
    // a (the first cell, at the right) wins the edge between the cells
    let green = painted(&dump, [0, 128, 0]);
    assert_eq!(green.len(), 4, "{:?}", green);
    assert!(green.iter().any(|&(x, _, w, _)| close(x + w / 2.0, ax)));
}

#[test]
fn cells_spanning_rows_and_columns_have_no_inner_borders() {
    let dump = dump(
        "spans",
        "table { border-collapse: collapse } td { border: 1px solid rgb(255, 0, 0) }",
        r#"<table><tr><td id="a" rowspan="2">a</td><td id="b" colspan="2">b</td></tr>
           <tr><td>c</td><td>d</td></tr></table>"#,
    );
    // one line per side of a cell: the top and left ones, and the bottom and right ones at the
    // edges of the table (a: 3, b: 3, c: 3, d: 4)
    let rects = painted(&dump, [255, 0, 0]);
    assert_eq!(rects.len(), 13, "{:?}", rects);
    // no line runs through the cells that span
    for (id, horizontal) in [("a", true), ("b", false)] {
        let (x, y, w, h) = border_box(&dump, id);
        let inside = |rx: f64, ry: f64, rw: f64, rh: f64| {
            if horizontal {
                ry > y + 1.0 && ry + rh < y + h - 1.0 && rx < x + w - 1.0 && rx + rw > x + 1.0
            } else {
                rx > x + 1.0 && rx + rw < x + w - 1.0 && ry < y + h - 1.0 && ry + rh > y + 1.0
            }
        };
        assert!(
            !rects.iter().any(|&(rx, ry, rw, rh)| inside(rx, ry, rw, rh)),
            "a line inside of {id}: {:?}",
            rects
        );
    }
}

#[test]
fn separated_borders_are_unchanged() {
    let dump = dump(
        "separate",
        "table { border-spacing: 4px } td { border: 2px solid rgb(255, 0, 0) }",
        GRID,
    );
    let (ax, _, aw, _) = border_box(&dump, "a");
    let (bx, _, _, _) = border_box(&dump, "b");
    assert!(close(bx - (ax + aw), 4.0));
    // every cell paints its own 4 borders
    assert_eq!(painted(&dump, [255, 0, 0]).len(), 16);
}
