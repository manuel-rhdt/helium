//! Runs vendored web-platform-tests reftests: a test and its `<link rel="match">` reference must
//! produce the same painted output.
//!
//! Helium has no rasterizer, so instead of comparing pixels we compare the `paint` section of
//! `helium --dump-layout`: the solid rectangles (backgrounds and borders) and the glyph runs with
//! their positions. This is independent of how the two documents are structured, which is what
//! reftests rely on. Following the WPT convention that "red means failure", a test must also not
//! paint anything pure red.

mod common;

use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Maximum difference in px for two coordinates to count as equal.
const TOLERANCE: f64 = 0.5;

fn dump_layout(input: &Path) -> Result<Value, String> {
    let output = common::helium()
        .unwrap()
        .arg("--font-dir")
        .arg(wpt_root().join("fonts"))
        .arg("--dump-layout")
        .arg("-")
        .arg(input)
        .arg(pdf_path(input))
        .output()
        .unwrap();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "helium failed: {}",
            stderr.lines().last().unwrap_or("")
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid layout dump: {}", e))
}

/// Finds the reference of a test from its `<link rel="match" href="...">`.
fn find_reference(test: &Path) -> Option<PathBuf> {
    let html = std::fs::read_to_string(test).ok()?;
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<link") {
        let tag_end = rest[start..].find('>')? + start;
        let tag = &rest[start..tag_end];
        rest = &rest[tag_end..];
        if tag.contains("rel=\"match\"") {
            let href = tag.split("href=\"").nth(1)?.split('"').next()?;
            return Some(test.parent()?.join(href));
        }
    }
    None
}

fn approx_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= TOLERANCE
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| approx_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).map_or(false, |w| approx_eq(v, w)))
        }
        _ => a == b,
    }
}

/// Sorted list of the painted items in `paint[kind]`, so that the order they were created in does
/// not matter.
fn paint_items(dump: &Value, kind: &str) -> Vec<Value> {
    let mut items = dump["paint"][kind].as_array().cloned().unwrap_or_default();
    // a paragraph without glyphs (the one of an image, say) paints nothing
    items.retain(|item| {
        item["ops"].as_array().map_or(true, |ops| {
            ops.iter().any(|op| {
                !matches!(op["op"].as_str(), Some("newline" | "tag" | "end_marked_content" | "font" | "color"))
            })
        })
    });
    items.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["page"].as_i64().unwrap_or(0),
                v["y"].as_f64().unwrap_or(0.0).round() as i64,
                v["x"].as_f64().unwrap_or(0.0).round() as i64,
                v["color"].to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    items
}

