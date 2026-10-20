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
    let input = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/loremipsum.html");
    let output = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("loremipsum.pdf");
    // make sure a stale file from an earlier run cannot make the test pass
    if output.exists() {
        std::fs::remove_file(&output)?;
    }

    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
    cmd.arg(input).arg(&output);
    cmd.assert().success();

    let pdf = std::fs::read(&output)?;
    assert!(pdf.starts_with(b"%PDF-"), "output does not start with a PDF header");
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
    assert_eq!(declared, page_objects, "/Count does not match number of page objects");
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

    Ok(std::fs::read(&output)?)
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
        format!("<!DOCTYPE html><html><body><h1>Long</h1>{}</body></html>", body)
    };

    let short = convert_to_pdf("pagination_short", &document(1))?;
    let long = convert_to_pdf("pagination_long", &document(5))?;

    assert!(short.starts_with(b"%PDF-") && long.starts_with(b"%PDF-"));
    assert_eq!(pdf_page_count(&short), 1, "a single paragraph should fit on one page");
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
    cmd.arg("--dump-layout").arg("-").arg(&input).arg(dir.join("dump_layout.pdf"));
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""id": "a""#))
        // border box includes the padding, the margin is reported separately
        .stdout(predicate::str::contains(r#""width": 110, "height": 30}"#))
        .stdout(predicate::str::contains(r#""margin": {"top": 3, "right": 0, "bottom": 3, "left": 3}"#));

    Ok(())
}
