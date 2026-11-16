//! `url()` images as the layers of a background, checked by looking for the images in the PDF.

mod common;

/// A 4 x 4 pixel PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg==";

/// The text of the PDF of `html` (written to `dir/name.html`), with the content streams not
/// compressed, and the number of times that an image is painted in it.
fn render(dir: &std::path::Path, name: &str, html: &str) -> (String, usize) {
    std::fs::create_dir_all(dir).unwrap();
    let input = dir.join(format!("{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = dir.join(format!("{name}.pdf"));
    let result = common::helium()
        .unwrap()
        .arg("--no-compression")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let pdf = String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned();
    let painted = pdf.matches(" Do").count();
    (pdf, painted)
}

fn dir(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("background_images__{name}"))
}

fn page(style: &str) -> String {
    format!(
        r#"<!DOCTYPE html><style>@page {{ margin: 0 }} body {{ margin: 0 }}</style><div style="width: 100px; height: 60px; {style}"></div>"#
    )
}

#[test]
fn a_data_uri_is_painted_once_without_repeat() {
    let (pdf, painted) = render(&dir("data"), "once", &page(&format!("background: url(data:image/png;base64,{PNG}) no-repeat")));
    assert!(pdf.contains("/Subtype /Image"), "the image is embedded");
    assert_eq!(painted, 1);
}

#[test]
fn a_repeated_image_is_painted_in_every_tile() {
    // 100 x 60 box, 25 x 20 tiles: 4 x 3
    let (pdf, painted) = render(
        &dir("repeat"),
        "tiles",
        &page(&format!("background-image: url(data:image/png;base64,{PNG}); background-size: 25px 20px")),
    );
    assert_eq!(pdf.matches("/Subtype /Image").count(), 1, "the image is embedded once");
    assert_eq!(painted, 12);
}

#[test]
fn a_relative_url_in_a_style_sheet_is_relative_to_the_style_sheet() {
    let dir = dir("sheet");
    std::fs::create_dir_all(dir.join("css")).unwrap();
    std::fs::create_dir_all(dir.join("img")).unwrap();
    let png = {
        // decode the base64 by hand: no extra dependency for a test
        let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bytes = vec![];
        let (mut buffer, mut bits) = (0u32, 0);
        for c in PNG.bytes().filter(|&c| c != b'=') {
            buffer = (buffer << 6) | table.iter().position(|&t| t == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((buffer >> bits) as u8);
                buffer &= (1 << bits) - 1;
            }
        }
        bytes
    };
    std::fs::write(dir.join("img/dot.png"), png).unwrap();
    std::fs::write(dir.join("css/style.css"), "div { background: url(../img/dot.png) no-repeat }").unwrap();
    let html = r#"<!DOCTYPE html><link rel="stylesheet" href="css/style.css"><style>@page { margin: 0 } body { margin: 0 }</style><div style="width: 100px; height: 60px"></div>"#;
    let (pdf, painted) = render(&dir, "doc", html);
    assert!(pdf.contains("/Subtype /Image"));
    assert_eq!(painted, 1);
}

#[test]
fn a_file_outside_of_the_document_directory_is_not_read() {
    let (pdf, painted) = render(&dir("outside"), "outside", &page("background: url(../../../../../../etc/hostname) no-repeat"));
    assert!(!pdf.contains("/Subtype /Image"));
    assert_eq!(painted, 0);
}

#[test]
fn cover_scales_the_image_to_fill_the_box() {
    // the box is 100 x 60, so a 4 x 4 image is drawn 100 x 100: one tile, clipped to the box
    let (_, painted) = render(
        &dir("cover"),
        "cover",
        &page(&format!("background: url(data:image/png;base64,{PNG}) center / cover no-repeat")),
    );
    assert_eq!(painted, 1);
}

#[test]
fn the_background_of_the_page_and_of_the_canvas_are_painted() {
    let at_page = format!(
        r#"<!DOCTYPE html><style>@page {{ margin: 20px; background: url(data:image/png;base64,{PNG}) no-repeat }} body {{ margin: 0 }}</style><p>text</p>"#
    );
    let (_, painted) = render(&dir("page"), "page", &at_page);
    assert_eq!(painted, 1);
    // the background of the root element covers the whole canvas
    let canvas = format!(
        r#"<!DOCTYPE html><style>html {{ background: url(data:image/png;base64,{PNG}) no-repeat }} body {{ margin: 0 }}</style><p>text</p>"#
    );
    let (_, painted) = render(&dir("canvas"), "canvas", &canvas);
    assert_eq!(painted, 1);
}