/// What the filled rectangles paint, as the rectangles of the visible regions of each colour:
/// they are painted in order onto a grid of their edges, and the runs of cells of one colour are
/// merged. A box with borders is then the same as one box of its size, and a rectangle that is
/// covered by a later one does not count.
fn regions(dump: &Value) -> Vec<Value> {
    const SAME: f64 = 0.05;
    let rects: Vec<(i64, [i64; 4], f64, f64, f64, f64)> = dump["paint"]["rects"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let get = |k: &str| r[k].as_f64().unwrap();
            let color = [0, 1, 2, 3].map(|i| r["color"][i].as_i64().unwrap());
            (
                r["page"].as_i64().unwrap_or(0),
                color,
                get("x"),
                get("y"),
                get("x") + get("width"),
                get("y") + get("height"),
            )
        })
        .collect();
    let edges = |values: Vec<f64>| {
        let mut values = values;
        values.sort_by(f64::total_cmp);
        let mut merged: Vec<f64> = vec![];
        for value in values {
            if merged.last().map_or(true, |last| value - last > SAME) {
                merged.push(value);
            }
        }
        merged
    };
    let find = |edges: &[f64], value: f64| edges.iter().position(|e| (e - value).abs() <= SAME).unwrap();
    let mut out = vec![];
    let mut pages: Vec<i64> = rects.iter().map(|r| r.0).collect();
    pages.sort();
    pages.dedup();
    for page in pages {
        let on_page: Vec<_> = rects.iter().filter(|r| r.0 == page).collect();
        let xs = edges(on_page.iter().flat_map(|r| [r.2, r.4]).collect());
        let ys = edges(on_page.iter().flat_map(|r| [r.3, r.5]).collect());
        let mut grid = vec![vec![None; xs.len().saturating_sub(1)]; ys.len().saturating_sub(1)];
        for &&(_, color, x0, y0, x1, y1) in &on_page {
            let (x0, x1) = (find(&xs, x0), find(&xs, x1));
            let (y0, y1) = (find(&ys, y0), find(&ys, y1));
            for row in &mut grid[y0..y1] {
                for cell in &mut row[x0..x1] {
                    *cell = Some(color);
                }
            }
        }
        // the runs of each row, merged with the identical runs of the rows above
        let mut open: Vec<(usize, usize, [i64; 4], usize)> = vec![]; // x0, x1, color, first row
        let mut emit = |run: (usize, usize, [i64; 4], usize), end: usize, out: &mut Vec<Value>| {
            out.push(serde_json::json!({
                "page": page,
                "x": xs[run.0],
                "y": ys[run.3],
                "width": xs[run.1] - xs[run.0],
                "height": ys[end] - ys[run.3],
                "color": run.2,
            }));
        };
        for (row, cells) in grid.iter().enumerate() {
            let mut runs = vec![];
            let mut at = 0;
            while at < cells.len() {
                let Some(color) = cells[at] else {
                    at += 1;
                    continue;
                };
                let start = at;
                while at < cells.len() && cells[at] == Some(color) {
                    at += 1;
                }
                runs.push((start, at, color));
            }
            let mut still_open = vec![];
            for run in open.drain(..) {
                if runs.iter().any(|&(a, b, c)| (a, b, c) == (run.0, run.1, run.2)) {
                    still_open.push(run);
                } else {
                    emit(run, row, &mut out);
                }
            }
            for (a, b, c) in runs {
                if !still_open.iter().any(|run| (run.0, run.1, run.2) == (a, b, c)) {
                    still_open.push((a, b, c, row));
                }
            }
            open = still_open;
        }
        for run in open {
            emit(run, grid.len(), &mut out);
        }
    }
    out.sort_by_key(|v| {
        (v["page"].as_i64(), v["y"].as_f64().map(|y| y.round() as i64), v["x"].as_f64().map(|x| x.round() as i64), v["color"].to_string())
    });
    out
}

fn paints_red(dump: &Value) -> bool {
    regions(dump)
        .iter()
        .any(|r| r["color"][0] == 255 && r["color"][1] == 0 && r["color"][2] == 0)
}

fn run_reftest(test: &Path) -> Result<(), String> {
    let reference = find_reference(test).ok_or("no <link rel=\"match\">")?;
    let actual = dump_layout(test)?;
    let expected = dump_layout(&reference)?;

    if paints_red(&actual) {
        return Err("paints red".into());
    }
    let (rects, ref_rects) = (regions(&actual), regions(&expected));
    let (text, ref_text) = (paint_items(&actual, "text"), paint_items(&expected, "text"));
    if rects.is_empty() && text.is_empty() && ref_rects.is_empty() && ref_text.is_empty() {
        return Err("nothing was painted, the comparison is vacuous".into());
    }
    let equal = |a: &[Value], b: &[Value]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| approx_eq(a, b))
    };
    if !equal(&rects, &ref_rects) {
        return Err(format!(
            "rectangles differ\n  test:      {}\n  reference: {}",
            Value::Array(rects),
            Value::Array(ref_rects)
        ));
    }
    if !equal(&text, &ref_text) {
        return Err(format!(
            "text differs\n  test:      {}\n  reference: {}",
            Value::Array(text),
            Value::Array(ref_text)
        ));
    }
    Ok(())
}

#[test]
fn wpt_reftests() {
    let known_failures = read_expectations("reftest-expectations.txt");

    let mut tests = Vec::new();
    collect_tests(&wpt_root(), &mut tests);
    tests.retain(|t| {
        let is_reference = t
            .file_stem()
            .and_then(|s| s.to_str())
            .map_or(false, |s| s.ends_with("-ref"));
        !is_reference && find_reference(t).is_some()
    });
    tests.sort();
    assert!(
        !tests.is_empty(),
        "no WPT reftests found in {}",
        wpt_root().display()
    );

    let mut problems = stale_expectations(&known_failures);
    for test in &tests {
        let name = test_name(test);
        match (run_reftest(test), known_failures.contains(&name)) {
            (Ok(()), false) | (Err(_), true) => {}
            (Err(why), false) => problems.push(format!("FAIL {}: {}", name, why)),
            (Ok(()), true) => problems.push(format!(
                "UNEXPECTED PASS {} (remove it from tests/wpt/reftest-expectations.txt)",
                name
            )),
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
