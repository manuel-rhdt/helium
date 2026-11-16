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
    let out = common::helium()
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
