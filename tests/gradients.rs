//! CSS gradients as `background-image`: what is written to the PDF.

mod common;

fn render(name: &str, html: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("gradients__{}.html", name));
    let output = dir.join(format!("gradients__{}.pdf", name));
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
    String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(output).unwrap())).into_owned()
}

fn box_with(style: &str) -> String {
    format!("<style>div{{width:100px;height:50px}}</style><div style=\"{}\"></div>", style)
}

fn count(pdf: &str, token: &str) -> usize {
    pdf.split_whitespace().filter(|t| *t == token).count()
}

fn shading_types(pdf: &str) -> Vec<u32> {
    pdf.split("/ShadingType ")
        .skip(1)
        .map(|rest| rest.split_whitespace().next().unwrap().parse().unwrap())
        .collect()
}

#[test]
fn linear_gradient_is_an_axial_shading() {
    let pdf = render("linear", &box_with("background: linear-gradient(red, blue)"));
    assert_eq!(shading_types(&pdf), [2]);
    assert_eq!(count(&pdf, "sh"), 1);
    // painted inside of a clip to the box
    assert!(pdf.contains("W n"), "{}", pdf);
}

#[test]
fn the_longhand_and_the_shorthand_are_the_same() {
    let a = render("longhand", &box_with("background-image: linear-gradient(red, blue)"));
    assert_eq!(shading_types(&a), [2]);
    let b = render("shorthand", &box_with("background: linear-gradient(red, blue)"));
    assert_eq!(shading_types(&b), [2]);
}

#[test]
fn radial_gradient_is_a_radial_shading() {
    let pdf = render("radial", &box_with("background: radial-gradient(circle at 20% 30%, red, blue)"));
    assert_eq!(shading_types(&pdf), [3]);
    assert_eq!(count(&pdf, "sh"), 1);
}

#[test]
fn conic_gradient_is_a_patch_mesh() {
    let pdf = render("conic", &box_with("background: conic-gradient(red, yellow, blue, red)"));
    assert_eq!(shading_types(&pdf), [6]);
}

#[test]
fn layers_are_painted_in_reverse_order() {
    let pdf = render(
        "layers",
        &box_with("background: radial-gradient(red, blue), linear-gradient(green, yellow)"),
    );
    // the last layer is at the bottom, so it is painted first
    assert_eq!(shading_types(&pdf), [2, 3]);
    assert_eq!(count(&pdf, "sh"), 2);
}

#[test]
fn the_color_is_under_the_gradient() {
    let pdf = render("color", &box_with("background: #fed linear-gradient(red, blue)"));
    assert_eq!(count(&pdf, "sh"), 1);
    // the fill of the colour comes before the shading
    let fill = pdf.find(" f EMC").expect("a fill");
    let shading = pdf.find(" sh").expect("a shading");
    assert!(fill < shading);
}

#[test]
fn unsupported_gradients_are_ignored() {
    // a colour hint, a single stop and an unknown function are not valid here
    for (name, image) in [
        ("hint", "linear-gradient(red, 30%, blue)"),
        ("single", "linear-gradient(red)"),
        ("unknown", "diagonal-gradient(red, blue)"),
    ] {
        let pdf = render(name, &box_with(&format!("background: #fed {}", image)));
        assert!(shading_types(&pdf).is_empty(), "{}", name);
        // the whole declaration is invalid, so there is no background colour either
        assert_eq!(count(&pdf, "sh"), 0);
    }
}

#[test]
fn none_has_no_image() {
    let pdf = render("none", &box_with("background-image: none; background-color: red"));
    assert!(shading_types(&pdf).is_empty());
}

#[test]
fn rounded_boxes_clip_the_gradient_to_the_curve() {
    let pdf = render(
        "rounded",
        &box_with("background: linear-gradient(red, blue); border-radius: 10px"),
    );
    assert_eq!(shading_types(&pdf), [2]);
    // four curves for the clip
    assert_eq!(count(&pdf, "c"), 4);
}

#[test]
fn one_opacity_is_set_in_the_graphics_state() {
    let pdf = render(
        "alpha",
        &box_with("background: linear-gradient(rgba(255,0,0,.5), rgba(0,0,255,.5))"),
    );
    assert!(pdf.contains("/ca 0.5"), "{}", pdf);
    assert!(!pdf.contains("/SMask"), "{}", pdf);
}

#[test]
fn different_opacities_use_a_soft_mask() {
    let pdf = render("fade", &box_with("background: linear-gradient(to right, red, transparent)"));
    assert!(pdf.contains("/SMask"), "{}", pdf);
    assert!(pdf.contains("/Luminosity"), "{}", pdf);
    assert!(pdf.contains("/Subtype /Form"), "{}", pdf);
    // the colour gradient and the gradient of the mask
    assert_eq!(shading_types(&pdf), [2, 2]);
    assert!(pdf.contains("/DeviceGray"));
}

#[test]
fn opaque_gradients_have_no_soft_mask() {
    let pdf = render("opaque", &box_with("background: linear-gradient(red, blue)"));
    assert!(!pdf.contains("/SMask"));
    assert!(!pdf.contains("/ca "));
}

#[test]
fn currentcolor_is_the_text_color() {
    let pdf = render(
        "currentcolor",
        &box_with("color: rgb(0,255,0); background: linear-gradient(currentcolor, rgb(0,0,255))"),
    );
    assert!(pdf.contains("/C0 [ 0 1 0 ]"), "{}", pdf);
    assert!(pdf.contains("/C1 [ 0 0 1 ]"), "{}", pdf);
}

#[test]
fn empty_boxes_paint_nothing() {
    let pdf = render(
        "empty",
        "<div style=\"height:0;background:linear-gradient(red,blue)\"></div><p>x</p>",
    );
    assert!(shading_types(&pdf).is_empty());
}

#[test]
fn a_box_on_two_pages_has_a_gradient_on_each() {
    // taller than a page: 60 lines of 20px
    let lines = "<div style=\"height: 20px\"></div>".repeat(60);
    let pdf = render(
        "fragmented",
        &format!(
            "<style>body {{ margin: 0 }}</style><div style=\"background: linear-gradient(red, blue)\">{}</div>",
            lines
        ),
    );
    assert_eq!(count(&pdf, "sh"), 2);
}
