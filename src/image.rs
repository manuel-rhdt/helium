//! Loading the images of `<img>` elements.
//!
//! SVG images are parsed into a `usvg` tree that `crate::svg` draws as vector graphics.
//!
//! JPEG files and PNG files are supported. Their pixels are normally not decoded: the PDF
//! embeds the compressed data as it is (`DCTDecode` for JPEG, `FlateDecode` with a predictor for
//! PNG), so the only thing needed to lay an image out is its size in pixels. Only the PNG files
//! that PDF cannot take as they are (with an alpha channel, or interlaced) are decoded.

use std::path::{Path, PathBuf};

use lemon_pdf::image::{Image, ImageError};

/// An image that is going to be painted into the PDF: a raster image that is embedded as it is,
/// or a vector image that is drawn.
#[derive(Debug)]
pub enum LoadedImage {
    Raster(RasterImage),
    Svg(SvgImage),
}

/// An SVG image.
pub struct SvgImage {
    pub tree: usvg::Tree,
}

impl std::fmt::Debug for SvgImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgImage").field("size", &self.tree.size()).finish()
    }
}

impl LoadedImage {
    /// The intrinsic size in CSS pixels.
    pub fn size(&self) -> (f32, f32) {
        match self {
            LoadedImage::Raster(image) => (image.width as f32, image.height as f32),
            LoadedImage::Svg(image) => (image.tree.size().width(), image.tree.size().height()),
        }
    }

    /// Reads an image file. The format is recognized by the contents: PNG and JPEG by their
    /// signatures, and everything that looks like XML is read as SVG.
    pub fn from_bytes(data: Vec<u8>, base_dir: &Path) -> Result<LoadedImage, String> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) || data.starts_with(&[0xFF, 0xD8]) {
            return RasterImage::from_bytes(data).map(LoadedImage::Raster);
        }
        let text = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&data);
        if text.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<') {
            return parse_svg(&data, base_dir).map(|tree| LoadedImage::Svg(SvgImage { tree }));
        }
        Err("only JPEG, PNG and SVG images are supported".to_owned())
    }
}

/// The shaper of the text of SVG images, shared by all of them so that it keeps the fonts it has
/// loaded.
fn shaper() -> std::sync::Arc<dyn usvg::TextShaper> {
    static SHAPER: std::sync::OnceLock<std::sync::Arc<crate::svg_text::HarfbuzzShaper>> =
        std::sync::OnceLock::new();
    SHAPER.get_or_init(Default::default).clone()
}

/// The options for reading SVG: its own images are resolved under the same rules as the images
/// of the document (files only inside of `base_dir`, and no network) and its text uses the fonts
/// of the document.
fn svg_options(base_dir: &Path) -> usvg::Options<'static> {
    let mut options = usvg::Options::default();
    options.resources_dir = Some(base_dir.to_path_buf());
    options.fontdb = crate::layout::text::font::svg_font_database();
    options.font_family = "serif".to_owned();
    options.text_shaper = Some(shaper());
    // the size of an SVG that has none, like the default object size of CSS
    options.default_size = usvg::Size::from_wh(300.0, 150.0).unwrap();
    options.image_href_resolver.resolve_string = {
        let default = usvg::ImageHrefResolver::default_string_resolver();
        let base_dir = base_dir.to_path_buf();
        Box::new(move |href, options| match resolve_path(href, &base_dir) {
            Ok(path) => default(&path.to_string_lossy(), options),
            Err(error) => {
                log::warn!("SVG: cannot load the image `{}`: {}", href, error);
                None
            }
        })
    };
    options
}

/// Parses an SVG file.
fn parse_svg(data: &[u8], base_dir: &Path) -> Result<usvg::Tree, String> {
    let options = svg_options(base_dir);
    // `usvg` is meant for untrusted files, but a panic must not take the document down
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| usvg::Tree::from_data(data, &options)))
        .map_err(|_| "the SVG image could not be processed".to_owned())?
        .map_err(|error| format!("invalid SVG image: {}", error))
}

