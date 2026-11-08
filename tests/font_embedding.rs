mod common;

use assert_cmd::prelude::*;

fn render(name: &str, html: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("font_embedding_{}.html", name));
    let output = dir.join(format!("font_embedding_{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    common::helium()
        .unwrap()
        .arg(&input)
        .arg(&output)
        .assert()
        .success();
    common::expand_object_streams(&std::fs::read(output).unwrap())
}

/// Finds `/BaseFont /ABCDEF+Name` entries and returns the six-letter tags.
fn subset_tags(pdf: &[u8]) -> Vec<String> {
    let key = b"/BaseFont /";
    let mut tags = vec![];
    for i in 0..pdf.len().saturating_sub(key.len() + 7) {
        if &pdf[i..i + key.len()] == key {
            let name = &pdf[i + key.len()..i + key.len() + 7];
            if name[6] == b'+' && name[..6].iter().all(u8::is_ascii_uppercase) {
                tags.push(String::from_utf8_lossy(&name[..6]).into_owned());
            }
        }
    }
    tags
}

#[test]
fn embedded_fonts_are_subsetted_and_tagged() {
    let pdf = render("small", "<p>Hello subsetting</p>");
    // The complete system font is far larger than a document with 12 glyphs.
    assert!(pdf.len() < 100_000, "PDF is {} bytes", pdf.len());
    assert!(!subset_tags(&pdf).is_empty(), "no tagged subset font name");
}

#[test]
fn different_glyph_sets_give_different_tags() {
    let a = subset_tags(&render("tag_a", "<p>abc</p>"));
    let b = subset_tags(&render("tag_b", "<p>xyz</p>"));
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

#[test]
fn bigger_text_gives_bigger_subset() {
    let small = render("size_a", "<p>a</p>").len();
    let large = render(
        "size_b",
        "<p>The quick brown fox jumps over the lazy dog. PACK MY BOX WITH FIVE DOZEN LIQUOR JUGS 0123456789</p>",
    )
    .len();
    assert!(large > small);
}

/// Every font is embedded: when no font of `font-family` is installed, the system's sans-serif
/// font (or the font built into Helium) is used instead of a non-embedded standard PDF font.
#[test]
fn fonts_are_always_embedded() {
    let pdf = render(
        "always_embedded",
        r#"<p style="font-family: 'Helium Missing Font'">Hello</p>
           <pre>mono</pre><p style="font-family: serif"><b>serif</b></p>"#,
    );
    let pdf = String::from_utf8_lossy(&pdf);
    assert!(!pdf.contains("/Type1"), "standard font in PDF");
    let fonts = pdf.matches("/FontDescriptor").count();
    let files = pdf.matches("/FontFile2").count() + pdf.matches("/FontFile3").count();
    assert!(fonts > 0);
    assert_eq!(pdf.matches("/Type /FontDescriptor").count(), files);
}

/// Characters the font has no glyph for are taken from a fallback font, or, if no installed
/// font has one, reported by name.
#[test]
fn missing_glyphs_use_a_fallback_font_or_are_reported() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("font_embedding_fallback.html");
    let output = dir.join("font_embedding_fallback.pdf");
    std::fs::write(
        &input,
        r#"<p style="font-family: 'Helium Missing Font'">A &#x2192; B, &#x0378;</p>"#,
    )
    .unwrap();
    let result = common::helium()
        .unwrap()
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);

    // U+0378 is unassigned, so no font has it.
    assert!(stderr.contains("U+0378"), "{stderr}");
    assert!(stderr.contains("\"Helium Missing Font\""), "{stderr}");
    // The bundled font has an arrow, so it is always found.
    assert!(!stderr.contains("U+2192"), "{stderr}");
}


/// One entry of a CID font's `/W` array: the CIDs it assigns an advance to.
#[derive(Debug)]
enum WidthEntry {
    /// `c_first c_last w` — every CID in the range gets `w`.
    Range { from: u32, to: u32, advance: f64 },
    /// `c_first [w1 w2 …]` — consecutive CIDs with individual advances.
    Consecutive { start: u32, advances: Vec<f64> },
}

