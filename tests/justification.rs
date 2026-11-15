//! `text-align: justify`: which lines are stretched.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("justification__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("justification__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The most negative adjustment between the glyphs of each line of the first text object, negated.
/// The space of a stretched line is wider than the one of a line that is not, which shows as a
/// negative adjustment; kerning mostly tightens pairs of letters.
fn stretch_per_line(dump: &Value) -> Vec<f64> {
    let mut lines = vec![0.0f64];
    for op in dump["paint"]["text"][0]["ops"].as_array().unwrap() {
        match op["op"].as_str().unwrap() {
            "newline" => lines.push(0.0),
            "kerned" => {
                for item in op["items"].as_array().unwrap() {
                    if let Some(adjustment) = item.as_f64() {
                        let line = lines.last_mut().unwrap();
                        *line = line.max(-adjustment);
                    }
                }
            }
            _ => {}
        }
    }
    lines
}

const TEXT: &str = "gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron";

#[test]
fn lines_are_stretched_except_the_last_and_those_before_a_forced_break() {
    let dump = dump(
        "forced",
        &format!(
            r#"<p style="width: 300px; text-align: justify">alpha beta<br>{}</p>"#,
            TEXT
        ),
    );
    let stretch = stretch_per_line(&dump);
    assert!(stretch.len() >= 3, "{:?}", stretch);
    // the line in front of the line feed and the last line are not stretched, those in between are
    assert!(stretch[0] < 20.0, "{:?}", stretch);
    assert!(stretch[1] > 20.0, "{:?}", stretch);
    assert!(stretch[stretch.len() - 1] < 20.0, "{:?}", stretch);
}

#[test]
fn a_line_break_at_the_end_of_a_paragraph_does_not_stretch_the_line() {
    let dump = dump(
        "trailing_br",
        r#"<p style="width: 300px; text-align: justify">“Fitzwilliam Darcy.”<br></p>"#,
    );
    assert!(stretch_per_line(&dump)[0] < 20.0);
}

/// The sum of the adjustments (thousandths of an em) that move a glyph to the left: kerning and
/// the narrower spaces of a line whose spaces are shrunk.
fn tightening(dump: &Value) -> f64 {
    dump["paint"]["text"][0]["ops"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["op"] == "kerned")
        .flat_map(|op| op["items"].as_array().unwrap())
        .filter_map(Value::as_f64)
        .filter(|adjustment| *adjustment > 0.0)
        .sum()
}

/// The width of the `<span>` in the dump of a layout, in CSS pixels.
fn span_width(node: &Value) -> Option<f64> {
    if node["tag"] == "span" {
        return node["border_box"]["width"].as_f64();
    }
    node["children"].as_array()?.iter().find_map(span_width)
}

#[test]
fn a_last_line_that_fits_only_with_shrunken_spaces_is_shrunk() {
    const SENTENCE: &str = "“And which of the two do you call my little recent piece of modesty?”";
    let probe = dump(
        "shrink_probe",
        &format!(r#"<span style="display: inline-block; white-space: nowrap">{SENTENCE}</span>"#),
    );
    let natural = span_width(&probe["document"]).unwrap();
    // 6 px narrower than the text: the line breaking lets the spaces shrink to make it one line
    let shrink = 6.0;
    let html = |width: f64| {
        format!(r#"<p style="width: {width}px; text-align: justify; margin: 0">{SENTENCE}</p>"#)
    };
    let fits = dump("shrink_fits", &html(natural + 100.0));
    let tight = dump("shrink_tight", &html(natural - shrink));
    assert_eq!(stretch_per_line(&tight).len(), 1, "one line");
    // the 6 px are 6 / 16 em, or 375 thousandths, which the spaces give up
    let given_up = tightening(&tight) - tightening(&fits);
    assert!((given_up - shrink / 16.0 * 1000.0).abs() < 20.0, "{given_up}");
}

/// The mean adjustment (thousandths of an em) of every line of a paragraph that is stretched or
/// shrunk, over all pages: the same for lines with the same content.
fn mean_adjustments(html: &str, name: &str) -> Vec<f64> {
    let dump = dump(name, html);
    let mut means = vec![];
    for text in dump["paint"]["text"].as_array().unwrap() {
        let mut lines = vec![vec![]];
        for op in text["ops"].as_array().unwrap() {
            match op["op"].as_str().unwrap() {
                "newline" => lines.push(vec![]),
                "kerned" => lines
                    .last_mut()
                    .unwrap()
                    .extend(op["items"].as_array().unwrap().iter().filter_map(Value::as_f64)),
                _ => {}
            }
        }
        means.extend(lines.iter().filter(|line| !line.is_empty()).map(|line| line.iter().sum::<f64>() / line.len() as f64));
    }
    means
}

#[test]
fn the_end_of_a_very_long_paragraph_is_justified_like_its_start() {
    // Every full line holds the same words, so every line takes the same stretch. The positions
    // along a paragraph reach tens of thousands of pixels, where summing the glyph widths in
    // single precision drifted by pixels: the lines at the end were measured wrongly and
    // overflowed or fell short.
    let text = vec!["alpha"; 4000].join(" ");
    let means = mean_adjustments(
        &format!(r#"<p style="width: 300px; text-align: justify; margin: 0">{text}</p>"#),
        "long",
    );
    assert!(means.len() > 400, "{}", means.len());
    // without the first and the last line
    let full = &means[1..means.len() - 1];
    let (low, high) = full.iter().fold((f64::MAX, f64::MIN), |(low, high), x| (low.min(*x), high.max(*x)));
    assert!(high - low < 1.0, "the adjustments of the lines range from {low} to {high}");
}
