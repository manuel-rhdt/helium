//! The only place where a reference in a document is turned into bytes.
//!
//! Everything that a document can point to from the inside (`<img src>`, `url()` in CSS,
//! `@font-face`, `<link rel=stylesheet>`, the images of an SVG) is read through [`read`] or
//! [`read_file`]. Nothing else in the crate opens a file for a document, and
//! `tests/resource_access.rs` checks that by scanning the source, so this module is the one
//! to read when asking what a hostile document can get at.
//!
//! The rules:
//!
//! - A reference is a `data:` URI (the bytes are in the reference itself) or a relative path.
//!   Every other URL scheme is refused: nothing is fetched from the network, and `file:` is not
//!   a way around the next rule.
//! - A path is read only if, after the symbolic links and the `..` have been resolved, it is a
//!   regular file inside of the directory of the document, or of the resource root if the engine
//!   has one ([`set_resource_root`]). An absolute path is not an exception: it must be inside as
//!   well.
//! - A file is not read if it is larger than [`MAX_BYTES`].
//! - The error is the same for a file that does not exist and for one that is outside of the
//!   directory, and it names only the reference as the document gave it, never a path of the
//!   machine. What it says is therefore no use for finding out what is on the machine. The details
//!   go to the debug log, which is the operator's.
//!
//! Not covered: the path is resolved and then opened, so a process that can change the files
//! inside of the directory while a document is rendered could swap a link between the two steps.
//! The directory is meant to belong to the document (one directory for one request).
//!
//! The fonts of the machine (system fonts and `--font-dir`) are not references of a document and
//! are loaded by `layout::text` (see the list in `tests/resource_access.rs`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The largest file that is read: 128 MiB (larger than any font or image that makes sense).
pub const MAX_BYTES: u64 = 128 << 20;

static RESOURCE_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Sets the directory that the files of a document must be inside of, if it is not the directory
/// of the document itself: a document may refer to files next to it, such as
/// `../support/image.png`. Must be called before the first resource is read; the first call wins.
pub fn set_resource_root(root: PathBuf) {
    let _ = RESOURCE_ROOT.set(root);
}

/// The bytes that `src` refers to: a `data:` URI, or a file as [`read_file`] describes.
pub fn read(src: &str, relative_to: &Path, root: &Path) -> Result<Vec<u8>, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err("the reference is empty".to_owned());
    }
    if let Some(rest) = strip_prefix_ignore_case(src, "data:") {
        return data_uri(rest);
    }
    read_file(src, relative_to, root).map(|(bytes, _)| bytes)
}

/// The contents of the file that the relative URL `src` (relative to the directory `relative_to`)
/// refers to, and its canonical path. The file must be inside of `root` (or of the resource root,
/// which replaces it).
pub fn read_file(src: &str, relative_to: &Path, root: &Path) -> Result<(Vec<u8>, PathBuf), String> {
    read_file_within(src, relative_to, RESOURCE_ROOT.get().map_or(root, |root| root.as_path()))
}

/// [`read_file`] with the directory that the file must be inside of, as it is (the resource root
/// does not replace it): the part that the tests can reach.
fn read_file_within(src: &str, relative_to: &Path, allowed: &Path) -> Result<(Vec<u8>, PathBuf), String> {
    let src = src.trim();
    let refused = || format!("cannot read `{}`: it does not exist or is outside of the directory of the document", src);
    if let Some(colon) = src.find(':') {
        // a scheme (`https:`), not a directory with a colon in its name
        if src[..colon].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return Err(format!(
                "`{}` is not supported: only files next to the document and data URIs are",
                &src[..=colon]
            ));
        }
    }

    // a path in a URL: percent-encoded, and without the query or fragment
    let path = src.split(['?', '#']).next().unwrap_or(src);
    let path = percent_decode(path)?;
    let full = relative_to.join(&path);
    let canonical = full.canonicalize().map_err(|error| {
        log::debug!("resource `{}`: {}: {}", src, full.display(), error);
        refused()
    })?;
    let base = allowed
        .canonicalize()
        .map_err(|error| format!("the directory of the document cannot be read: {}", error))?;
    if !canonical.starts_with(&base) {
        log::debug!("resource `{}`: {} is outside of {}", src, canonical.display(), base.display());
        return Err(refused());
    }

    let file = std::fs::File::open(&canonical).map_err(|error| {
        log::debug!("resource `{}`: {}: {}", src, canonical.display(), error);
        refused()
    })?;
    let metadata = file.metadata().map_err(|_| refused())?;
    if !metadata.is_file() {
        return Err(format!("cannot read `{}`: it is not a file", src));
    }
    if metadata.len() > MAX_BYTES {
        return Err(format!("cannot read `{}`: the file is larger than {} MiB", src, MAX_BYTES >> 20));
    }
    // (a file can grow after the check)
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read `{}`: {}", src, error.kind()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("cannot read `{}`: the file is larger than {} MiB", src, MAX_BYTES >> 20));
    }
    Ok((bytes, canonical))
}

/// The bytes of a data URI (what is after `data:`).
fn data_uri(rest: &str) -> Result<Vec<u8>, String> {
    let (header, payload) = rest
        .split_once(',')
        .ok_or_else(|| "a data URI needs a comma before its data".to_owned())?;
    if header.to_ascii_lowercase().ends_with(";base64") {
        decode_base64(payload)
    } else {
        // text, mostly SVG: percent-encoded
        percent_decode_bytes(payload)
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

fn percent_decode(text: &str) -> Result<String, String> {
    String::from_utf8(percent_decode_bytes(text)?).map_err(|_| format!("`{}` is not valid UTF-8", text))
}

fn percent_decode_bytes(text: &str) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("invalid percent escape in `{}`", text))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(out)
}

