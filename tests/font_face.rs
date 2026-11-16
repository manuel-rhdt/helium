//! `@font-face` with fonts from files next to the document, `data:` URIs and installed fonts:
//! which fonts end up embedded.

mod common;

use std::path::{Path, PathBuf};

fn work_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("font_face__{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(common::wpt_fonts().join("Ahem.ttf"), dir.join("Ahem.ttf")).unwrap();
    dir
}

/// The names of the fonts that the PDF of `html` embeds, in the directory `dir`.
fn embedded(dir: &Path, html: &str) -> Vec<String> {
    let (fonts, _) = render(dir, html);
    fonts
}

fn render(dir: &Path, html: &str) -> (Vec<String>, String) {
    let input = dir.join("index.html");
    let output = dir.join("index.pdf");
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let pdf = String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned();
    let mut fonts: Vec<String> = pdf
        .split("/BaseFont /")
        .skip(1)
        .map(|rest| rest.split(|c: char| c.is_whitespace() || c == '/').next().unwrap().to_owned())
        .map(|name| name.split('+').last().unwrap().trim_end_matches("-Identity-H").to_owned())
        .collect();
    fonts.sort();
    fonts.dedup();
    (fonts, String::from_utf8_lossy(&out.stderr).into_owned())
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn a_font_file_next_to_the_document() {
    let dir = work_dir("file");
    let html = r#"<style>@font-face { font-family: "My Ahem"; src: url(Ahem.ttf) format("truetype"); }</style>
        <p style="font-family: 'My Ahem'">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
    // family names are not case-sensitive
    let html = html.replace("'My Ahem'", "'MY ahem'");
    assert_eq!(embedded(&dir, &html), ["Ahem"]);
}

#[test]
fn a_data_uri() {
    let dir = work_dir("data");
    let data = std::fs::read(dir.join("Ahem.ttf")).unwrap();
    let html = format!(
        r#"<style>@font-face {{ font-family: Inline; src: url("data:font/ttf;base64,{}"); }}</style>
        <p style="font-family: Inline">Hello</p>"#,
        base64(&data)
    );
    assert_eq!(embedded(&dir, &html), ["Ahem"]);
}

#[test]
fn the_face_hides_installed_fonts_of_its_name() {
    let dir = work_dir("shadow");
    let plain = r#"<p style="font-family: Helvetica">Hello</p>"#;
    assert_eq!(embedded(&dir, plain), ["TeXGyreHeros-Regular"]);
    let html = format!("<style>@font-face {{ font-family: Helvetica; src: url(Ahem.ttf); }}</style>{plain}");
    assert_eq!(embedded(&dir, &html), ["Ahem"]);
}

#[test]
fn a_source_that_fails_falls_through_to_the_next() {
    let dir = work_dir("sources");
    let html = r#"<style>@font-face { font-family: X;
            src: url(missing.ttf), url(../../etc/passwd), url(https://example.com/a.ttf), local("No Such Font"), url(Ahem.ttf); }</style>
        <p style="font-family: X">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
}

#[test]
fn a_family_without_a_usable_source_is_skipped_in_the_font_family_list() {
    let dir = work_dir("failed");
    // not the installed font of that name either
    let html = r#"<style>@font-face { font-family: Helvetica; src: url(missing.ttf); }</style>
        <p style="font-family: Helvetica, Courier">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["TeXGyreCursor-Regular"]);
    // woff files cannot be read: the rule is skipped, not the document
    std::fs::write(dir.join("font.woff"), b"wOFF....").unwrap();
    let html = r#"<style>@font-face { font-family: W; src: url(font.woff) format("woff"), url(font.woff); }</style>
        <p style="font-family: W, Courier">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["TeXGyreCursor-Regular"]);
}

#[test]
fn local_fonts_by_their_postscript_name() {
    let dir = work_dir("local");
    let html = r#"<style>@font-face { font-family: Alias; src: local("TeXGyreTermes-Bold"); }</style>
        <p style="font-family: Alias">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["TeXGyreTermes-Bold"]);
}

#[test]
fn faces_are_chosen_by_weight_and_style() {
    let dir = work_dir("weights");
    let faces = r#"<style>
        @font-face { font-family: Duo; src: url(Ahem.ttf); }
        @font-face { font-family: Duo; font-weight: bold; src: local("TeXGyreHeros-Bold"); }
        @font-face { font-family: Duo; font-style: italic; src: local("TeXGyreCursor-Italic"); }
        </style>"#;
    let page = |style: &str| format!("{faces}<p style=\"font-family: Duo; {style}\">Hello</p>");
    assert_eq!(embedded(&dir, &page("")), ["Ahem"]);
    assert_eq!(embedded(&dir, &page("font-weight: bold")), ["TeXGyreHeros-Bold"]);
    assert_eq!(embedded(&dir, &page("font-weight: 600")), ["TeXGyreHeros-Bold"]);
    assert_eq!(embedded(&dir, &page("font-weight: 300")), ["Ahem"]);
    assert_eq!(embedded(&dir, &page("font-style: italic")), ["TeXGyreCursor-Italic"]);
    // no bold italic face: the style is more important than the weight
    assert_eq!(embedded(&dir, &page("font-style: italic; font-weight: bold")), ["TeXGyreCursor-Italic"]);
}

