//! Hyphenation limits: `hyphenate-limit-chars` and `hyphenate-limit-last`, tested through the
//! layout dump.

mod common;

use serde_json::Value;

const TEXT: &str = "Extraordinarily characteristic misunderstandings of internationalization \
    notwithstanding, representatives acknowledged unconditional responsibilities concerning \
    incomprehensible administrative classifications throughout approximately seventeen \
    unfortunately inconsequential recommendations regarding architecture";

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("hyphenation_limits_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("hyphenation_limits_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The lines of the first paragraph, as the text of the pieces of every line (the text of a line is
/// what is left when the lines are put together again: a line that ends in the middle of a word
/// has no space after it).
fn lines(dump: &Value) -> Vec<String> {
    dump["paint"]["text"][0]["pieces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|piece| piece["text"].as_str().unwrap().to_owned())
        // (the hyphen of a hyphenated line is a piece of its own: a soft hyphen)
        .filter(|text| text != "\u{ad}")
        .collect()
}

/// The words that were broken, as `(characters in the word, before the hyphen, after it)`.
fn breaks(lines: &[String]) -> Vec<(usize, usize, usize)> {
    let joined: Vec<char> = TEXT.chars().collect();
    let mut position = 0;
    let mut out = vec![];
    for line in &lines[..lines.len() - 1] {
        position += line.chars().count();
        if joined.get(position) == Some(&' ') {
            position += 1;
            continue;
        }
        // in the middle of a word
        let start = joined[..position].iter().rposition(|&ch| ch == ' ').map_or(0, |index| index + 1);
        let end = joined[position..].iter().position(|&ch| ch == ' ').map_or(joined.len(), |index| position + index);
        out.push((end - start, position - start, end - position));
    }
    out
}

fn page(css: &str) -> String {
    format!(
        "<style>@page {{ size: 260px 600px; margin: 20px }} body {{ margin: 0; font: 14px/18px \"DejaVu Sans\"; \
         hyphens: auto }} {css}</style><p lang=\"en\" style=\"margin: 0\">{TEXT}</p>"
    )
}

#[test]
fn hyphenation_limits_are_five_two_and_two_by_default() {
    let broken = breaks(&lines(&dump("default", &page(""))));
    assert!(broken.len() >= 2, "{broken:?}");
    assert!(broken.iter().all(|&(word, before, after)| word >= 5 && before >= 2 && after >= 2), "{broken:?}");
    // (the default is about the smallest that the limits allow: some break is that near the end)
    assert!(broken.iter().any(|&(_, before, after)| before == 2 || after == 2 || before == 3 || after == 3), "{broken:?}");
}

#[test]
fn hyphenate_limit_chars_sets_the_word_and_the_characters_before_and_after() {
    let broken = breaks(&lines(&dump("limits", &page("p { hyphenate-limit-chars: 12 5 4 }"))));
    assert!(!broken.is_empty());
    assert!(broken.iter().all(|&(word, before, after)| word >= 12 && before >= 5 && after >= 4), "{broken:?}");
}

#[test]
fn a_missing_third_number_is_the_second_and_auto_is_the_default() {
    // "auto 4": 4 before and after, and the word as long as it is by default
    let broken = breaks(&lines(&dump("two", &page("p { hyphenate-limit-chars: auto 4 }"))));
    assert!(broken.iter().all(|&(word, before, after)| word >= 5 && before >= 4 && after >= 4), "{broken:?}");
    let broken = breaks(&lines(&dump("one", &page("p { hyphenate-limit-chars: 10 }"))));
    assert!(broken.iter().all(|&(word, before, after)| word >= 10 && before >= 2 && after >= 2), "{broken:?}");
}

#[test]
fn the_limits_are_inherited_and_invalid_values_are_ignored() {
    let broken = breaks(&lines(&dump("inherited", &page("body { hyphenate-limit-chars: 12 5 4 }"))));
    assert!(broken.iter().all(|&(word, before, after)| word >= 12 && before >= 5 && after >= 4), "{broken:?}");
    // a value that is not valid leaves the default (so words of 5 letters can be broken)
    for value in ["1 2 3 4", "-1", "foo", "3.5"] {
        let broken = breaks(&lines(&dump("invalid", &page(&format!("p {{ hyphenate-limit-chars: {value} }}")))));
        assert!(broken.iter().any(|&(_, before, after)| before < 5 || after < 4), "{value}: {broken:?}");
    }
}

/// How many of the paragraphs of the dump have a hyphen at the end of the line before the last
/// (and how many have at least three lines, which is where a paragraph can have one).
fn next_to_last_hyphens(dump: &Value) -> (usize, usize) {
    let mut hyphenated = 0;
    let mut counted = 0;
    for paragraph in dump["paint"]["text"].as_array().unwrap() {
        let pieces: Vec<&str> =
            paragraph["pieces"].as_array().unwrap().iter().map(|piece| piece["text"].as_str().unwrap()).collect();
        // the lines, each with whether a soft hyphen piece follows it
        let mut lines: Vec<bool> = vec![];
        for piece in pieces {
            if piece == "\u{ad}" {
                if let Some(last) = lines.last_mut() {
                    *last = true;
                }
            } else {
                lines.push(false);
            }
        }
        if lines.len() >= 3 {
            counted += 1;
            if lines[lines.len() - 2] {
                hyphenated += 1;
            }
        }
    }
    (hyphenated, counted)
}

/// The first paragraphs of the novel of the gallery, set as the book sets them: justified, in
/// the measure of a page of a trade paperback.
fn book_paragraphs(css: &str, count: usize) -> String {
    book_pages(css, count, "80in")
}

/// The same on pages of the height `height`.
fn book_pages(css: &str, count: usize, height: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("gallery/book/index.html");
    let book = std::fs::read_to_string(path).unwrap();
    let mut body = String::new();
    let mut found = 0;
    for chunk in book.split("<p>").skip(1) {
        let Some(end) = chunk.find("</p>") else { continue };
        let text: String = chunk[..end].replace('\n', " ");
        // (plain paragraphs of some length: no markup, and none that is a line of dialogue)
        if text.len() < 300 || text.contains('<') {
            continue;
        }
        body.push_str(&format!("<p>{text}</p>"));
        found += 1;
        if found == count {
            break;
        }
    }
    format!(
        "<html lang=\"en\"><style>@page {{ size: 5.5in {height}; margin: 0.8in 0.7in }} \
         body {{ margin: 0; font: 10.5pt/14.5pt \"TeX Gyre Pagella\", serif; {css} }} \
         p {{ margin: 0; text-align: justify; text-indent: 1.3em; hyphens: auto }}</style>{body}</html>"
    )
}

#[test]
fn a_hyphen_before_the_last_line_is_discouraged_and_always_avoids_it() {
    // 400 paragraphs of the novel in the measure of the book (about 62 characters to the line)
    let default = next_to_last_hyphens(&dump("final_default", &book_paragraphs("", 400)));
    let always = next_to_last_hyphens(&dump("final_always", &book_paragraphs("hyphenate-limit-last: always", 400)));
    assert!(default.1 >= 300, "{default:?}");
    // TeX's `\finalhyphendemerits` only discourages it, so some paragraphs have it
    assert!(default.0 >= 10, "{default:?}");
    // with `always` it is avoided unless every other way is much worse (the specification says
    // that the line "should not" be hyphenated)
    assert!(always.0 * 4 <= default.0, "always {always:?} against default {default:?}");
}

#[test]
fn hyphenate_limit_last_is_inherited_and_none_is_the_default() {
    let plain = next_to_last_hyphens(&dump("final_plain", &book_paragraphs("", 120)));
    assert_eq!(next_to_last_hyphens(&dump("final_none", &book_paragraphs("hyphenate-limit-last: none", 120))), plain);
    // set on the body, the paragraphs have it
    let always = next_to_last_hyphens(&dump("final_inherit", &book_paragraphs("hyphenate-limit-last: always", 120)));
    assert!(always.0 < plain.0 || plain.0 == 0, "{always:?} {plain:?}");
    // a value that is not valid leaves the default
    for value in ["sometimes", "page spread", "1"] {
        let invalid = next_to_last_hyphens(&dump("final_invalid", &book_paragraphs(&format!("hyphenate-limit-last: {value}"), 120)));
        assert_eq!(invalid, plain, "{value}");
    }
}

/// `(pages, pages whose last line ends in a hyphen)`: the last piece of the last paragraph that
/// is drawn on a page is the hyphen of a hyphenated line, which is a piece of its own.
fn page_end_hyphens(dump: &Value) -> (usize, usize) {
    let mut last: std::collections::BTreeMap<u64, bool> = std::collections::BTreeMap::new();
    for paragraph in dump["paint"]["text"].as_array().unwrap() {
        let page = paragraph["page"].as_u64().unwrap();
        let pieces = paragraph["pieces"].as_array().unwrap();
        last.insert(page, pieces.last().is_some_and(|piece| piece["text"] == "\u{ad}"));
    }
    (last.len(), last.values().filter(|&&hyphenated| hyphenated).count())
}

#[test]
fn a_page_does_not_end_in_a_hyphen_if_hyphenate_limit_last_says_so() {
    // pages of 8 lines or so, with a paragraph of the novel after another
    let pages = |css: &str| page_end_hyphens(&dump("page_ends", &book_pages(css, 60, "3.4in")));
    let (count, default) = pages("");
    assert!(count >= 20, "{count} pages");
    // (a page ends in a hyphen where the page is full at that line, which is about one in nine)
    assert!(default >= 3, "{default} of {count}");
    let (_, none) = pages("hyphenate-limit-last: none");
    assert_eq!(none, default);
    // the page, the column and `always` are about every page
    for value in ["page", "column", "always"] {
        let (pages_with, with) = pages(&format!("hyphenate-limit-last: {value}"));
        assert!(with * 3 <= default, "{value}: {with} of {pages_with} pages against {default} of {count}");
    }
    // the spread is about the right pages, which are half of the pages
    let (_, spread) = pages("hyphenate-limit-last: spread");
    assert!(spread < default, "spread {spread} of {default}");
}
