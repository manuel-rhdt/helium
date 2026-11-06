//! Helpers shared by the integration tests.
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
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("html" | "xht" | "xhtml")
        ) {
            out.push(path);
        }
    }
}

/// The test's path relative to `tests/wpt`, with `/` as separator.
pub fn test_name(test: &Path) -> String {
    test.strip_prefix(wpt_root())
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
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

/// Appends the decompressed contents of all object streams to the PDF bytes, so that tests
/// can search for dictionaries (pages, annotations, ...) with plain text matching.
pub fn expand_object_streams(pdf: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut result = pdf.to_vec();
    let marker = b"/Type /ObjStm";
    let mut pos = 0;
    while let Some(found) = pdf[pos..].windows(marker.len()).position(|w| w == marker) {
        let at = pos + found;
        pos = at + marker.len();
        let Some(stream_start) = pdf[at..].windows(8).position(|w| w == b"\nstream\n") else {
            continue;
        };
        let start = at + stream_start + 8;
        let Some(end) = pdf[start..].windows(10).position(|w| w == b"\nendstream") else {
            continue;
        };
        let mut inflated = Vec::new();
        flate2::read::ZlibDecoder::new(&pdf[start..start + end])
            .read_to_end(&mut inflated)
            .expect("object stream is not valid zlib data");
        result.push(b'\n');
        result.extend_from_slice(&inflated);
    }
    result
}

/// The `helium` binary, set up to use only its built-in fonts (`HELIUM_NO_SYSTEM_FONTS`), so
/// that the tests do not depend on the fonts installed on the machine.
pub fn helium() -> Result<std::process::Command, assert_cmd::cargo::CargoError> {
    use assert_cmd::prelude::*;
    let mut cmd = std::process::Command::cargo_bin(env!("CARGO_PKG_NAME"))?;
    cmd.env("HELIUM_NO_SYSTEM_FONTS", "1");
    Ok(cmd)
}
