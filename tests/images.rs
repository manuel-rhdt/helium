//! `<img>`: JPEG and PNG files are embedded without decoding, SVG files are drawn as vector
//! graphics, and all are laid out as replaced boxes.

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
    let out = common::umbruch()
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
        ("not_a_jpeg", "not_a_jpeg.jpg", "JPEG, PNG and SVG"),
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
fn png_files_with_alpha_become_soft_masks() {
    let r = render(
        "png_alpha",
        &format!(
            "{PAGE}<img src=\"alpha.png\"><img src=\"gray_alpha.png\"><img src=\"alpha16.png\">\
             <img src=\"palette_alpha.png\">"
        ),
    );
    assert_eq!(r.images.len(), 4, "{}", r.log);
    for image in &r.images {
        assert_close(image[2], 30.0);
        assert_close(image[3], 15.0);
    }
    // the pixels are decoded: no PNG predictor, an image and a soft mask for each
    assert_eq!(count(&r.pdf, "/Predictor"), 0, "{}", r.log);
    assert_eq!(count(&r.pdf, "/SMask"), 4, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 8);
}

#[test]
fn interlaced_png_files_are_decoded() {
    let r = render("png_interlaced", &format!("{PAGE}<img src=\"interlaced.png\">"));
    assert_eq!(r.images.len(), 1, "{}", r.log);
    assert_close(r.images[0][2], 30.0);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Predictor"), 0);
    assert_eq!(count(&r.pdf, "/SMask"), 0);
}

#[test]
fn a_png_that_is_damaged_is_an_empty_box() {
    // the data of the file stops in the middle
    let mut file = std::fs::read(fixtures().join("alpha.png")).unwrap();
    file.truncate(file.len() - 30);
    let encoded = encode_base64(&file);
    let r = render(
        "png_damaged",
        &format!("{PAGE}<p>a<img src=\"data:image/png;base64,{encoded}\">b</p>"),
    );
    assert!(r.log.contains("PNG"), "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 0);
    assert_close(r.images[0][2], 0.0);
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

/// An SVG document that is 40 x 20 pixels, with the shapes of `body`.
fn svg(body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" viewBox=\"0 0 40 20\">{body}</svg>"
    )
}

/// A `data:` URI of an SVG document, percent-encoded.
fn svg_uri(document: &str) -> String {
    let mut out = String::from("data:image/svg+xml,");
    for byte in document.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~/ =:".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

#[test]
fn an_svg_is_a_form_of_its_size() {
    let r = render("svg_size", &format!("{PAGE}<img src=\"vector.svg\">"));
    assert_eq!(r.images.len(), 1, "{}", r.log);
    // 40 x 20 pixels
    assert_close(r.images[0][2], 30.0);
    assert_close(r.images[0][3], 15.0);
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 0);
    // the gradient is vector graphics too
    assert_eq!(count(&r.pdf, "/ShadingType 2"), 1);
    // the form is painted with the y axis flipped, 40 x 20 pixels are 30 x 15 points
    assert!(r.pdf.contains("0.75 0 0 -0.75"), "{}", r.pdf);
}

#[test]
fn an_svg_follows_the_proportions_like_other_images() {
    let r = render(
        "svg_ratio",
        &format!(
            "{PAGE}<img src=\"vector.svg\" style=\"width:60px\">\
             <img src=\"vector.svg\" style=\"height:10px\">"
        ),
    );
    assert_close(r.images[0][2], 45.0);
    assert_close(r.images[0][3], 22.5);
    assert_close(r.images[1][2], 15.0);
    assert_close(r.images[1][3], 7.5);
    // the form is written once
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 1);
}

#[test]
fn an_svg_is_centered_in_a_box_of_another_shape() {
    // 40 x 20 pixels in a box of 20 x 20 pixels: half the scale, and centered vertically
    let r = render(
        "svg_fit",
        &format!("{PAGE}<img src=\"vector.svg\" style=\"width:20px;height:20px\">"),
    );
    assert_close(r.images[0][2], 15.0);
    assert_close(r.images[0][3], 15.0);
    assert!(r.pdf.contains("0.38 0 0 -0.38"), "{}", r.pdf);
}

