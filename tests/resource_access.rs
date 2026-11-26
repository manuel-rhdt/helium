//! `src/resource.rs` is the only place where a reference of a document is turned into bytes.
//!
//! This test scans the source for everything that opens a file, a directory, a socket or a
//! process, and fails for a file that is not in the list below. A new entry in the list is a
//! change of what a hostile document can reach, and a reviewer has to agree to it: the list is
//! short on purpose, and every entry says why it is not a reference of a document.
//!
//! The scan is a text search: it finds what is written, not what a macro or a dependency does.
//! It covers `src` and the vendored `svg`, not the other dependencies.

use std::path::{Path, PathBuf};

/// What counts: the calls that reach the file system, the network and other programs.
const PATTERNS: &[&str] = &[
    "fs::read",
    "fs::File",
    "File::open",
    "File::create",
    "OpenOptions",
    "read_dir",
    "read_to_string",
    "std::net",
    "TcpStream",
    "UdpSocket",
    "Command::new",
    "include_str!(concat!(env!(",
    "fs::write",
    "fs::copy",
    "fs::rename",
    "fs::remove",
    "fs::create_dir",
    "memmap",
];

/// The files that may contain them, and why.
const ALLOWED: &[(&str, &str)] = &[
    ("src/resource.rs", "the reader of the references of a document"),
    ("src/main.rs", "the command line: the input file and the output file are named by the one who runs the program"),
    (
        "src/layout/text/fallback.rs",
        "the fonts that the machine has installed, found by fontconfig: not named by a document",
    ),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The code of a file without its comments and without the tests at its end.
fn code(text: &str) -> String {
    let text = text.split("#[cfg(test)]").next().unwrap_or(text);
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn only_the_resource_module_reads_what_a_document_refers_to() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("svg/src"), &mut files);
    assert!(files.len() > 50, "the scan found only {} files", files.len());
    let mut found = vec![];
    for file in &files {
        let relative = file.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if ALLOWED.iter().any(|(allowed, _)| *allowed == relative) {
            continue;
        }
        let text = code(&std::fs::read_to_string(file).unwrap());
        for pattern in PATTERNS {
            if text.contains(pattern) {
                found.push(format!("{relative}: `{pattern}`"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "these files reach the file system, the network or a process without being in the list of tests/resource_access.rs \
         (documents must be read through `umbruch::resource`):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_list_has_no_entry_that_is_not_needed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, _) in ALLOWED {
        let text = code(&std::fs::read_to_string(root.join(file)).unwrap());
        assert!(PATTERNS.iter().any(|pattern| text.contains(pattern)), "{file} does not need to be in the list");
    }
}
