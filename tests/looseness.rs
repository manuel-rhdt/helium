//! `looseness`: the pages choose among the ways to break the paragraphs that ask for it, which have
//! a few lines more or fewer than the best breaking.
//!
//! The pages are 300px by 100px, and the text has a line height of 20px.

mod common;

use serde_json::Value;

fn dump(name: &str, style: &str, paragraphs: usize) -> Value {
    const TEXT: &str = "the quick brown fox jumps over the lazy dog and then it rests in the shade of an old tree ";
    let html = format!(
        "<!DOCTYPE html><html lang=en><title>t</title><style>@page {{ size: 300px 100px; margin: 0 }}
         body {{ margin: 0; font: 16px/20px serif }} p {{ margin: 0; {style} }}</style><p>{}</p>",
        TEXT.repeat(paragraphs)
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("looseness__{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .args(["--no-system-fonts", "--dump-layout", "-"])
        .arg(&input)
        .arg(dir.join(format!("looseness__{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The number of lines of the paragraph on every page.
fn lines(dump: &Value) -> Vec<usize> {
    let mut lines: Vec<usize> = vec![];
    for text in dump["paint"]["text"].as_array().unwrap() {
        let page = text["page"].as_u64().unwrap() as usize;
        if lines.len() <= page {
            lines.resize(page + 1, 0);
        }
        let newlines = text["ops"].as_array().unwrap().iter().filter(|op| op["op"] == "newline").count();
        lines[page] += 1 + newlines;
    }
    lines
}

#[test]
fn a_document_without_looseness_is_broken_as_well_as_it_can_be() {
    let plain = lines(&dump("plain", "", 3));
    // looseness 0 is the initial value
    assert_eq!(lines(&dump("zero", "looseness: 0", 3)), plain);
}

#[test]
fn the_pages_take_a_looser_breaking_that_fills_a_page() {
    // the best breaking has too few lines to fill the first page (a paragraph is not cut to leave
    // fewer than two lines on a page), and a breaking with a line more does
    let plain = lines(&dump("fill_plain", "", 3));
    let loose = lines(&dump("fill_loose", "looseness: 1", 3));
    assert!(loose[0] > plain[0], "{plain:?} and {loose:?}");
    assert!(loose.len() <= plain.len(), "{plain:?} and {loose:?}");
}

#[test]
fn the_ways_to_break_a_paragraph_are_the_same_every_time() {
    let first = lines(&dump("again_0", "looseness: 1", 8));
    for run in 1..4 {
        assert_eq!(lines(&dump(&format!("again_{run}"), "looseness: 1", 8)), first);
    }
}

#[test]
fn looseness_is_inherited() {
    let on_the_paragraph = lines(&dump("inherit_p", "looseness: 1", 3));
    let html = "<!DOCTYPE html><html lang=en><title>t</title><style>@page { size: 300px 100px; margin: 0 }
         body { margin: 0; font: 16px/20px serif; looseness: 1 } p { margin: 0 }</style><p>"
        .to_string()
        + &"the quick brown fox jumps over the lazy dog and then it rests in the shade of an old tree ".repeat(3)
        + "</p>";
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("looseness__inherit_body.html");
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .args(["--no-system-fonts", "--dump-layout", "-"])
        .arg(&input)
        .arg(dir.join("looseness__inherit_body.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let on_the_body = lines(&serde_json::from_slice(&output.stdout).unwrap());
    assert_eq!(on_the_body, on_the_paragraph);
}
