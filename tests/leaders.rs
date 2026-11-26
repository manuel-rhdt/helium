//! `leader()` in `content` (CSS Generated Content 3, 2.5): a string that is repeated over what
//! is left of the line, tested through the layout dump. The font is a monospace one (TeX Gyre
//! Cursor, 0.6 em wide), so that the widths are known: 6px at 10px.

mod common;

use serde_json::Value;

/// A page of 300px with margins of 20px: a line is 260px wide.
const PAGE: &str = "@page { size: 300px 200px; margin: 20px } body { margin: 0; font: 10px/12px \"TeX Gyre Cursor\" } \
    p { margin: 0 }";

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("leader_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("leader_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// What the leader of a paragraph looks like.
#[derive(Debug, PartialEq)]
struct Leader {
    /// The text that was drawn for it.
    text: String,
    /// Where it starts and where the text after it starts, from the start of the line.
    start: f64,
    end: f64,
}

/// The paragraphs (of one line each, unless they wrap) of the dump: the text of every piece, and
/// the leader of a paragraph, if it has one.
fn paragraphs(dump: &Value) -> Vec<(Vec<String>, Option<Leader>)> {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|paragraph| {
            let pieces: Vec<String> = paragraph["pieces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|piece| piece["text"].as_str().unwrap().to_owned())
                .collect();
            let ops = paragraph["ops"].as_array().unwrap();
            let leader = ops.iter().position(|op| op["op"] == "artifact").map(|artifact| {
                let start = ops[artifact + 1]["x_offset"].as_f64().unwrap();
                let after = ops[artifact + 2..].iter().find(|op| op["op"] == "newline").unwrap();
                // the pieces: the text in front of the leader, the leader and the text after it
                let text = pieces.iter().find(|piece| piece.chars().all(|ch| ".,_ab".contains(ch))).cloned().unwrap_or_default();
                Leader { text, start, end: start + after["x_offset"].as_f64().unwrap() }
            });
            (pieces, leader)
        })
        .collect()
}

fn page(css: &str, body: &str) -> String {
    format!("<style>{PAGE} {css}</style>{body}")
}

#[test]
fn a_leader_fills_the_line_between_the_text_before_and_after_it() {
    let dump = dump(
        "fills",
        &page("a::after { content: leader('.') '12' } a { display: block }", "<a>Chapter one</a>"),
    );
    let (pieces, leader) = &paragraphs(&dump)[0];
    let leader = leader.as_ref().expect("a leader");
    // 11 letters (66px) and the number (12px) leave 182px: 30 dots of 6px on the grid from the
    // start of the line, and the number ends at the end of the line
    assert_eq!(leader.text, ".".repeat(30), "{pieces:?}");
    assert!((leader.start - 66.0).abs() < 0.01 && (leader.end - 248.0).abs() < 0.01, "{leader:?}");
    assert_eq!(pieces.last().unwrap(), "12");
}

#[test]
fn the_pattern_is_on_a_grid_that_starts_at_the_start_of_the_line() {
    // "x" is 6px wide and the pattern 12px: the first one goes where the grid has room, so that
    // the patterns of the lines that follow each other line up
    let dump = dump(
        "grid",
        &page("a::after { content: leader('ab') '12' } a { display: block }", "<a>x</a><a>xxxxxxx</a>"),
    );
    let paragraphs = paragraphs(&dump);
    let (first, second) = (paragraphs[0].1.as_ref().unwrap(), paragraphs[1].1.as_ref().unwrap());
    assert!((first.start - 12.0).abs() < 0.01, "{first:?}");
    // 7 letters are 42px: the grid has the next place at 48px
    assert!((second.start - 48.0).abs() < 0.01, "{second:?}");
    assert!((first.end - 248.0).abs() < 0.01 && (second.end - 248.0).abs() < 0.01);
    assert!(first.text.starts_with("abab") && first.text.len() % 2 == 0, "{first:?}");
}

#[test]
fn the_types_are_strings() {
    for (name, kind, text) in [("dotted", "dotted", "."), ("solid", "solid", "_"), ("space", "space", " "), ("string", "'-='", "-=")] {
        let dump = dump(
            &format!("type_{name}"),
            &page(&format!("a::after {{ content: leader({kind}) '1' }} a {{ display: block }}"), "<a>t</a>"),
        );
        let (pieces, _) = &paragraphs(&dump)[0];
        let drawn: String = pieces.iter().skip(1).take(pieces.len().saturating_sub(2)).cloned().collect();
        if name == "space" {
            // (nothing is drawn for a space: it has no glyph to show)
            continue;
        }
        assert!(drawn.starts_with(text) && drawn.len() > 20, "{name}: {pieces:?}");
    }
}

#[test]
fn a_line_with_a_leader_is_as_wide_as_the_paragraph_whatever_the_alignment() {
    for align in ["left", "right", "center", "justify"] {
        let dump = dump(
            &format!("align_{align}"),
            &page(&format!("a::after {{ content: leader('.') '12' }} a {{ display: block; text-align: {align} }}"), "<a>Chapter one</a>"),
        );
        let leader = paragraphs(&dump)[0].1.take().unwrap();
        assert!((leader.end - 248.0).abs() < 0.01, "{align}: {leader:?}");
    }
}

#[test]
fn a_justified_line_gives_its_room_to_the_leader_and_not_to_its_spaces() {
    let dump = dump(
        "justify",
        &page("a::after { content: leader('.') '12' } a { display: block; text-align: justify }", "<a>a b c</a>"),
    );
    let paragraph = &dump["paint"]["text"][0];
    let text: String = paragraph["pieces"].as_array().unwrap().iter().map(|piece| piece["text"].as_str().unwrap()).collect();
    assert!(text.starts_with("a b c."), "{text}");
    // "a b c" is 5 letters and spaces (30px) and the number 12px: the leader has the 218px that
    // are left (on the grid from 30px); the spaces are not stretched, or the line would be wider
    let leader = paragraphs(&dump)[0].1.take().unwrap();
    assert!((leader.start - 30.0).abs() < 0.01 && (leader.end - 248.0).abs() < 0.01, "{leader:?}");
}

#[test]
fn a_long_entry_wraps_and_the_leader_is_on_the_last_line() {
    let dump = dump(
        "wraps",
        &page(
            "a::after { content: leader('.') '12' } a { display: block }",
            "<a>one two three four five six seven eight nine ten eleven twelve thirteen</a>",
        ),
    );
    let paragraph = &dump["paint"]["text"][0];
    let artifacts = paragraph["ops"].as_array().unwrap().iter().filter(|op| op["op"] == "artifact").count();
    assert_eq!(artifacts, 1, "{paragraph}");
    let (_, leader) = &paragraphs(&dump)[0];
    assert!((leader.as_ref().unwrap().end + 12.0 - 260.0).abs() < 6.0);
}

#[test]
fn a_leader_that_is_not_valid_leaves_the_content_alone() {
    for value in ["leader()", "leader(foo)", "leader('')", "leader(dotted, solid)"] {
        let dump = dump(
            &format!("invalid_{}", value.replace(|ch: char| !ch.is_alphanumeric(), "_")),
            &page(&format!("a::after {{ content: {value} '12' }} a {{ content: none; display: block }}"), "<a>x</a>"),
        );
        let paragraphs = paragraphs(&dump);
        assert!(paragraphs.iter().all(|(_, leader)| leader.is_none()), "{value}: {paragraphs:?}");
    }
}

#[test]
fn a_leader_inside_a_table_of_contents_resolves_the_page_numbers_of_its_targets() {
    // the leader and `target-counter()` together, the way a table of contents uses them
    let dump = dump(
        "toc",
        &page(
            "a::after { content: leader('.') target-counter(attr(href), page) } a { display: block } h1 { break-before: page }",
            "<nav><a href=\"#c\">Chapter</a></nav><h1 id=\"c\">C</h1>",
        ),
    );
    let (pieces, leader) = &paragraphs(&dump)[0];
    assert_eq!(pieces.last().unwrap(), "2");
    assert!(leader.is_some());
}
