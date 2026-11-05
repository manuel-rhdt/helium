mod common;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

#[test]
fn file_doesnt_exist() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
    cmd.args(&["test/file/doesnt/exist", "never.pdf"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn converts_loremipsum_to_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/documents/loremipsum.html"
    );
    let output = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("loremipsum.pdf");
    // make sure a stale file from an earlier run cannot make the test pass
    if output.exists() {
        std::fs::remove_file(&output)?;
    }

    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
    cmd.arg(input).arg(&output);
    cmd.assert().success();

    let pdf = std::fs::read(&output)?;
    assert!(
        pdf.starts_with(b"%PDF-"),
        "output does not start with a PDF header"
    );
    assert!(
        pdf.windows(5).rev().take(1024).any(|w| w == b"%%EOF"),
        "output has no %%EOF trailer"
    );

    Ok(())
}

/// Reads the page count from the PDF's page tree and checks it against the number of
/// page objects.
fn pdf_page_count(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    let count_pos = text.find("/Count ").expect("no /Count in page tree") + "/Count ".len();
    let declared: usize = text[count_pos..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .expect("/Count is not a number");
    let page_objects = text.matches("/Type /Page\n").count() + text.matches("/Type /Page ").count();
    assert_eq!(
        declared, page_objects,
        "/Count does not match number of page objects"
    );
    declared
}

fn convert_to_pdf(name: &str, html: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    let output = dir.join(format!("{}.pdf", name));
    std::fs::write(&input, html)?;
    if output.exists() {
        std::fs::remove_file(&output)?;
    }

    Command::cargo_bin(env!("CARGO_PKG_NAME"))?
        .arg(&input)
        .arg(&output)
        .assert()
        .success();

    Ok(common::expand_object_streams(&std::fs::read(&output)?))
}

#[test]
fn long_document_is_paginated() -> Result<(), Box<dyn std::error::Error>> {
    let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                     tempor incididunt ut labore et dolore magna aliqua. "
        .repeat(8);
    let document = |paragraphs: usize| {
        let body: String = (0..paragraphs)
            .map(|_| format!("<p>{}</p>", paragraph))
            .collect();
        format!(
            "<!DOCTYPE html><html><body><h1>Long</h1>{}</body></html>",
            body
        )
    };

    let short = convert_to_pdf("pagination_short", &document(1))?;
    let long = convert_to_pdf("pagination_long", &document(5))?;

    assert!(short.starts_with(b"%PDF-") && long.starts_with(b"%PDF-"));
    assert_eq!(
        pdf_page_count(&short),
        1,
        "a single paragraph should fit on one page"
    );
    assert!(
        pdf_page_count(&long) >= 2,
        "five paragraphs should not fit on a single page"
    );

    Ok(())
}

#[test]
fn dumps_layout_as_json() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("dump_layout.html");
    std::fs::write(
        &input,
        r#"<!DOCTYPE html><div id="a" style="width: 100px; height: 20px; padding: 5px; margin: 3px"></div>"#,
    )?;

    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
    cmd.arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("dump_layout.pdf"));
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""id": "a""#))
        // border box includes the padding, the margin is reported separately
        // (the Taffy layout reports the specified margins, not the used `margin-right` of an
        // over-constrained block)
        .stdout(predicate::str::contains(r#""width": 110, "height": 30}"#))
        .stdout(predicate::str::contains(
            r#""margin": {"top": 3, "right": 3, "bottom": 3, "left": 3}"#,
        ));

    Ok(())
}

/// Selectors that look at sibling elements (`:first-of-type`, `+`, `~`) used to loop forever
/// because the sibling iterators included the element itself.
#[test]
fn sibling_selectors_terminate() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("sibling_selectors.html");
    std::fs::write(
        &input,
        "<html><head><style>h1:first-of-type { color: red } h1 + p { color: blue } h1 ~ h1 { color: green }</style></head>\
         <body><h1>One</h1><p>Hi</p><h1>Two</h1></body></html>",
    )
    .unwrap();
    Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg(&input)
        .arg(dir.join("sibling_selectors.pdf"))
        .assert()
        .success();
}

/// The hyperlinks in a document are written as link annotations and attached to the pages they
/// are on. A link that is split over multiple lines produces one annotation per line.
#[test]
fn hyperlinks_are_written_to_the_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "hyperlinks",
        r#"<!DOCTYPE html>
<html><body>
<p>One <a href="https://example.com">link</a> here.</p>
<p>Another <a href="https://example.org/a/very/long/path">long link that will wrap over the end of this line because it is long enough to do so</a> over.</p>
</body></html>"#,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    let utf8_owned = text.into_owned();

    let link_annotations: Vec<&str> = utf8_owned.split("/Subtype /Link").skip(1).collect();
    assert_eq!(
        link_annotations.len(),
        3,
        "expected 3 link annotations (one is split over two lines)"
    );

    assert!(
        utf8_owned.contains("/URI (https://example.com)"),
        "missing example.com URI"
    );
    // the wrapped link appears once per line it is on
    assert_eq!(
        utf8_owned
            .matches("/URI (https://example.org/a/very/long/path)")
            .count(),
        2
    );
    // every page that has annotations must reference them
    assert!(
        utf8_owned.contains("/Annots ["),
        "page does not reference its annotations"
    );

    Ok(())
}

/// Cross-reference links (`<a href="#...">`) become `GoTo` link annotations that jump to the
/// element with the matching `id`, on whatever page it ends up on. Links to unknown fragments
/// stay URI links.
#[test]
fn cross_reference_links_jump_within_the_document() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "cross_references",
        r##"<!DOCTYPE html>
<html><body>
<p><a href="#target">jump down</a></p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p id="target">the target</p>
</body></html>"##,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    let utf8_owned = text.into_owned();

    assert_eq!(
        utf8_owned.matches("/Subtype /Link").count(),
        1,
        "expected one link"
    );
    // the link must be a GoTo action (not a URI) with a page-0 destination.
    // The dictionary serialization splits key-value pairs across lines, so
    // /S /GoTo and /D [...] are separate assertions.
    assert!(
        utf8_owned.contains("/S /GoTo"),
        "cross-reference is not a GoTo action"
    );
    assert!(
        utf8_owned.contains("/D [ 0 /XYZ"),
        "GoTo destination does not reference page 0"
    );
    // and it must not be a URI link
    assert!(
        !utf8_owned.contains("/S /URI"),
        "cross-reference should not be a URI link"
    );

    Ok(())
}

/// A link to an unknown fragment stays a URI link instead of being dropped.
#[test]
fn link_to_unknown_fragment_stays_a_uri_link() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "unknown_fragment",
        r##"<!DOCTYPE html><html><body><p><a href="#missing">nothing</a></p></body></html>"##,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("/S /URI"),
        "expected a URI link for an unknown fragment"
    );
    assert!(
        !text.contains("/S /GoTo"),
        "unknown fragments must not become GoTo links"
    );
    Ok(())
}
