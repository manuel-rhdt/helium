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
    let output = common::umbruch()
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
    let output = common::umbruch()
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

/// The repro of a bug report: the first paragraph took a way to break it with a line more, but
/// the box that it is in was taken from Taffy's cache with the height of the best breaking, so
/// the second paragraph started on the last line of the first, and without its indent.
#[test]
fn a_paragraph_that_takes_a_looser_breaking_pushes_down_what_follows_it() {
    let html = "<!DOCTYPE html><meta charset=utf-8><style>
  @page { size: 453.5px 520px; margin: 56.7px }
  body { margin: 0; font: 14px/19.33px serif; orphans: 2; widows: 2 }
  .gap { height: 197px }
  p { margin: 0; text-align: justify; hyphens: auto; text-indent: 1.3em; looseness: 1 }
</style>
<div class=gap></div>
<p>My dear Mr Bennet, said his lady to him one day, have you heard that Netherfield Park is let at last? Mr Bennet replied that he had not. But it is, returned she, for Mrs Long has just been here, and she told me all about it.</p>
<p>It is a truth universally acknowledged, that a single man in possession of a good fortune must be in want of a wife. However little known the feelings or views of such a man may be on his first entering a neighbourhood, this truth is so well fixed in the minds of the surrounding families, that he is considered as the rightful property of some one or other of their daughters.</p>";
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("looseness__pushes.html");
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .args(["--no-system-fonts", "--dump-layout", "-"])
        .arg(&input)
        .arg(dir.join("looseness__pushes.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
    // (the paragraphs of the first page, in the order of the document: the text, and its lines)
    let texts: Vec<&Value> = dump["paint"]["text"].as_array().unwrap().iter().filter(|text| text["page"] == 0).collect();
    let (first, second) = (texts[0], texts[1]);
    let newlines = first["ops"].as_array().unwrap().iter().filter(|op| op["op"] == "newline").count();
    let end_of_first = first["y"].as_f64().unwrap() + (newlines + 1) as f64 * 19.33;
    assert!(
        second["y"].as_f64().unwrap() >= end_of_first - 0.01,
        "the second paragraph starts at {}, and the first ends at {end_of_first}",
        second["y"]
    );
}

#[test]
fn the_choices_of_the_pages_can_be_dumped() {
    let html = "<!DOCTYPE html><html lang=en><title>t</title><style>@page { size: 300px 100px; margin: 0 }
         body { margin: 0; font: 16px/20px serif; looseness: 1 } p { margin: 0 }</style><p>"
        .to_string()
        + &"the quick brown fox jumps over the lazy dog and then it rests in the shade of an old tree ".repeat(3)
        + "</p>";
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("looseness__dump.html");
    std::fs::write(&input, html).unwrap();
    let report = dir.join("looseness__dump.json");
    let output = common::umbruch()
        .unwrap()
        .args(["--no-system-fonts", "--dump-pagination"])
        .arg(&report)
        .arg(&input)
        .arg(dir.join("looseness__dump.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let pages = report["pages"].as_array().unwrap();
    assert!(pages.len() >= 2);
    assert_eq!(report["tuning"]["looseness"], 1);
    for page in pages {
        assert!(page["room"].as_f64().unwrap() >= 0.0);
        assert_eq!(page["cost"]["pages"], 1);
        let paragraphs = page["paragraphs"].as_array().unwrap();
        assert!(!paragraphs.is_empty());
        // the lines of the paragraph that the page holds, each with its badness
        for paragraph in paragraphs {
            assert_eq!(paragraph["badness"].as_array().unwrap().len() as u64, paragraph["lines"].as_u64().unwrap());
        }
    }
    // the lines of the pages are the lines of the document
    let reported: u64 = pages
        .iter()
        .flat_map(|page| page["paragraphs"].as_array().unwrap())
        .map(|paragraph| paragraph["lines"].as_u64().unwrap())
        .sum();
    assert_eq!(report["lines"]["count"].as_u64().unwrap(), reported);
}


/// A container with two columns whose content has a way to break it with a line fewer: the
/// balanced columns are a line shorter when its paragraphs ask for `looseness`, and the text is
/// the same (found with `tools/looseness_bench.py`, the kind `balanced`).
#[test]
fn balanced_columns_are_shorter_when_their_paragraphs_can_be_broken_in_another_way() {
    const CONTAINER: &str = r#"<div class=cols style="column-count: 2"><p>From they many when was made understanding so with unfortunately some find has see first like. About each has its disappointment like so extraordinary so with what understanding extraordinary is by are what than as. About other and notwithstanding people has my your notwithstanding. Water what do know to long were out which do. This as his into we they or if to.</p><p>Were no not than if about than circumstances made of way may they has if time but so over can circumstances from. Two so in can she called people way are at out other did would his at it has how where.</p><p>It has extraordinary acquaintance then where. Down like where long has for his from there would water would long with only an very how acquaintance. Up disappointment more made it only of on it for this way. The from way an an how. Unfortunately there disappointment then one no extraordinary will said first over said made him on are.</p><p>She over notwithstanding only people words make out would way find. But she it neighbourhood my with my called with unfortunately what are was a had make neighbourhood. Could about if time more been a some these than like may like just will an extraordinary be so. Is as see her do up after very all some of its other some up been about an where. My more more than his each is there.</p><p>We have if but up will after. Where notwithstanding or can these into people unfortunately has or him we at more. Know these do unfortunately what as his your these how. Was is as has there disappointment we words over each could not we or unfortunately what have your called.</p><p>Each over their had a was then the about out no neighbourhood time out where. But most had were on did acquaintance this people one a would. When may which had by see just did if of on notwithstanding as we way see them just. After out up her had so know way may about was up was. We out make it they would was these like them each at from if in called called on when than. Many some not be has been who no how each have people circumstances no many.</p></div>"#;
    let height_of = |name: &str, looseness: u32| -> (f64, usize) {
        let html = format!(
            "<!DOCTYPE html><html lang=en><meta charset=utf-8><title>t</title><style>
             @page {{ size: 800px 4000px; margin: 40px }}
             body {{ margin: 0; font: 14px/20px serif; orphans: 2; widows: 2; looseness: {looseness} }}
             p {{ margin: 0; text-align: justify; hyphens: auto; text-indent: 1.3em }}
             .cols {{ margin: 0 0 20px; column-gap: 24px }}</style>{CONTAINER}"
        );
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
        let input = dir.join(format!("looseness__{name}.html"));
        std::fs::write(&input, html).unwrap();
        let output = common::umbruch()
            .unwrap()
            .args(["--no-system-fonts", "--dump-layout", "-"])
            .arg(&input)
            .arg(dir.join(format!("looseness__{name}.pdf")))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let dump: Value = serde_json::from_slice(&output.stdout).unwrap();
        fn find(node: &Value) -> Option<f64> {
            if node["class"].as_str().is_some_and(|class| class.split(' ').any(|name| name == "cols")) {
                return node["border_box"]["height"].as_f64();
            }
            node["children"].as_array()?.iter().find_map(find)
        }
        // the lines that are in the columns
        let lines: usize = dump["paint"]["text"]
            .as_array()
            .unwrap()
            .iter()
            .map(|text| 1 + text["ops"].as_array().unwrap().iter().filter(|op| op["op"] == "newline").count())
            .sum();
        (find(&dump["document"]).expect("the container"), lines)
    };
    let (plain, plain_lines) = height_of("columns_plain", 0);
    let (loose, loose_lines) = height_of("columns_loose", 1);
    assert_eq!(plain - loose, 20.0, "{plain} and {loose}");
    // (a line fewer in all)
    assert!(loose_lines < plain_lines, "{plain_lines} and {loose_lines}");
}
