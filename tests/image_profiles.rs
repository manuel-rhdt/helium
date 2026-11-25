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
