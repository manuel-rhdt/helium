//! Pages are laid out through a window on the box tree (see `docs/pagination.md`): what the
//! paginator counts for a page is what the page has when it is laid out.

mod common;

use serde_json::Value;

fn run(name: &str, html: &str) -> (Value, String) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("page_windows__{}.html", name));
    let output = dir.join(format!("page_windows__{}.pdf", name));
    let dump = dir.join(format!("page_windows__{}.json", name));
    std::fs::write(&input, html).unwrap();
    run_file(&input, &output, &dump)
}

fn run_file(input: &std::path::Path, output: &std::path::Path, dump: &std::path::Path) -> (Value, String) {
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts", "--dump-layout"])
        .arg(dump)
        .arg(input)
        .arg(output)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{}", log);
    (serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap(), log)
}

fn elements<'a>(node: &'a Value, tag: &str, found: &mut Vec<&'a Value>) {
    if node["tag"] == tag && node["border_box"].is_object() {
        found.push(node);
    }
    for child in node["children"].as_array().into_iter().flatten() {
        elements(child, tag, found);
    }
}

fn number(value: &Value, key: &str) -> f64 {
    value["border_box"][key].as_f64().unwrap()
}

#[test]
fn the_height_of_a_page_is_what_the_paginator_counted_in_every_document() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut documents: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/documents"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    for entry in std::fs::read_dir(root.join("gallery")).unwrap() {
        let index = entry.unwrap().path().join("index.html");
        if index.exists() {
            documents.push(index);
        }
    }
    assert!(documents.len() > 20);
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (number, document) in documents.iter().enumerate() {
        let (_, log) = run_file(
            document,
            &dir.join(format!("page_windows_doc{number}.pdf")),
            &dir.join(format!("page_windows_doc{number}.json")),
        );
        assert!(!log.contains("the paginator counted"), "{}: {}", document.display(), log);
    }
}

#[test]
fn a_table_is_split_between_its_rows_and_the_rows_move_to_the_top_of_the_page() {
    let rows = "<tr><td>row</td></tr>".repeat(8);
    let (dump, log) = run(
        "table",
        &format!(
            "<style>@page {{ size: 300px 300px; margin: 20px; }} body {{ margin: 0 }}
             p {{ margin: 0; height: 100px }} td {{ height: 50px; padding: 0 }}</style>
             <p></p><table>{rows}</table><p></p>"
        ),
    );
    assert!(!log.contains("the paginator counted"), "{log}");
    let mut found = vec![];
    elements(&dump["document"], "tr", &mut found);
    let pages: Vec<u64> = found.iter().map(|row| row["page"].as_u64().unwrap()).collect();
    // 260px of content: the paragraph and three rows, then five rows on a page, then the rest
    assert_eq!(pages, [0, 0, 0, 1, 1, 1, 1, 1], "{pages:?}");
    assert_eq!(number(found[3], "y"), 20.0);
    // (the rows of a table are 2px apart)
    assert_eq!(number(found[7], "y"), 20.0 + 4.0 * 52.0);
}

#[test]
fn a_box_with_a_padding_that_starts_a_page_has_its_padding_on_that_page() {
    let (dump, _) = run(
        "padding",
        "<style>@page { size: 300px 300px; margin: 20px; } body { margin: 0 }
         .a { height: 200px } .b { padding: 10px; margin-top: 30px }</style>
         <div class=a></div><div class=b><p style=\"margin:0;height:100px\"></p></div>",
    );
    let mut found = vec![];
    elements(&dump["document"], "div", &mut found);
    let b = found.iter().find(|div| div["class"] == "b").unwrap();
    // the margin in front of the break is discarded, the padding is part of the box
    assert_eq!(b["page"].as_u64(), Some(1));
    assert_eq!(number(b, "y"), 20.0);
    assert_eq!(number(b, "height"), 120.0);
}

#[test]
fn a_paragraph_on_the_first_page_is_broken_for_the_margins_of_the_first_page() {
    let text = "the quick brown fox jumps over the lazy dog ".repeat(60);
    let (dump, log) = run(
        "lines",
        &format!(
            "<style>@page {{ size: 300pt 200pt; margin: 20pt; }} @page :first {{ margin: 20pt 60pt; }}
             body {{ margin: 0; font: 12pt/20pt sans-serif }} p {{ margin: 0; orphans: 1; widows: 1 }}</style>
             <p>{text}</p>"
        ),
    );
    assert!(!log.contains("the paginator counted"), "{log}");
    let mut found = vec![];
    elements(&dump["document"], "p", &mut found);
    // the first page is 180pt wide, the others 260pt
    assert_eq!(found[0]["page"].as_u64(), Some(0));
    let width = number(found[0], "width") * 0.75;
    assert!((width - 180.0).abs() < 0.1, "{width}");
}

/// A table of full width whose rows continue on a page that is wider than the one it starts on:
/// the first page is 160px wide, the others 360px.
fn table_over_pages_of_different_widths(name: &str) -> (Value, String) {
    let cell = "<td>aaa bbb ccc ddd eee fff ggg hhh</td><td>b</td>";
    let rows = format!("<tr>{cell}</tr>").repeat(6);
    run(
        name,
        &format!(
            "<style>@page {{ size: 400px 300px; margin: 20px }} @page :first {{ margin: 20px 120px }}
             body {{ margin: 0; font: 20px/20px sans-serif }}
             table {{ width: 100%; border-collapse: separate }} td {{ padding: 0 }}</style>
             <table>{rows}</table>"
        ),
    )
}

#[test]
fn the_rows_of_a_table_on_a_page_of_another_width_are_laid_out_for_that_page() {
    let (dump, log) = table_over_pages_of_different_widths("table_widths");
    // the height that the paginator counted for the rows of the second page is the one that they
    // have there
    assert!(!log.contains("the paginator counted"), "{log}");
    let mut rows = vec![];
    elements(&dump["document"], "tr", &mut rows);
    assert_eq!(rows.len(), 6);
    let first: Vec<_> = rows.iter().filter(|row| row["page"] == 0).collect();
    let later: Vec<_> = rows.iter().filter(|row| row["page"] == 1).collect();
    assert!(!first.is_empty() && !later.is_empty());
    // the table is as wide as its page, so the rows are 200px wider on the page that is wider
    assert!((number(later[0], "width") - number(first[0], "width") - 200.0).abs() < 0.01);
    // and the text of a cell breaks for the width of the cell on its page
    assert!(number(first[0], "height") > number(later[0], "height"));
}

#[test]
#[ignore = "the columns of a split table are laid out again for every page (the table is laid out whole at \
            the width of each page, and the window shows Taffy all its cells); keeping the columns of the \
            page where the table starts needs the tracks to be pinned in the style of the window and the \
            extents of the rows to be asked at those tracks (step 6 of the order of work in \
            docs/pagination.md)"]
fn the_columns_of_a_table_stay_the_ones_of_the_page_where_it_starts() {
    let (dump, _) = table_over_pages_of_different_widths("table_columns");
    let mut cells = vec![];
    elements(&dump["document"], "td", &mut cells);
    let widths = |page: u64| -> Vec<f64> {
        cells.iter().filter(|cell| cell["page"] == page).map(|cell| number(cell, "width")).collect()
    };
    let (first, later) = (widths(0), widths(1));
    assert!(!first.is_empty() && !later.is_empty());
    assert!((first[0] - later[0]).abs() < 0.01 && (first[1] - later[1]).abs() < 0.01, "{first:?} {later:?}");
}