#[test]
fn a_weight_range_matches_all_of_its_weights() {
    let dir = work_dir("range");
    let html = |weight: u32| {
        format!(
            r#"<style>@font-face {{ font-family: R; font-weight: 200 800; src: url(Ahem.ttf); }}</style>
            <p style="font-family: R; font-weight: {weight}">Hello</p>"#
        )
    };
    for weight in [200, 400, 700, 800, 900, 100] {
        assert_eq!(embedded(&dir, &html(weight)), ["Ahem"], "{weight}");
    }
}

#[test]
fn the_later_of_two_equal_faces_wins() {
    let dir = work_dir("later");
    let html = r#"<style>
        @font-face { font-family: Twice; src: local("TeXGyreHeros-Regular"); }
        @font-face { font-family: Twice; src: url(Ahem.ttf); }
        </style><p style="font-family: Twice">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
}

#[test]
fn media_blocks_and_invalid_rules() {
    let dir = work_dir("media");
    let html = |media: &str| {
        format!(
            r#"<style>@media {media} {{ @font-face {{ font-family: M; src: url(Ahem.ttf); }} }}
            @font-face {{ font-family: Broken }}
            @font-face {{ src: url(Ahem.ttf) }}</style>
            <p style="font-family: M, Courier">Hello</p>"#
        )
    };
    assert_eq!(embedded(&dir, &html("print")), ["Ahem"]);
    assert_eq!(embedded(&dir, &html("screen")), ["TeXGyreCursor-Regular"]);
}

#[test]
fn urls_are_relative_to_the_style_sheet() {
    let dir = work_dir("linked");
    std::fs::create_dir_all(dir.join("css")).unwrap();
    std::fs::copy(dir.join("Ahem.ttf"), dir.join("css/Sheet.ttf")).unwrap();
    std::fs::write(
        dir.join("css/style.css"),
        "@font-face { font-family: Linked; src: url(Sheet.ttf); }",
    )
    .unwrap();
    let html = r#"<link rel="stylesheet" href="css/style.css">
        <p style="font-family: Linked">Hello</p>"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
}

#[test]
fn different_fonts_with_the_same_name_are_both_embedded() {
    let dir = work_dir("same_name");
    let mut data = std::fs::read(dir.join("Ahem.ttf")).unwrap();
    // the last byte belongs to the data of a table that is not read
    *data.last_mut().unwrap() ^= 0xFF;
    std::fs::write(dir.join("Ahem2.ttf"), &data).unwrap();
    let html = r#"<style>
        @font-face { font-family: One; src: url(Ahem.ttf); }
        @font-face { font-family: Two; src: url(Ahem2.ttf); }
        @font-face { font-family: Three; src: url(Ahem.ttf); }
        </style>
        <p style="font-family: One">a</p><p style="font-family: Two">b</p><p style="font-family: Three">c</p>"#;
    let fonts = embedded(&dir, html);
    // (the same file twice is one font)
    assert_eq!(fonts.len(), 2, "{fonts:?}");
    assert!(fonts.iter().all(|name| name.starts_with("Ahem")), "{fonts:?}");
}

#[test]
fn the_text_of_inline_svg_uses_the_faces_too() {
    let dir = work_dir("svg");
    let html = r#"<style>@font-face { font-family: Vector; src: url(Ahem.ttf); }</style>
        <svg width="200" height="50"><text x="5" y="30" font-family="Vector" font-size="20">Hello</text></svg>"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
}

#[test]
fn the_text_of_img_svg_uses_the_faces_too() {
    let dir = work_dir("img_svg");
    std::fs::write(
        dir.join("t.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="5" y="30" font-family="Vector" font-size="20">Hello</text></svg>"#,
    )
    .unwrap();
    let html = r#"<style>@font-face { font-family: Vector; src: url(Ahem.ttf); }</style><img src="t.svg">"#;
    assert_eq!(embedded(&dir, html), ["Ahem"]);
}

#[test]
fn the_characters_of_a_face_without_glyphs_use_fallback_fonts() {
    let dir = work_dir("fallback");
    // Ahem has no Greek letters, but has glyphs for the Latin ones
    let html = r#"<style>@font-face { font-family: F; src: url(Ahem.ttf); }</style>
        <p style="font-family: F">abc α</p>"#;
    let fonts = embedded(&dir, html);
    assert!(fonts.contains(&"Ahem".to_owned()) && fonts.len() > 1, "{fonts:?}");
}
