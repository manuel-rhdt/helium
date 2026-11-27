//! The ICC profile of an embedded JPEG or PNG file is the colour space of the image in the PDF; an
//! image without a profile, or with one that cannot be used for it, keeps its device colours (which
//! `DefaultRGB` defines as sRGB).

mod common;

use assert_cmd::prelude::*;
use std::path::{Path, PathBuf};

const FILES: [&str; 5] = ["p3-profile.jpg", "p3-profile.png", "p3-profile-alpha.png", "gray-with-rgb-profile.png", "bad-profile.png"];

fn render(name: &str, images: &[&str], args: &[&str]) -> (String, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("image_profiles_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    for file in FILES.iter().chain(&["rgb.jpg"]) {
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(file), dir.join(file)).unwrap();
    }
    let html: String = images.iter().map(|image| format!("<img src=\"{image}\" width=40 height=40>")).collect();
    std::fs::write(dir.join("index.html"), format!("<!doctype html><html lang=en><title>Profiles</title>{html}")).unwrap();
    let output = dir.join("out.pdf");
    common::umbruch().unwrap().args(["--no-system-fonts", "--no-compression"]).args(args).arg(dir.join("index.html")).arg(&output).assert().success();
    (String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(&output).unwrap())).into_owned(), output)
}

/// Like `render`, but the images are `<image>` elements of an SVG file (`img.svg`, shown by an `<img>`)
/// or of an inline `<svg>`.
fn render_svg(name: &str, images: &[&str], inline: bool, args: &[&str]) -> (String, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("image_profiles_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    for file in FILES.iter().chain(&["rgb.jpg", "cmyk.jpg"]) {
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(file), dir.join(file)).unwrap();
    }
    let elements: String = images.iter().map(|image| format!("<image href=\"{image}\" width=\"40\" height=\"40\"/>")).collect();
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"40\" height=\"{}\" viewBox=\"0 0 40 {}\">{elements}</svg>", 40, 40);
    let body = if inline {
        svg.clone()
    } else {
        std::fs::write(dir.join("img.svg"), &svg).unwrap();
        "<img src=\"img.svg\" width=40 height=40>".to_owned()
    };
    std::fs::write(dir.join("index.html"), format!("<!doctype html><html lang=en><title>Profiles</title>{body}")).unwrap();
    let output = dir.join("out.pdf");
    common::umbruch().unwrap().args(["--no-system-fonts", "--no-compression"]).args(args).arg(dir.join("index.html")).arg(&output).assert().success();
    (String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(&output).unwrap())).into_owned(), output)
}

/// The images (their dictionaries) that have an ICC profile as their colour space.
fn icc_images(pdf: &str) -> usize {
    pdf.matches("/ColorSpace [ /ICCBased").count()
}

#[test]
fn a_jpeg_and_pngs_with_a_profile_have_it() {
    let (pdf, _) = render("have", &["p3-profile.jpg", "p3-profile.png", "p3-profile-alpha.png"], &[]);
    assert_eq!(icc_images(&pdf), 3, "{}", pdf.lines().filter(|line| line.contains("ColorSpace")).collect::<Vec<_>>().join("\n"));
    // the profile of the three images is written once, besides the sRGB profile of the page
    assert_eq!(pdf.matches("/N 3").count(), 2);
}

#[test]
fn an_image_without_a_profile_keeps_its_device_colors() {
    let (pdf, _) = render("without", &["rgb.jpg"], &[]);
    assert_eq!(icc_images(&pdf), 0);
    assert!(pdf.contains("/ColorSpace /DeviceRGB"));
}

#[test]
fn a_profile_that_does_not_fit_is_ignored() {
    // an RGB profile on a gray image, and a profile with a wrong signature
    let (pdf, _) = render("misfit", &["gray-with-rgb-profile.png", "bad-profile.png"], &[]);
    assert_eq!(icc_images(&pdf), 0);
    assert!(pdf.contains("/ColorSpace /DeviceGray"));
    assert!(pdf.contains("/ColorSpace /DeviceRGB"));
}

#[test]
fn images_with_a_profile_conform_to_pdfa() {
    let Some(lib) = std::env::var_os("VERAPDF_LIB").map(PathBuf::from).filter(|lib| lib.is_dir()) else {
        eprintln!("skipped: VERAPDF_LIB is not set (see tools/fetch_verapdf.sh)");
        return;
    };
    for level in ["2b", "3a"] {
        let (_, output) = render(&format!("pdfa{level}"), &["p3-profile.jpg", "p3-profile.png", "p3-profile-alpha.png"], &["--pdfa", level]);
        let result = std::process::Command::new("java")
            .arg("-cp")
            .arg(format!("{}/*", lib.display()))
            .arg("org.verapdf.apps.GreenfieldCliWrapper")
            .args(["--flavour", level, "--format", "text"])
            .arg(&output)
            .output()
            .expect("cannot run java");
        let text = String::from_utf8_lossy(&result.stdout);
        assert!(text.lines().any(|line| line.starts_with("PASS")), "{level}: {text}");
    }
}

#[test]
fn svg_images_with_a_profile_have_it() {
    for inline in [false, true] {
        let (pdf, _) = render_svg(&format!("svg_have_{inline}"), &["p3-profile.jpg", "p3-profile.png", "p3-profile-alpha.png"], inline, &[]);
        assert_eq!(icc_images(&pdf), 3, "{}", pdf.lines().filter(|line| line.contains("ColorSpace")).collect::<Vec<_>>().join("\n"));
        // the profile is written once, besides the sRGB profile of the page
        assert_eq!(pdf.matches("/N 3").count(), 2);
    }
}

#[test]
fn svg_images_without_or_with_a_misfit_profile_keep_device_colors() {
    let (pdf, _) = render_svg("svg_without", &["rgb.jpg"], false, &[]);
    assert_eq!(icc_images(&pdf), 0);
    assert!(pdf.contains("/ColorSpace /DeviceRGB"));
    let (pdf, _) = render_svg("svg_misfit", &["gray-with-rgb-profile.png", "bad-profile.png"], false, &[]);
    assert_eq!(icc_images(&pdf), 0);
    assert!(pdf.contains("/ColorSpace /DeviceGray"));
}

#[test]
fn a_cmyk_image_in_an_svg_is_embedded_as_it_is() {
    let (pdf, _) = render_svg("svg_cmyk", &["cmyk.jpg"], false, &[]);
    assert!(pdf.contains("/ColorSpace /DeviceCMYK"), "{}", pdf.lines().filter(|line| line.contains("ColorSpace")).collect::<Vec<_>>().join("\n"));
}

#[test]
fn a_cmyk_image_in_an_svg_is_an_error_for_pdfa() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("image_profiles_svg_cmyk_pdfa");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/cmyk.jpg"), dir.join("cmyk.jpg")).unwrap();
    std::fs::write(dir.join("img.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"40\"><image href=\"cmyk.jpg\" width=\"40\" height=\"40\"/></svg>").unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><html lang=en><title>T</title><img src=\"img.svg\" width=40 height=40>").unwrap();
    common::umbruch().unwrap().args(["--no-system-fonts", "--pdfa", "2b"]).arg(dir.join("index.html")).arg(dir.join("out.pdf")).assert().failure();
}
