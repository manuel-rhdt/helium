//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The directory with the web-platform-tests: `tests/wpt`, or the one that the environment
/// variable `WPT_ROOT` names (to try a larger copy of the tests without vendoring it).
pub fn wpt_root() -> PathBuf {
    std::env::var_os("WPT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt"))
}

/// The fonts that the tests use (Ahem): always the vendored ones.
pub fn wpt_fonts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt/fonts")
}

/// Where the lists of known failures are.
fn expectations_path(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt").join(file)
}

/// Whether the runners write the lists of known failures from what they find
/// (`WPT_UPDATE_EXPECTATIONS=1`) instead of comparing with them.
pub fn update_expectations() -> bool {
    std::env::var_os("WPT_UPDATE_EXPECTATIONS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Writes the list of known failures: the header (the comment lines at the start of the file)
/// is kept.
pub fn write_expectations(file: &str, failing: &BTreeSet<String>) {
    let path = expectations_path(file);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    // the header is the comment up to the first empty line
    let keep: Vec<&str> = old.lines().collect();
    let header_len = keep.iter().position(|line| line.trim().is_empty()).map_or(keep.len(), |at| at + 1);
    let mut out: String = keep[..header_len].iter().map(|line| format!("{}\n", line)).collect();
    // (the reasons that were written by hand stay where the entries they are for are)
    let mut reasons = String::new();
    let mut pending: Vec<&str> = vec![];
    for line in keep[header_len..].iter() {
        if line.starts_with('#') {
            pending.push(line);
        } else if line.trim().is_empty() {
            pending.clear();
        } else if failing.contains(line.trim()) && !pending.is_empty() {
            for comment in pending.drain(..) {
                reasons.push_str(comment);
                reasons.push('\n');
            }
            reasons.push_str(line);
            reasons.push_str("\n\n");
        } else {
            pending.clear();
        }
    }
    let described: BTreeSet<&str> = reasons.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).collect();
    out.push_str(&reasons);
    if failing.iter().any(|name| !described.contains(name.as_str())) {
        out.push_str("# Not investigated yet (`WPT_UPDATE_EXPECTATIONS=1` writes this list from what the tests do)\n");
    }
    for name in failing {
        if !described.contains(name.as_str()) {
            out.push_str(name);
            out.push('\n');
        }
    }
    std::fs::write(path, out).unwrap();
}

/// Runs `f` for every item on all the cores, and returns the results in the order of the items.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<R>>> = items.iter().map(|_| Default::default()).collect();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(index) else { break };
                *results[index].lock().unwrap() = Some(f(item));
            });
        }
    });
    results.into_iter().map(|r| r.into_inner().unwrap().unwrap()).collect()
}

/// Runs the command and kills it if it takes longer than `seconds`.
pub fn output_within(mut cmd: std::process::Command, seconds: u64) -> Option<std::process::Output> {
    use std::process::Stdio;
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    let (mut stdout, mut stderr) = (child.stdout.take()?, child.stderr.take()?);
    let out_thread = std::thread::spawn(move || {
        let mut buffer = vec![];
        std::io::Read::read_to_end(&mut stdout, &mut buffer).ok();
        buffer
    });
    let err_thread = std::thread::spawn(move || {
        let mut buffer = vec![];
        std::io::Read::read_to_end(&mut stderr, &mut buffer).ok();
        buffer
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    Some(std::process::Output {
        status,
        stdout: out_thread.join().ok()?,
        stderr: err_thread.join().ok()?,
    })
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
    std::fs::read_to_string(expectations_path(file))
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
