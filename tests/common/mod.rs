//! Helpers shared by the web-platform-tests runners.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn wpt_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt")
}

pub fn collect_tests(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_tests(&path, out);
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("html" | "xht" | "xhtml")) {
            out.push(path);
        }
    }
}

/// The test's path relative to `tests/wpt`, with `/` as separator.
pub fn test_name(test: &Path) -> String {
    test.strip_prefix(wpt_root()).unwrap().to_string_lossy().replace('\\', "/")
}

/// Paths listed in an expectations file (blank lines and `#` comments are ignored).
pub fn read_expectations(file: &str) -> BTreeSet<String> {
    std::fs::read_to_string(wpt_root().join(file))
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Describes every expectation that refers to a file that does not exist.
pub fn stale_expectations(known_failures: &BTreeSet<String>) -> Vec<String> {
    known_failures
        .iter()
        .filter(|name| !wpt_root().join(name).exists())
        .map(|name| format!("expectations list missing test {}", name))
        .collect()
}

/// Where the PDF for `input` is written so it can be inspected by hand: one file per input under
/// `target/tmp/wpt-pdfs`, e.g. `css__CSS2__normal-flow__height-001.xht.pdf`.
pub fn pdf_path(input: &Path) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wpt-pdfs");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{}.pdf", test_name(input).replace('/', "__")))
}
