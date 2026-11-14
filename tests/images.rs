//! `<img>`: JPEG and PNG files are embedded without decoding and laid out as replaced boxes.

mod common;

use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/images")
}

struct Rendered {
    /// The PDF with uncompressed content streams, as text.
    pdf: String,
    /// The border boxes (x, y, width, height in points) of the `<img>` elements in tree order.
    images: Vec<[f64; 4]>,
    /// The (zero-based) page of each of them.
    pages: Vec<u64>,
    log: String,
}

fn boxes_of(node: &serde_json::Value, out: &mut Vec<[f64; 4]>, pages: &mut Vec<u64>) {
    if node["tag"] == "img" {
        let b = &node["border_box"];
        let get = |key: &str| b[key].as_f64().unwrap() * 0.75;
        out.push([get("x"), get("y"), get("width"), get("height")]);
        pages.push(node["page"].as_u64().unwrap_or(0));
    }
    for child in node["children"].as_array().into_iter().flatten() {
        boxes_of(child, out, pages);
    }
}

/// Renders `html`, whose `src` attributes are relative to `tests/images`.
fn render(name: &str, html: &str) -> Rendered {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = fixtures().join(format!("_{}.html", name));
    let output = dir.join(format!("images__{}.pdf", name));
    let layout = dir.join(format!("images__{}.json", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts", "--dump-layout"])
        .arg(&layout)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    std::fs::remove_file(&input).unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{}", log);
    let dump: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout).unwrap()).unwrap();
    let mut images = vec![];
    let mut pages = vec![];
    boxes_of(&dump["document"], &mut images, &mut pages);
    Rendered {
        pdf: String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned(),
        images,
        pages,
        log,
    }
}

const PAGE: &str = "<style>@page{width:300pt;height:300pt;margin:20pt}</style>";

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 0.1, "{} != {}", actual, expected);
}

fn count(pdf: &str, needle: &str) -> usize {
    pdf.matches(needle).count()
}

#[test]
fn an_image_is_as_big_as_its_pixels() {
    // 40 x 20 pixels are 30 x 15 points
    let r = render("natural", &format!("{PAGE}<img src=\"red.jpg\">"));
    assert_eq!(r.images.len(), 1);
    assert_close(r.images[0][2], 30.0);
    assert_close(r.images[0][3], 15.0);
    assert_eq!(count(&r.pdf, "/DCTDecode"), 1);
}

#[test]
fn the_other_side_follows_the_proportions() {
    let r = render(
        "ratio",
        &format!(
            "{PAGE}<img src=\"red.jpg\" style=\"width:60px\">\
             <img src=\"red.jpg\" style=\"height:10px\">\
             <img src=\"red.jpg\" width=\"20\">\
             <img src=\"red.jpg\" height=\"10\">\
             <img src=\"red.jpg\" width=\"20\" height=\"20\">\
             <img src=\"red.jpg\" style=\"display:block\">"
        ),
    );
    let sizes: Vec<_> = r.images.iter().map(|b| (b[2], b[3])).collect();
    let expected = [
        (45.0, 22.5),
        (15.0, 7.5),
        (15.0, 7.5),
        (15.0, 7.5),
        (15.0, 15.0),
        (30.0, 15.0),
    ];
    assert_eq!(sizes.len(), expected.len(), "{:?}", sizes);
    for (actual, expected) in sizes.iter().zip(expected) {
        assert_close(actual.0, expected.0);
        assert_close(actual.1, expected.1);
    }
}

#[test]
fn css_inline_and_block_images() {
    let r = render(
        "display",
        &format!(
            "{PAGE}<p>a<img src=\"red.jpg\" style=\"display:inline\">b</p>\
             <img src=\"red.jpg\" style=\"display:block;max-width:10px\">"
        ),
    );
    assert_eq!(r.images.len(), 2);
    assert_close(r.images[0][2], 30.0);
    // `max-width` clips the width and the proportions follow
    assert_close(r.images[1][2], 7.5);
    assert_close(r.images[1][3], 3.75);
}

#[test]
fn an_image_sits_on_the_baseline() {
    let r = render("baseline", &format!("{PAGE}<p>text<img src=\"red.jpg\">text</p>"));
    // the bottom of the image is the baseline of the line, so the paragraph is taller than the
    // image but the image is not centered in it
    let [_, y, _, h] = r.images[0];
    assert!(y + h > 20.0 + 15.0, "bottom of the image at {}", y + h);
}

#[test]
fn every_image_file_is_embedded_once() {
    let r = render(
        "dedupe",
        &format!(
            "{PAGE}<img src=\"red.jpg\"><img src=\"red.jpg\"><img src=\"./red.jpg\">\
             <img src=\"gray.jpg\">"
        ),
    );
    // the file is written once, however often it is painted; `./red.jpg` is another name
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 3, "{}", r.log);
    assert_eq!(count(&r.pdf, " Do "), 4);
}