/// The SVG image of an `<svg>` element of the document, from the XML tree that
/// `crate::svg_dom::document` made of it. `font_size` and `font_family` are the ones of the
/// element, for the lengths in `em`. The font family of the text is an attribute of the root
/// element of the document.
pub fn svg_from_dom(
    document: &usvg::xml::Document,
    base_dir: &Path,
    font_size: f32,
) -> Result<LoadedImage, String> {
    let mut options = svg_options(base_dir);
    options.font_size = font_size;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| usvg::Tree::from_xml(document, &options)))
        .map_err(|_| "the SVG image could not be processed".to_owned())?
        .map(|tree| LoadedImage::Svg(SvgImage { tree }))
        .map_err(|error| format!("invalid SVG image: {}", error))
}

/// A JPEG or PNG file that is going to be embedded into the PDF.
#[derive(Debug)]
pub struct RasterImage {
    /// The complete file.
    pub data: Vec<u8>,
    /// The size in pixels, which is also the intrinsic size in CSS pixels.
    pub width: u32,
    pub height: u32,
    /// The decoded pixels, for the PNG files that cannot be embedded as they are.
    decoded: Option<Image>,
}

impl RasterImage {
    /// Checks that `data` is a JPEG or PNG file that can be embedded and reads its size.
    ///
    /// The format is recognized by the signature of the file, not by its name or MIME type.
    pub fn from_bytes(data: Vec<u8>) -> Result<RasterImage, String> {
        let (image, decoded) = to_pdf_image(&data)?;
        if image.width() == 0 || image.height() == 0 {
            return Err("the image has no pixels".to_owned());
        }
        Ok(RasterImage {
            width: image.width(),
            height: image.height(),
            decoded: decoded.then_some(image),
            data,
        })
    }

    /// The image as the PDF library embeds it.
    pub fn to_pdf_image(&self) -> Result<Image, String> {
        match &self.decoded {
            Some(image) => Ok(image.clone()),
            None => to_pdf_image(&self.data).map(|(image, _)| image),
        }
    }
}

/// The image for the PDF library, and whether its pixels were decoded (and not the file kept).
///
/// JPEG files and the PNG files without alpha channel and interlacing are embedded as they
/// are. The other PNG files are decoded to 8 bits per component, with the alpha channel as a
/// soft mask.
fn to_pdf_image(data: &[u8]) -> Result<(Image, bool), String> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        match Image::from_png(data) {
            Ok(image) => Ok((image, false)),
            Err(ImageError::UnsupportedPng(_)) => decode_png(data).map(|image| (image, true)),
            Err(error) => Err(error.to_string()),
        }
    } else if data.starts_with(&[0xFF, 0xD8]) {
        Image::from_jpeg(data.to_vec())
            .map(|image| (image, false))
            .map_err(|error| error.to_string())
    } else {
        Err("only JPEG and PNG images are supported".to_owned())
    }
}

/// Decodes a PNG file. Palettes and transparent colors are expanded and 16 bit samples are
/// reduced to 8 bits; gamma and color profiles are ignored.
fn decode_png(data: &[u8]) -> Result<Image, String> {
    use png::ColorType;
    let invalid = |error: png::DecodingError| format!("invalid PNG file: {}", error);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(invalid)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "the PNG image is too large".to_owned())?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer).map_err(invalid)?;
    buffer.truncate(frame.buffer_size());
    if frame.bit_depth != png::BitDepth::Eight {
        return Err("unexpected bit depth of the decoded PNG image".to_owned());
    }
    let (width, height) = (frame.width, frame.height);
    match frame.color_type {
        ColorType::Rgba => Image::from_rgba8(width, height, &buffer),
        ColorType::GrayscaleAlpha => Image::from_gray_alpha8(width, height, &buffer),
        ColorType::Rgb => Image::from_rgb8(width, height, buffer),
        ColorType::Grayscale => Image::from_gray8(width, height, buffer),
        ColorType::Indexed => return Err("the palette of the PNG image was not expanded".to_owned()),
    }
    .map_err(|error| error.to_string())
}