impl WidthEntry {
    fn cids(&self) -> std::ops::RangeInclusive<u32> {
        match self {
            WidthEntry::Range { from, to, .. } => *from..=*to,
            WidthEntry::Consecutive { start, advances } => {
                *start..=*start + advances.len() as u32 - 1
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum WidthToken {
    Number(f64),
    Array(Vec<f64>),
}

/// Parses the `/W` array of the first CID font of an expanded PDF.
fn width_entries(pdf: &[u8]) -> Vec<WidthEntry> {
    let key = b"/W [";
    let start = (0..pdf.len().saturating_sub(key.len()))
        .find(|&i| &pdf[i..i + key.len()] == key)
        .expect("no /W array in the PDF");
    let mut tokens = vec![];
    let mut numbers: Vec<f64> = vec![];
    let mut buf = String::new();
    let mut in_array = false;
    for &b in &pdf[start + key.len()..] {
        match b {
            b'[' => {
                in_array = true;
                numbers.clear();
                buf.clear();
            }
            b']' if in_array => {
                if let Ok(value) = buf.parse::<f64>() {
                    numbers.push(value);
                }
                tokens.push(WidthToken::Array(numbers.clone()));
                in_array = false;
                buf.clear();
            }
            b']' => {
                if let Ok(value) = buf.parse::<f64>() {
                    tokens.push(WidthToken::Number(value));
                }
                break;
            }
            b'0'..=b'9' | b'-' | b'.' | b'+' => buf.push(b as char),
            b' ' | b'\n' | b'\r' | b'\t' | b',' => {
                if let Ok(value) = buf.parse::<f64>() {
                    if in_array {
                        numbers.push(value);
                    } else {
                        tokens.push(WidthToken::Number(value));
                    }
                }
                buf.clear();
            }
            _ => break,
        }
    }

    let mut entries = vec![];
    let mut i = 0;
    while i < tokens.len() {
        let WidthToken::Number(cid) = tokens[i] else {
            panic!("unexpected /W token: {:?}", tokens[i]);
        };
        match tokens.get(i + 1) {
            Some(WidthToken::Array(advances)) => {
                entries.push(WidthEntry::Consecutive {
                    start: cid as u32,
                    advances: advances.clone(),
                });
                i += 2;
            }
            Some(WidthToken::Number(to)) => {
                let advance = match tokens.get(i + 2) {
                    Some(WidthToken::Number(advance)) => *advance,
                    other => panic!("truncated /W entry after CID {cid}: {other:?}"),
                };
                entries.push(WidthEntry::Range {
                    from: cid as u32,
                    to: *to as u32,
                    advance,
                });
                i += 3;
            }
            other => panic!("unexpected /W token after CID {cid}: {other:?}"),
        }
    }
    entries
}

/// A viewer advances the pen by the `/W` advance, so every entry must own the CIDs it names.
/// A compact `c_first c_last w` range that starts one CID early takes the glyph before the run
/// with it: the range wins over the per-glyph entry already written, and that glyph plus
/// everything after it in the run is drawn at the wrong x — in the reported document a capital
/// `N` collided with the letter after it.
#[test]
fn width_ranges_do_not_take_over_the_glyph_before_the_run() {
    // Compaction only happens for three consecutive glyphs of equal advance, so the documents
    // include runs that fonts of every family have: digits, and `<`, `=`, `>`.
    let documents = [
        ("width_range_digits", "<p>/0123456789</p>"),
        ("width_range_punct", "<p>&lt;=&gt; In . Nn9# 012</p>"),
        (
            "width_range_sans",
            r#"<p style="font-family: 'DejaVu Sans'">/0123456 Nn9#</p>"#,
        ),
    ];
    let mut ranges = 0;
    for (name, html) in documents {
        let pdf = render(name, html);
        let entries = width_entries(&pdf);
        assert!(!entries.is_empty(), "{name}: the PDF has no /W entries");
        ranges += entries
            .iter()
            .filter(|entry| matches!(entry, WidthEntry::Range { .. }))
            .count();

        let mut seen = std::collections::HashSet::new();
        for entry in &entries {
            for cid in entry.cids() {
                assert!(
                    seen.insert(cid),
                    "{name}: CID {cid} is given a width twice, so a glyph is drawn at the \
                     wrong advance — a compact range must start at the first CID of its run\n\
                     offending entry: {entry:?}\nall entries: {entries:?}"
                );
            }
        }
    }
    assert!(
        ranges > 0,
        "no compact /W range was emitted, so the test does not exercise the code it guards"
    );
}
