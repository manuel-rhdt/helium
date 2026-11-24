//! `hanging-punctuation` (CSS Text 3): opening quotes at the start of the first line and stops,
//! commas and hyphens at the end of a line hang outside of the line, and closing quotes at the end
//! of the last line. The font is a monospace one (TeX Gyre Cursor, 6px wide at 10px), so a line
//! of 240px holds 40 characters and what hangs makes room for one more.

mod common;

use serde_json::Value;

fn dump(name: &str, css: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("hanging_{name}.html"));
    std::fs::write(
        &input,
        format!(
            "<style>@page {{ size: 280px 400px; margin: 20px }} body {{ margin: 0; font: 10px/12px \"TeX Gyre Cursor\" }} \
             p {{ margin: 0 0 12px; {css} }}</style>{body}"
        ),
    )
    .unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("hanging_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The text of the lines of paragraph number `paragraph` (without the hyphens of hyphenated lines,
/// which are pieces of their own, and with the flag of whether the line is hyphenated).
fn lines(dump: &Value, paragraph: usize) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = vec![];
    for piece in dump["paint"]["text"][paragraph]["pieces"].as_array().unwrap() {
        let text = piece["text"].as_str().unwrap();
        if text == "\u{ad}" {
            if let Some(last) = out.last_mut() {
                last.1 = true;
            }
        } else {
            out.push((text.to_owned(), false));
        }
    }
    out
}

/// Where the first line starts: the first move of the text, from the start of its line.
fn first_offset(dump: &Value, paragraph: usize) -> f64 {
    dump["paint"]["text"][paragraph]["ops"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["op"] == "newline")
        .map_or(0.0, |op| op["x_offset"].as_f64().unwrap())
}

fn longest(lines: &[(String, bool)]) -> usize {
    lines.iter().map(|(text, _)| text.chars().count()).max().unwrap_or(0)
}

/// Words with a stop or a comma, so that three of them and the spaces are 32 characters and the
/// fourth makes 41: one more than the 40 of a line, which fits if the last one hangs.
const WORDS: &str = "aaaaaaaaa, bbbbbbbbb. ccccccccc, ddddddd. eeeeeeeee, nnnnnnnnn. ggggggggg, hhhhhhh. \
                     iiiiiiiii, jjjjjjjjj. kkkkkkkkk, mmmmmmm.";

#[test]
fn nothing_hangs_by_default() {
    let dump = dump("none", "", &format!("<p>{WORDS}</p>"));
    let lines = lines(&dump, 0);
    assert!(longest(&lines) <= 40, "{lines:?}");
    assert_eq!(first_offset(&dump, 0), 0.0);
}

#[test]
fn a_stop_or_a_comma_at_the_end_of_a_line_hangs_with_force_end() {
    let none = lines(&dump("force_none", "", &format!("<p>{WORDS}</p>")), 0);
    let dump = dump("force", "hanging-punctuation: force-end", &format!("<p>{WORDS}</p>"));
    let force = lines(&dump, 0);
    // a line has room for its 40 characters and the stop or comma that hangs after them
    assert_eq!(longest(&force), 41, "{force:?}");
    assert!(force.iter().filter(|(text, _)| text.chars().count() == 41).all(|(text, _)| text.ends_with(['.', ','])), "{force:?}");
    // (so there are fewer lines, or as many)
    assert!(force.len() <= none.len(), "{force:?} against {none:?}");
    assert_eq!(first_offset(&dump, 0), 0.0);
}

#[test]
fn allow_end_lets_it_hang_only_where_the_line_does_not_fit_otherwise() {
    // the fourth word makes 41 characters, so the stop at the end hangs to make it fit
    let allow = lines(&dump("allow", "hanging-punctuation: allow-end", &format!("<p>{WORDS}</p>")), 0);
    assert_eq!(longest(&allow), 41, "{allow:?}");
}

#[test]
fn a_line_that_fits_without_hanging_does_not_hang_with_allow_end() {
    // justified, so that what hangs shows in the spaces: the first line ends in a comma and is 36
    // characters, which fit, so `allow-end` leaves it as it is and `force-end` hangs the comma
    // and stretches the spaces to the line without it
    let body = "<p>aaaaaaaaa bbbbbbbbb ccccccccc ddddddd, eeeeeeeee nnnnnnnnn</p>";
    let ops = |name: &str, css: &str| {
        dump(name, &format!("text-align: justify; {css}"), body)["paint"]["text"][0]["ops"].clone()
    };
    let none = ops("fits_none", "");
    assert_eq!(ops("fits_allow", "hanging-punctuation: allow-end"), none);
    assert_ne!(ops("fits_force", "hanging-punctuation: force-end"), none);
}

