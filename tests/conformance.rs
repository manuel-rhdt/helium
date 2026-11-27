//! Validates PDF files against PDF/A and PDF/UA-1 with veraPDF (<https://verapdf.org>), the
//! reference validator.
//!
//! The tests need Java and the jars of veraPDF. They are skipped unless the environment variable
//! `VERAPDF_LIB` is the directory with the jars; `tools/fetch_verapdf.sh DIR` downloads them to
//! `DIR/lib`. A document is validated at the levels that it is written for, and a failure that is
//! known is listed in `KNOWN_FAILURES`: when one of them is fixed the test fails until it is
//! removed, so the list only gets shorter.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;

/// The documents of the gallery that are validated (the book is left out for its length).
const DOCUMENTS: [&str; 8] = ["report", "invoice", "article", "letter", "cv", "math-notes", "slides", "emoji"];

/// `(document, profile)` pairs that do not conform, and why.
const KNOWN_FAILURES: [(&str, &str); 1] = [
    // the `h2` of the sidebar comes before the `h1` in the document (7.4.2-1: a heading level is
    // skipped)
    ("cv", "ua1"),
];

fn verapdf_lib() -> Option<PathBuf> {
    let lib = PathBuf::from(std::env::var_os("VERAPDF_LIB")?);
    lib.is_dir().then_some(lib)
}

fn gallery(document: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("gallery").join(document)
}

/// Renders a document of the gallery (with its directory as the working directory, as the gallery
/// does) with the arguments, and returns the file.
fn render(document: &str, tag: &str, args: &[&str]) -> PathBuf {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("conformance_{document}_{tag}.pdf"));
    let mut command = common::umbruch().unwrap();
    command.current_dir(gallery(document)).arg("--no-system-fonts").args(args).arg("index.html").arg(&output);
    command.assert().success();
    output
}

/// The profiles that veraPDF reports as failed: `(file name, profile)`.
fn validate(lib: &Path, profile: &str, files: &[PathBuf]) -> Vec<PathBuf> {
    let output = Command::new("java")
        .arg("-cp")
        .arg(format!("{}/*", lib.display()))
        .arg("org.verapdf.apps.GreenfieldCliWrapper")
        .args(["--flavour", profile, "--format", "text"])
        .args(files)
        .output()
        .expect("cannot run java");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut failed = vec![];
    let mut verdicts = 0;
    for line in text.lines() {
        let Some((verdict, rest)) = line.split_once(' ') else { continue };
        if verdict == "PASS" || verdict == "FAIL" {
            verdicts += 1;
            if verdict == "FAIL" {
                failed.push(PathBuf::from(rest.rsplit_once(' ').map_or(rest, |(path, _)| path)));
            }
        }
    }
    assert_eq!(verdicts, files.len(), "veraPDF did not give a verdict for every file:\n{text}\n{}", String::from_utf8_lossy(&output.stderr));
    failed
}

/// Checks that exactly the documents of `KNOWN_FAILURES` fail the profile.
fn check(lib: &Path, profile: &str, files: Vec<(&str, PathBuf)>) {
    let paths: Vec<PathBuf> = files.iter().map(|(_, path)| path.clone()).collect();
    let failed = validate(lib, profile, &paths);
    for (document, path) in &files {
        let known = KNOWN_FAILURES.contains(&(*document, profile));
        let failed = failed.contains(path);
        assert!(!failed || known, "{document} does not conform to {profile}: run veraPDF on {}", path.display());
        assert!(failed || !known, "{document} conforms to {profile} now: remove it from KNOWN_FAILURES");
    }
}

#[test]
fn the_gallery_conforms_to_pdfua1() {
    let Some(lib) = verapdf_lib() else {
        eprintln!("skipped: VERAPDF_LIB is not set (see tools/fetch_verapdf.sh)");
        return;
    };
    let files = DOCUMENTS.iter().map(|document| (*document, render(document, "ua1", &[]))).collect();
    check(&lib, "ua1", files);
}

#[test]
fn the_gallery_conforms_to_pdfa() {
    let Some(lib) = verapdf_lib() else {
        eprintln!("skipped: VERAPDF_LIB is not set (see tools/fetch_verapdf.sh)");
        return;
    };
    // every document at the lowest and at the highest level of the level a (which also checks
    // the structure tree), and one document at every level
    for level in ["2b", "3a"] {
        let files = DOCUMENTS.iter().map(|document| (*document, render(document, level, &["--pdfa", level]))).collect();
        check(&lib, level, files);
    }
    for level in ["2a", "2u", "3b", "3u"] {
        check(&lib, level, vec![("letter", render("letter", level, &["--pdfa", level]))]);
    }
}

/// The documents of the gallery that need nothing that PDF/A-1 does not allow (the others use
/// transparency: the `invoice`, the `cv`, `math-notes` and `slides` have SVG or colours with opacity, and the
/// `emoji` colour glyphs); `a_document_with_transparency_is_not_written_as_pdfa1` has the other side.
#[test]
fn the_gallery_documents_without_transparency_conform_to_pdfa1() {
    let Some(lib) = verapdf_lib() else {
        eprintln!("skipped: VERAPDF_LIB is not set (see tools/fetch_verapdf.sh)");
        return;
    };
    let files = ["report", "article", "letter"].iter().map(|document| (*document, render(document, "1b", &["--pdfa", "1b"]))).collect();
    check(&lib, "1b", files);
}

#[cfg(feature = "embedded-fonts")]
#[test]
fn a_document_with_transparency_is_not_written_as_pdfa1() {
    for document in ["invoice", "cv", "math-notes", "slides", "emoji"] {
        let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("conformance_{document}_1b_error.pdf"));
        let _ = std::fs::remove_file(&output);
        let result = common::umbruch()
            .unwrap()
            .current_dir(gallery(document))
            .args(["--no-system-fonts", "--pdfa", "1b", "index.html"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{document}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("PDF/A-1"), "{document}");
        assert!(!output.exists(), "{document}: the file is kept");
    }
}

#[test]
fn wide_gamut_colors_conform_to_pdfa() {
    let Some(lib) = verapdf_lib() else {
        eprintln!("skipped: VERAPDF_LIB is not set (see tools/fetch_verapdf.sh)");
        return;
    };
    // colours of a wide gamut are CIELAB colours, which PDF/A allows; with an opacity (a
    // transparency group) and a link besides
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("conformance_wide.html");
    std::fs::write(
        &input,
        "<!doctype html><html lang=en><title>Wide</title>\
         <p style=\"color: color(display-p3 1 0 0); background: oklch(0.7 0.3 150); border: 2px solid color(rec2020 0 1 0); opacity: .9\">\
         wide <a href=\"#x\" style=\"color: lab(60 120 80)\">link</a></p><p id=x>x</p>",
    )
    .unwrap();
    for level in ["2b", "3a"] {
        let output = dir.join(format!("conformance_wide_{level}.pdf"));
        common::umbruch().unwrap().args(["--no-system-fonts", "--pdfa", level]).arg(&input).arg(&output).assert().success();
        assert!(validate(&lib, level, &[output.clone()]).is_empty(), "wide-gamut colours do not conform to {level}: {}", output.display());
    }
}