/// Decodes standard base64. Whitespace is ignored and the padding is optional, as in the
/// forgiving-base64 decoding of the URL standard.
fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    let mut padding = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            b if b.is_ascii_whitespace() => continue,
            _ => return Err("the data URI is not valid base64".to_owned()),
        };
        if padding > 0 {
            return Err("the data URI is not valid base64".to_owned());
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVs bG8").unwrap(), b"hello");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("a$b").is_err());
        assert!(decode_base64("aG=V").is_err());
    }

    #[test]
    fn percent() {
        assert_eq!(percent_decode("a%20b.jpg").unwrap(), "a b.jpg");
        assert!(percent_decode("a%2").is_err());
    }

    /// A directory with `root/inside.txt`, `root/sub/`, and (next to the root, so outside of it)
    /// `outside.txt`. Every test has its own.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!("umbruch_resource_{}_{}", std::process::id(), name));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("root/sub")).unwrap();
            std::fs::write(dir.join("root/inside.txt"), "in").unwrap();
            std::fs::write(dir.join("root/sub/deep.txt"), "deep").unwrap();
            std::fs::write(dir.join("outside.txt"), "SECRET").unwrap();
            Fixture(dir)
        }

        fn root(&self) -> PathBuf {
            self.0.join("root")
        }

        fn read(&self, src: &str) -> Result<Vec<u8>, String> {
            read_file_within(src, &self.root(), &self.root()).map(|(bytes, _)| bytes)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn files_inside_of_the_directory_are_read() {
        let f = Fixture::new("inside");
        assert_eq!(f.read("inside.txt").unwrap(), b"in");
        assert_eq!(f.read("./sub/deep.txt").unwrap(), b"deep");
        assert_eq!(f.read("sub/../inside.txt").unwrap(), b"in");
        assert_eq!(f.read("inside.txt?v=2#top").unwrap(), b"in");
        assert_eq!(f.read("sub/%64eep.txt").unwrap(), b"deep");
    }

    #[test]
    fn files_outside_of_the_directory_are_not_read() {
        let f = Fixture::new("outside");
        let absolute = f.0.join("outside.txt").to_string_lossy().into_owned();
        for src in ["../outside.txt", "sub/../../outside.txt", "%2e%2e/outside.txt", absolute.as_str(), "/etc/passwd"] {
            let error = f.read(src).unwrap_err();
            assert!(error.contains("outside of the directory"), "{src}: {error}");
        }
    }

    #[test]
    fn a_file_that_does_not_exist_and_one_that_is_outside_give_the_same_error() {
        let f = Fixture::new("oracle");
        let exists = f.read("../outside.txt").unwrap_err();
        let missing = f.read("../nothing_here.txt").unwrap_err();
        // the reference differs, nothing else does
        assert_eq!(exists.replace("outside.txt", "X"), missing.replace("nothing_here.txt", "X"));
        assert_eq!(f.read("/etc/passwd").unwrap_err().replace("/etc/passwd", "X"), f.read("/etc/no_such_file").unwrap_err().replace("/etc/no_such_file", "X"));
        // and it contains no path of the machine
        assert!(!exists.contains(f.0.to_str().unwrap()), "{exists}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_to_a_file_outside_is_not_followed() {
        let f = Fixture::new("symlink");
        std::os::unix::fs::symlink("../outside.txt", f.root().join("link.txt")).unwrap();
        std::os::unix::fs::symlink("..", f.root().join("up")).unwrap();
        std::os::unix::fs::symlink("inside.txt", f.root().join("fine.txt")).unwrap();
        assert!(f.read("link.txt").unwrap_err().contains("outside of the directory"));
        assert!(f.read("up/outside.txt").unwrap_err().contains("outside of the directory"));
        // a link that stays inside is fine
        assert_eq!(f.read("fine.txt").unwrap(), b"in");
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let f = Fixture::new("directory");
        assert!(f.read("sub").unwrap_err().contains("not a file"));
    }

    #[test]
    fn other_schemes_are_not_supported() {
        let f = Fixture::new("schemes");
        for src in ["https://example.com/a.css", "http://example.com/", "file:///etc/passwd", "ftp://x/y", "javascript:alert(1)"] {
            let error = read(src, &f.root(), &f.root()).unwrap_err();
            assert!(error.contains("not supported"), "{src}: {error}");
        }
    }

    #[test]
    fn data_uris_are_decoded_and_the_empty_reference_is_refused() {
        let dir = Path::new(".");
        assert_eq!(read("data:text/plain;base64,aGk=", dir, dir).unwrap(), b"hi");
        assert_eq!(read("DATA:text/plain,h%69", dir, dir).unwrap(), b"hi");
        assert!(read("data:text/plain", dir, dir).is_err());
        assert!(read("  ", dir, dir).is_err());
    }

    #[test]
    fn a_file_that_is_too_large_is_not_read() {
        // (a sparse file: the size is set without the bytes)
        let f = Fixture::new("large");
        let big = std::fs::File::create(f.root().join("big.bin")).unwrap();
        big.set_len(MAX_BYTES + 1).unwrap();
        assert!(f.read("big.bin").unwrap_err().contains("larger than"));
        let ok = std::fs::File::create(f.root().join("ok.bin")).unwrap();
        ok.set_len(1024).unwrap();
        assert_eq!(f.read("ok.bin").unwrap().len(), 1024);
    }
}
