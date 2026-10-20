//! Runs vendored web-platform-tests reftests: a test and its `<link rel="match">` reference must
//! produce the same painted output.
//!
//! Helium has no rasterizer, so instead of comparing pixels we compare the `paint` section of
//! `helium --dump-layout`: the solid rectangles (backgrounds and borders) and the glyph runs with
//! their positions. This is independent of how the two documents are structured, which is what
//! reftests rely on. Following the WPT convention that "red means failure", a test must also not
//! paint anything pure red.

mod common;

use assert_cmd::prelude::*;
use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Maximum difference in px for two coordinates to count as equal.
const TOLERANCE: f64 = 0.5;

fn dump_layout(input: &Path) -> Result<Value, String> {
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(input)
        .arg(pdf_path(input))
        .output()
        .unwrap();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("helium failed: {}", stderr.lines().last().unwrap_or("")));
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
        (Value::Number(x), Value::Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= TOLERANCE,
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| approx_eq(a, b)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).map_or(false, |w| approx_eq(v, w)))
        }
        _ => a == b,
    }
}

/// Sorted list of the painted items in `paint[kind]`, so that the order they were created in does
/// not matter.
fn paint_items(dump: &Value, kind: &str) -> Vec<Value> {
    let mut items = dump["paint"][kind].as_array().cloned().unwrap_or_default();
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

fn paints_red(dump: &Value) -> bool {
    paint_items(dump, "rects")
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
    let (rects, ref_rects) = (paint_items(&actual, "rects"), paint_items(&expected, "rects"));
    let (text, ref_text) = (paint_items(&actual, "text"), paint_items(&expected, "text"));
    if rects.is_empty() && text.is_empty() && ref_rects.is_empty() && ref_text.is_empty() {
        return Err("nothing was painted, the comparison is vacuous".into());
    }
    let equal = |a: &[Value], b: &[Value]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| approx_eq(a, b));
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
        let is_reference = t.file_stem().and_then(|s| s.to_str()).map_or(false, |s| s.ends_with("-ref"));
        !is_reference && find_reference(t).is_some()
    });
    tests.sort();
    assert!(!tests.is_empty(), "no WPT reftests found in {}", wpt_root().display());

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