/// Loads the image that the `src` attribute of an `<img>` refers to.
///
/// `src` is a `data:` URI with base64 data or the path of a file inside of `base_dir` (the
/// directory of the document). Other files are not read, so that a document cannot embed
/// arbitrary files of the machine that renders it, and nothing is fetched from the network.
pub fn load(src: &str, base_dir: &Path) -> Result<LoadedImage, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err("the `src` attribute is empty".to_owned());
    }
    if let Some(rest) = strip_prefix_ignore_case(src, "data:") {
        let (header, payload) = rest
            .split_once(',')
            .ok_or_else(|| "a data URI needs a comma before its data".to_owned())?;
        let data = if header.to_ascii_lowercase().ends_with(";base64") {
            decode_base64(payload)?
        } else {
            // text, mostly SVG: percent-encoded
            percent_decode_bytes(payload)?
        };
        return LoadedImage::from_bytes(data, base_dir);
    }
    let path = resolve_path(src, base_dir)?;
    let data = std::fs::read(&path)
        .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
    LoadedImage::from_bytes(data, base_dir)
}

/// The file that a relative URL refers to, if it is inside of `base_dir`.
fn resolve_path(src: &str, base_dir: &Path) -> Result<PathBuf, String> {
    let src = src.trim();
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
    Ok(canonical)
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

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/images").join(name))
            .unwrap()
    }

    #[test]
    fn png_files_with_alpha_are_decoded() {
        let plain = RasterImage::from_bytes(fixture("alpha.png")).unwrap();
        assert_eq!((plain.width, plain.height), (40, 20));
        let image = plain.to_pdf_image().unwrap();
        assert!(image.has_alpha());
        // 16 bit samples are reduced to 8 bits, a palette with partial transparency is expanded
        for name in ["alpha16.png", "gray_alpha.png", "palette_alpha.png"] {
            let image = RasterImage::from_bytes(fixture(name)).unwrap().to_pdf_image().unwrap();
            assert!(image.has_alpha(), "{}", name);
        }
        // an opaque PNG that is embedded as it is has no soft mask
        assert!(!RasterImage::from_bytes(fixture("red.png")).unwrap().to_pdf_image().unwrap().has_alpha());
    }

    #[test]
    fn interlacing_does_not_change_the_pixels() {
        let plain = RasterImage::from_bytes(fixture("alpha.png")).unwrap().to_pdf_image().unwrap();
        let interlaced = RasterImage::from_bytes(fixture("alpha_interlaced.png"))
            .unwrap()
            .to_pdf_image()
            .unwrap();
        assert_eq!(plain, interlaced);
    }

    #[test]
    fn a_damaged_png_is_an_error() {
        let mut file = fixture("alpha.png");
        file.truncate(file.len() - 30);
        assert!(RasterImage::from_bytes(file).unwrap_err().contains("PNG"));
    }

    #[test]
    fn rejects_other_data() {
        let error = RasterImage::from_bytes(b"not an image".to_vec()).unwrap_err();
        assert!(error.contains("JPEG and PNG"), "{}", error);
        let error = RasterImage::from_bytes(vec![0x89, b'P', b'N', b'G', 0, 0]).unwrap_err();
        assert!(error.contains("PNG"), "{}", error);
    }

    #[test]
    fn rejects_other_places() {
        let dir = std::path::Path::new(".");
        assert!(load("https://example.com/a.jpg", dir).unwrap_err().contains("not supported"));
        assert!(load("file:///etc/passwd", dir).unwrap_err().contains("not supported"));
        assert!(load("", dir).is_err());
        assert!(load("data:image/png,abc", dir).unwrap_err().contains("JPEG, PNG and SVG"));
        assert!(load("../../etc/passwd", dir).is_err());
    }
}
