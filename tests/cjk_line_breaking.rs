//! Line breaking and justification of text without spaces (Chinese and Japanese). The
//! assertions do not depend on the widths of the glyphs, so that they hold whichever font the
//! system falls back to for the ideographs.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

/// The ops of the painted text of a document with `body`.
fn ops(name: &str, body: &str) -> Vec<Value> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("cjk_{}.html", name));
    let html = format!(
        r#"<html><head><style>body {{ font-family: "DejaVu Sans"; }}</style></head><body>{}</body></html>"#,
        body
    );
    std::fs::write(&input, html).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("cjk_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|text| text["ops"].as_array().unwrap().clone())
        .collect()
}

/// The glyphs and kerns of every line (the lines are separated by `newline` ops).
fn lines(ops: &[Value]) -> Vec<Vec<Value>> {
    let mut lines = vec![vec![]];
    for op in ops {
        match op["op"].as_str() {
            Some("newline") => lines.push(vec![]),
            Some("kerned") => lines
                .last_mut()
                .unwrap()
                .extend(op["items"].as_array().unwrap().iter().cloned()),
            _ => {}
        }
    }
    lines.retain(|line| !line.is_empty());
    lines
}

const TEXT: &str = "道可道非常道名可名非常名無名天地之始有名萬物之母故常無欲以觀其妙";

#[test]
fn text_without_spaces_is_broken_between_ideographs() {
    let ops = ops("wrap", &format!(r#"<p style="width: 6.5em">{}</p>"#, TEXT));
    let lines = lines(&ops);
    assert!(lines.len() > 3, "the text wraps: {:?}", lines);
}

#[test]
fn justified_lines_stretch_the_gaps_between_ideographs_equally() {
    let ops = ops(
        "justify",
        &format!(
            r#"<p style="width: 6.5em; text-align: justify">{}</p>"#,
            TEXT
        ),
    );
    let lines = lines(&ops);
    assert!(lines.len() > 3, "{:?}", lines);
    for line in &lines[..lines.len() - 1] {
        let glyphs: usize = line
            .iter()
            .filter_map(Value::as_array)
            .map(|glyphs| glyphs.len())
            .sum();
        let kerns: Vec<f64> = line.iter().filter_map(Value::as_f64).collect();
        assert_eq!(kerns.len(), glyphs - 1, "a kern in every gap: {:?}", line);
        assert!(kerns[0] < 0.0, "the gaps are widened: {:?}", line);
        assert!(
            kerns.iter().all(|&kern| (kern - kerns[0]).abs() < 0.2),
            "all gaps are widened equally: {:?}",
            line
        );
    }
    // the last line is not justified
    let last = lines.last().unwrap();
    assert!(!last.iter().any(|item| item.as_f64().is_some_and(|kern| kern < -1.0)));
}
