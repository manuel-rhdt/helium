//! Loading the images of `<img>` elements.
//!
//! Only JPEG files are supported. They are not decoded: the PDF embeds the file as it is
//! (`DCTDecode`), so the only thing needed to lay an image out is its size in pixels.

use std::path::Path;

use lemon_pdf::image::Image;

/// A JPEG file that is going to be embedded into the PDF.
#[derive(Debug)]
pub struct JpegImage {
    /// The complete file.
    pub data: Vec<u8>,
    /// The size in pixels, which is also the intrinsic size in CSS pixels.
    pub width: u32,
    pub height: u32,
}

impl JpegImage {
    /// Checks that `data` is a JPEG file that can be embedded and reads its size.
    pub fn from_bytes(data: Vec<u8>) -> Result<JpegImage, String> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            return Err("PNG images are not supported yet, only JPEG".to_owned());
        }
        let image = Image::from_jpeg(data.clone()).map_err(|error| error.to_string())?;
        if image.width() == 0 || image.height() == 0 {
            return Err("the image has no pixels".to_owned());
        }
        Ok(JpegImage {
            width: image.width(),
            height: image.height(),
            data,
        })
    }

    /// The image as the PDF library embeds it.
    pub fn to_pdf_image(&self) -> Result<Image, String> {
        Image::from_jpeg(self.data.clone()).map_err(|error| error.to_string())
    }
}

/// Loads the image that the `src` attribute of an `<img>` refers to.
///
/// `src` is a `data:` URI with base64 data or the path of a file inside of `base_dir` (the
/// directory of the document). Other files are not read, so that a document cannot embed
/// arbitrary files of the machine that renders it, and nothing is fetched from the network.
pub fn load(src: &str, base_dir: &Path) -> Result<JpegImage, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err("the `src` attribute is empty".to_owned());
    }
    if let Some(rest) = strip_prefix_ignore_case(src, "data:") {
        let (header, payload) = rest
            .split_once(',')
            .ok_or_else(|| "a data URI needs a comma before its data".to_owned())?;
        if !header.to_ascii_lowercase().ends_with(";base64") {
            return Err("only base64 data URIs are supported".to_owned());
        }
        return JpegImage::from_bytes(decode_base64(payload)?);
    }
    if let Some(colon) = src.find(':') {
        // a scheme (`https:`), not a directory with a colon in its name
        if src[..colon]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(format!(
                "`{}` is not supported: only files next to the document and data URIs are",
                &src[..=colon]
            ));
        }
    }

    // a path in a URL: percent-encoded, and without the query or fragment
    let path = src.split(['?', '#']).next().unwrap_or(src);
    let path = percent_decode(path)?;
    let full = base_dir.join(&path);
    let canonical = full
        .canonicalize()
        .map_err(|error| format!("cannot read {}: {}", full.display(), error))?;
    let base = base_dir
        .canonicalize()
        .map_err(|error| format!("cannot read {}: {}", base_dir.display(), error))?;
    if !canonical.starts_with(&base) {
        return Err(format!(
            "{} is outside of the directory of the document",
            full.display()
        ));
    }
    let data = std::fs::read(&canonical)
        .map_err(|error| format!("cannot read {}: {}", canonical.display(), error))?;
    JpegImage::from_bytes(data)
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

fn percent_decode(text: &str) -> Result<String, String> {
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
    String::from_utf8(out).map_err(|_| format!("`{}` is not valid UTF-8", text))
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

    #[test]
    fn rejects_non_jpeg_data() {
        assert!(JpegImage::from_bytes(b"not an image".to_vec()).is_err());
        let error = JpegImage::from_bytes(vec![0x89, b'P', b'N', b'G', 0, 0]).unwrap_err();
        assert!(error.contains("PNG"), "{}", error);
    }

    #[test]
    fn rejects_other_places() {
        let dir = std::path::Path::new(".");
        assert!(load("https://example.com/a.jpg", dir).unwrap_err().contains("not supported"));
        assert!(load("file:///etc/passwd", dir).unwrap_err().contains("not supported"));
        assert!(load("", dir).is_err());
        assert!(load("data:image/png,abc", dir).unwrap_err().contains("base64"));
        assert!(load("../../etc/passwd", dir).is_err());
    }
}