#[test]
fn jpeg_color_spaces() {
    let r = render(
        "spaces",
        &format!(
            "{PAGE}<img src=\"red.jpg\"><img src=\"gray.jpg\"><img src=\"cmyk.jpg\">\
             <img src=\"progressive.jpg\">"
        ),
    );
    assert_eq!(r.images.len(), 4);
    assert_eq!(count(&r.pdf, "/DeviceRGB"), 2, "{}", r.log);
    assert_eq!(count(&r.pdf, "/DeviceGray"), 1);
    assert_eq!(count(&r.pdf, "/DeviceCMYK"), 1);
}

#[test]
fn the_file_is_embedded_unchanged() {
    let file = std::fs::read(fixtures().join("red.jpg")).unwrap();
    let r = render("unchanged", &format!("{PAGE}<img src=\"red.jpg\">"));
    let pdf = std::fs::read(
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("images__unchanged.pdf"),
    )
    .unwrap();
    assert!(
        pdf.windows(file.len()).any(|window| window == file.as_slice()),
        "the JPEG file is not in the PDF\n{}",
        r.log
    );
}

#[test]
fn data_uris() {
    let file = std::fs::read(fixtures().join("red.jpg")).unwrap();
    let encoded = encode_base64(&file);
    let r = render(
        "data",
        &format!("{PAGE}<img src=\"data:image/jpeg;base64,{encoded}\">"),
    );
    assert_eq!(r.images.len(), 1);
    assert_close(r.images[0][2], 30.0);
    assert_eq!(count(&r.pdf, "/DCTDecode"), 1);
}

#[test]
fn an_image_that_cannot_be_used_is_an_empty_box() {
    for (name, src, message) in [
        ("missing", "nothing.jpg", "cannot read"),
        ("escape", "../../Cargo.toml", "outside of the directory"),
        ("remote", "https://example.com/a.jpg", "not supported"),
        ("not_a_jpeg", "not_a_jpeg.jpg", "JPEG and PNG"),
    ] {
        let r = render(name, &format!("{PAGE}<p>a<img src=\"{src}\">b</p>"));
        assert!(r.log.contains(message), "{}: {}", name, r.log);
        assert_eq!(count(&r.pdf, "/Subtype /Image"), 0, "{}", name);
        assert_close(r.images[0][2], 0.0);
        assert_close(r.images[0][3], 0.0);
    }
}

#[test]
fn png_files_are_embedded_without_decoding() {
    let r = render(
        "png",
        &format!("{PAGE}<img src=\"red.png\"><img src=\"gray.png\"><img src=\"palette.png\">"),
    );
    assert_eq!(r.images.len(), 3, "{}", r.log);
    for image in &r.images {
        // 40 x 20 pixels
        assert_close(image[2], 30.0);
        assert_close(image[3], 15.0);
    }
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 3, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Predictor 15"), 3);
    assert_eq!(count(&r.pdf, "/DCTDecode"), 0);
    assert_eq!(count(&r.pdf, "/Indexed /DeviceRGB 2"), 1);
    // the transparent palette entry
    assert_eq!(count(&r.pdf, "/Mask [ 0 0 ]"), 1);
}

#[test]
fn the_file_type_is_not_taken_from_the_name() {
    // a PNG called `.jpg` and a JPEG called `.png`
    let dir = fixtures();
    std::fs::copy(dir.join("red.png"), dir.join("_png_as.jpg")).unwrap();
    std::fs::copy(dir.join("red.jpg"), dir.join("_jpg_as.png")).unwrap();
    let r = render("sniff", &format!("{PAGE}<img src=\"_png_as.jpg\"><img src=\"_jpg_as.png\">"));
    std::fs::remove_file(dir.join("_png_as.jpg")).unwrap();
    std::fs::remove_file(dir.join("_jpg_as.png")).unwrap();
    assert_eq!(count(&r.pdf, "/Predictor 15"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/DCTDecode"), 1);
}

#[test]
fn png_files_that_need_decoding_are_empty_boxes() {
    for (name, src, message) in [
        ("alpha", "alpha.png", "alpha channel"),
        ("interlaced", "interlaced.png", "interlaced"),
    ] {
        let r = render(name, &format!("{PAGE}<p>a<img src=\"{src}\">b</p>"));
        assert!(r.log.contains(message), "{}: {}", name, r.log);
        assert_eq!(count(&r.pdf, "/Subtype /Image"), 0, "{}", name);
        assert_close(r.images[0][2], 0.0);
    }
}

#[test]
fn an_image_that_does_not_fit_moves_to_the_next_page() {
    let r = render(
        "pages",
        &format!(
            "{PAGE}<img src=\"red.jpg\" style=\"display:block;width:200px;height:150pt\">\
             <img src=\"red.jpg\" style=\"display:block;width:200px;height:150pt\">"
        ),
    );
    assert_eq!(r.pages, [0, 1]);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 1);
    assert_eq!(count(&r.pdf, " Do "), 2);
}

fn encode_base64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
