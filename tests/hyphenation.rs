use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn dump(name: &str, width: u32, hyphens: &str, text: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(
        &input,
        format!(
            "<style>body {{ margin: 0 }}</style><div style=\"width: {}px; margin: 0; hyphens: {}\">{}</div>",
            width, hyphens, text
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(
            dir.join("wpt-pdfs")
                .join(format!("hyphenation__{}.pdf", name)),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn count_newlines(dump: &Value) -> usize {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|paragraph| paragraph["ops"].as_array().unwrap())
        .filter(|op| op["op"] == "newline")
        .count()
}

fn count_glyphs(dump: &Value) -> usize {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|paragraph| paragraph["ops"].as_array().unwrap())
        .map(|op| match op["op"].as_str() {
            Some("glyphs") => op["glyphs"].as_array().unwrap().len(),
            Some("kerned") => op["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .sum(),
            _ => 0,
        })
        .sum()
}

#[test]
fn auto_hyphenates_english_words_and_paints_hyphens() {
    let automatic = dump("auto", 55, "auto", "representation");
    let disabled = dump("none", 55, "none", "representation");

    assert!(count_newlines(&automatic) > 0, "{automatic}");
    assert_eq!(count_newlines(&disabled), 0, "{disabled}");
    assert!(count_glyphs(&automatic) > count_glyphs(&disabled));
}

#[test]
fn manual_mode_honors_soft_hyphens() {
    let manual = dump("manual", 40, "manual", "one\u{00ad}two");
    let disabled = dump("manual-none", 40, "none", "one\u{00ad}two");

    assert!(count_newlines(&manual) > 0, "{manual}");
    assert_eq!(count_newlines(&disabled), 0, "{disabled}");
    assert!(count_glyphs(&manual) > count_glyphs(&disabled));
}

#[test]
fn the_language_chooses_the_dictionary() {
    let english = dump("lang_en", 55, "auto", r#"<span lang="en">representation</span>"#);
    let british = dump("lang_en_gb", 55, "auto", r#"<span lang="en-GB">representation</span>"#);
    let none = dump("lang_none", 55, "none", "representation");
    assert!(count_glyphs(&english) > count_glyphs(&none));
    // British English falls back to the American dictionary, which is the one embedded
    assert!(count_glyphs(&british) > count_glyphs(&none));
}

#[test]
fn text_in_a_language_without_a_dictionary_is_not_hyphenated() {
    // the language is inherited from the parent element
    let german = dump(
        "lang_de",
        55,
        "auto",
        r#"<div lang="de"><span>representation</span></div>"#,
    );
    let none = dump("lang_de_none", 55, "none", "representation");
    assert_eq!(count_glyphs(&german), count_glyphs(&none));
}