#[test]
fn svg_data_uris() {
    let document = svg("<rect width=\"40\" height=\"20\" fill=\"#c00\"/>");
    let encoded = encode_base64(document.as_bytes());
    for uri in [svg_uri(&document), format!("data:image/svg+xml;base64,{encoded}")] {
        let r = render("svg_data", &format!("{PAGE}<img src=\"{uri}\">"));
        assert_eq!(r.images.len(), 1, "{}", r.log);
        assert_close(r.images[0][2], 30.0);
        assert_eq!(count(&r.pdf, "/Subtype /Form"), 1, "{}", r.log);
        assert!(r.pdf.contains("0.8 0 0 rg"), "{}", r.pdf);
    }
}

#[test]
fn an_svg_that_is_not_valid_is_an_empty_box() {
    let r = render("svg_invalid", &format!("{PAGE}<p>a<img src=\"{}\">b</p>", svg_uri("<svg><rect")));
    assert!(r.log.contains("invalid SVG"), "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 0);
    assert_close(r.images[0][2], 0.0);
}

#[test]
fn what_an_svg_draws() {
    let body = "\
        <defs>\
          <linearGradient id=\"r\" gradientUnits=\"userSpaceOnUse\" x2=\"10\" spreadMethod=\"repeat\">\
            <stop offset=\"0\" stop-color=\"#000\"/><stop offset=\"1\" stop-color=\"#fff\"/></linearGradient>\
          <radialGradient id=\"q\"><stop offset=\"0\" stop-color=\"#fff\"/><stop offset=\"1\" stop-color=\"#000\"/></radialGradient>\
          <clipPath id=\"c\"><circle cx=\"10\" cy=\"10\" r=\"8\"/></clipPath>\
          <mask id=\"m\"><rect width=\"40\" height=\"20\" fill=\"#888\"/></mask>\
          <pattern id=\"p\" width=\"4\" height=\"4\" patternUnits=\"userSpaceOnUse\"><rect width=\"2\" height=\"2\"/></pattern>\
        </defs>\
        <rect width=\"20\" height=\"10\" fill=\"url(#r)\"/>\
        <rect y=\"10\" width=\"20\" height=\"10\" fill=\"url(#q)\"/>\
        <rect x=\"20\" width=\"20\" height=\"10\" fill=\"url(#p)\" clip-path=\"url(#c)\"/>\
        <g opacity=\"0.5\" mask=\"url(#m)\"><rect x=\"20\" y=\"10\" width=\"20\" height=\"10\" stroke=\"#00f\" stroke-dasharray=\"2 1\" fill=\"#ff0\"/></g>\
        <text x=\"1\" y=\"19\" font-size=\"6\" font-family=\"sans-serif\">Aa</text>";
    let r = render("svg_features", &format!("{PAGE}<img src=\"{}\">", svg_uri(&svg(body))));
    assert_eq!(r.images.len(), 1, "{}", r.log);
    // two gradients, repeated and radial
    assert_eq!(count(&r.pdf, "/ShadingType 2"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/ShadingType 3"), 1);
    assert_eq!(count(&r.pdf, "/PatternType 1"), 1);
    // the clip path
    assert!(r.pdf.contains("W n"), "{}", r.pdf);
    // the opacity of the group applies to the group as a whole, through a transparency group (the
    // path has a fill and a stroke, which overlap)
    assert!(r.pdf.contains("/S /Transparency"));
    assert!(r.pdf.contains("/ca 0.5"));
    assert!(r.pdf.contains("/Luminosity"));
    assert!(r.pdf.contains("[ 2 1 ] 0 d"), "{}", r.pdf);
    // text is drawn as outlines: nothing asks for a font
    assert_eq!(count(&r.pdf, "/Type /Font"), 0, "{}", r.log);
}

#[test]
fn an_svg_cannot_read_what_a_document_cannot() {
    let r = render("svg_sandbox", &format!("{PAGE}<img src=\"nested.svg\">"));
    // the image inside of the directory is embedded (once, although it is used twice)
    assert_eq!(count(&r.pdf, "/DCTDecode"), 1, "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Image"), 1);
    assert!(r.log.contains("outside of the directory"), "{}", r.log);
    assert!(r.log.contains("not supported"), "{}", r.log);
}

#[test]
fn filters_are_ignored_with_a_warning() {
    let body = "<filter id=\"f\"><feGaussianBlur stdDeviation=\"2\"/></filter>\
                <rect width=\"20\" height=\"10\" filter=\"url(#f)\"/>";
    let r = render("svg_filter", &format!("{PAGE}<img src=\"{}\">", svg_uri(&svg(body))));
    assert!(r.log.contains("filters are not supported"), "{}", r.log);
    assert_eq!(count(&r.pdf, "/Subtype /Form"), 1);
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
