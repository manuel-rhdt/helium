//! Renders every document in `tests/documents` to a PDF in `target/tmp/documents` so the output
//! can be inspected by eye. To add an example, drop an `.html` file into `tests/documents`; it is
//! picked up automatically. Besides rendering successfully, each PDF must be well-formed.

use assert_cmd::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn documents() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/documents");
    let mut docs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("html"))
        .collect();
    docs.sort();
    docs
}

#[test]
fn example_documents_render_to_pdf() {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("documents");
    std::fs::create_dir_all(&out_dir).unwrap();

    let docs = documents();
    assert!(!docs.is_empty(), "no example documents found");

    let mut failures = Vec::new();
    for input in &docs {
        let output = out_dir.join(input.with_extension("pdf").file_name().unwrap());
        // make sure a stale file from an earlier run cannot make the test pass
        let _ = std::fs::remove_file(&output);

        let result = Command::cargo_bin(env!("CARGO_PKG_NAME"))
            .unwrap()
            .arg(input)
            .arg(&output)
            .output()
            .unwrap();
        let name = input.file_name().unwrap().to_string_lossy();
        if !result.status.success() {
            failures.push(format!(
                "{}: {}",
                name,
                String::from_utf8_lossy(&result.stderr)
            ));
            continue;
        }
        match std::fs::read(&output) {
            Ok(pdf)
                if pdf.starts_with(b"%PDF-")
                    && pdf.windows(5).rev().take(1024).any(|w| w == b"%%EOF") => {}
            Ok(_) => failures.push(format!("{}: output is not a well-formed PDF", name)),
            Err(e) => failures.push(format!("{}: no output written ({})", name, e)),
        }
    }
    assert!(
        failures.is_empty(),
        "failed to render:\n{}",
        failures.join("\n")
    );
    eprintln!("rendered {} documents to {}", docs.len(), out_dir.display());
}