#[test]
fn an_opening_quote_at_the_start_of_the_first_line_hangs_with_first() {
    let body = "<p>\u{201c}aaaaaaaaa bbbbbbbbb ccccccccc dddddddddd eeeeeeeee nnnnnnnnn</p><p>\u{201c}ggggggggg</p>";
    let dump = dump("first", "hanging-punctuation: first", body);
    // the quote is in the margin: 6px (its width, in this font) to the left of the text
    assert!((first_offset(&dump, 0) + 6.0).abs() < 0.01, "{}", first_offset(&dump, 0));
    assert!((first_offset(&dump, 1) + 6.0).abs() < 0.01);
    // only the first line of the paragraph: the lines after it start where the others do
    let lines = lines(&dump, 0);
    assert!(lines.len() >= 2);
    // (a line of 40 characters and the quote, which hangs, is 41)
    assert_eq!(lines[0].0.chars().count(), 41, "{lines:?}");
    let plain = self::dump("first_none", "", body);
    assert_eq!(first_offset(&plain, 0), 0.0);
}

#[test]
fn a_closing_quote_at_the_end_of_the_last_line_hangs_with_last() {
    // 10 + 1 + 10 + 1 + 10 + 1 + 7 + the quote are 41 characters, which is one too many
    let body = "<p>aaaaaaaaaa bbbbbbbbbb cccccccccc ddddddd\u{201d}</p>";
    let none = lines(&dump("last_none", "", body), 0);
    assert_eq!(none.len(), 2, "{none:?}");
    let last = lines(&dump("last", "hanging-punctuation: last", body), 0);
    assert_eq!(last.len(), 1, "{last:?}");
    // `first` and the others are not about the end of the last line
    let first = lines(&dump("last_first", "hanging-punctuation: first", body), 0);
    assert_eq!(first.len(), 2);
}

#[test]
fn the_values_combine_in_any_order_and_a_wrong_one_is_ignored() {
    let body = "<p>\u{201c}aaaaaaaaa bbbbbbbbb ccccccccc dddddddddd eeeeeeeee nnnnnnnnn</p>";
    let all = dump("all", "hanging-punctuation: last force-end first", body);
    assert!((first_offset(&all, 0) + 6.0).abs() < 0.01);
    // two of one group, a repeated keyword, an unknown one, or `none` with another one: no value
    for value in ["force-end allow-end", "first first", "sideways", "none first", "", "first,last"] {
        let dump = dump("invalid", &format!("hanging-punctuation: {value}"), body);
        assert_eq!(first_offset(&dump, 0), 0.0, "{value:?}");
    }
    // it is inherited, and `none` takes it back
    let inherited = dump("inherited", "", &format!("<div style=\"hanging-punctuation: first\">{body}</div>"));
    assert!((first_offset(&inherited, 0) + 6.0).abs() < 0.01);
    let reset = dump("reset", "hanging-punctuation: none", &format!("<div style=\"hanging-punctuation: first\">{body}</div>"));
    assert_eq!(first_offset(&reset, 0), 0.0);
}

#[test]
fn the_hyphen_of_a_hyphenated_word_hangs_with_the_end_values() {
    // a soft hyphen after the tenth letter of the fourth word: the line has 40 characters and its
    // hyphen, which is a 41st if it does not hang
    let body = "<p>aaaaaaaaa bbbbbbbbb ccccccccc dddddddddd&shy;eeeee</p>";
    let none = lines(&dump("hyphen_none", "", body), 0);
    assert_eq!(none[0], ("aaaaaaaaa bbbbbbbbb ccccccccc".to_owned(), false), "{none:?}");
    for value in ["force-end", "allow-end"] {
        let hung = lines(&dump("hyphen_hung", &format!("hanging-punctuation: {value}"), body), 0);
        assert_eq!(hung[0], ("aaaaaaaaa bbbbbbbbb ccccccccc dddddddddd".to_owned(), true), "{value}: {hung:?}");
    }
    // the other values leave the hyphen where it is
    let first = lines(&dump("hyphen_first", "hanging-punctuation: first last", body), 0);
    assert_eq!(first, none);
}
