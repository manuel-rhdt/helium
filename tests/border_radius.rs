//! `border-radius`: the parsing of the shorthand and the paths that are written to the PDF.

mod common;

fn render(name: &str, html: &str) -> (String, String) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("border_radius__{}.html", name));
    let output = dir.join(format!("border_radius__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{}", log);
    let pdf = String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned();
    (pdf, log)
}

fn count_ops(pdf: &str, op: &str) -> usize {
    pdf.split_whitespace().filter(|token| *token == op).count()
}

#[test]
fn square_boxes_have_no_curves() {
    let (pdf, _) = render(
        "square",
        "<style>div{width:50px;height:30px;background:#123;border:2px solid red}</style><div></div>",
    );
    assert_eq!(count_ops(&pdf, "c"), 0);
}

#[test]
fn rounded_background_has_four_curves() {
    let (pdf, log) = render(
        "background",
        "<style>div{width:50px;height:30px;background:#123;border-radius:8px}</style><div></div>",
    );
    assert!(!log.contains("Error while parsing"), "{}", log);
    assert_eq!(count_ops(&pdf, "c"), 4, "{}", pdf);
}

#[test]
fn uniform_border_is_one_ring() {
    // outer and inner outline, four curves each
    let (pdf, _) = render(
        "ring",
        "<style>div{width:50px;height:30px;border:3px solid #000;border-radius:8px}</style><div></div>",
    );
    assert_eq!(count_ops(&pdf, "c"), 8);
    assert_eq!(count_ops(&pdf, "f*"), 1);
}

#[test]
fn mixed_borders_clip_each_side() {
    let (pdf, _) = render(
        "mixed",
        "<style>div{width:50px;height:30px;border:3px solid #000;border-top-color:red;border-radius:8px}</style><div></div>",
    );
    assert_eq!(count_ops(&pdf, "W*"), 4);
}

#[test]
fn radius_shorthand_expands_like_margin() {
    // 1 value: all corners; 2: tl/br and tr/bl; 3 and 4: in order
    let (pdf, log) = render(
        "shorthand",
        "<style>div{width:50px;height:30px;background:#123;border-radius:0 10px}</style><div></div>",
    );
    assert!(!log.contains("Error while parsing"), "{}", log);
    // only the top-right and bottom-left corners are curved, but all four are still emitted
    assert_eq!(count_ops(&pdf, "c"), 4);
}

#[test]
fn zero_radius_is_painted_as_a_rect() {
    let (pdf, _) = render(
        "zero",
        "<style>div{width:50px;height:30px;background:#123;border-radius:0}</style><div></div>",
    );
    assert_eq!(count_ops(&pdf, "c"), 0);
}

#[test]
fn overlapping_radii_are_scaled_down() {
    // must not produce NaN or negative sizes
    let (pdf, _) = render(
        "overlap",
        "<style>div{width:20px;height:20px;background:#123;border-radius:1000px}</style><div></div>",
    );
    assert!(!pdf.contains("NaN") && !pdf.contains("inf"));
    assert_eq!(count_ops(&pdf, "c"), 4);
}
