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
