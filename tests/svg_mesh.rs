//! Gouraud triangle meshes in SVG (`<g data-umbruch-mesh="gouraud">`): one type 4 shading per
//! group, with the flat triangles as what a renderer without the extension shows.

mod common;

use std::process::Command;

const NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";

struct Out {
    pdf: String,
    raw: Vec<u8>,
    log: String,
    path: std::path::PathBuf,
}

fn render(name: &str, svg: &str) -> Out {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("svg_mesh__{}.svg", name));
    let output = dir.join(format!("svg_mesh__{}.pdf", name));
    std::fs::write(&input, svg).unwrap();
    let out = common::umbruch()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{}", log);
    let raw = std::fs::read(&output).unwrap();
    let pdf = String::from_utf8_lossy(&common::expand_object_streams(&raw)).into_owned();
    Out { pdf, raw, log, path: output }
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

/// A triangle of the mesh, with the colors `a`, `b` and `c` (`r,g,b,a`) at its vertices.
fn tri(points: [(f64, f64); 3], colors: [&str; 3], fill: &str) -> String {
    format!(
        "<path d=\"M{} {}L{} {}L{} {}Z\" fill=\"{fill}\" stroke=\"none\" data-colors=\"{}\"/>",
        points[0].0, points[0].1, points[1].0, points[1].1, points[2].0, points[2].1,
        colors.join(";"),
    )
}

fn mesh(attr: &str, body: &str) -> String {
    format!("<svg {NS} width=\"100\" height=\"100\"><g {attr}>{body}</g></svg>")
}

const GOURAUD: &str = "data-umbruch-mesh=\"gouraud\"";
const RED: &str = "1,0,0,1";
const GREEN: &str = "0,1,0,1";
const BLUE: &str = "0,0,1,1";

fn three_triangles() -> String {
    [
        tri([(10.0, 10.0), (50.0, 10.0), (10.0, 50.0)], [RED, GREEN, BLUE], "#555"),
        tri([(60.0, 10.0), (90.0, 10.0), (60.0, 40.0)], [GREEN, BLUE, RED], "#555"),
        tri([(10.0, 60.0), (40.0, 90.0), (10.0, 90.0)], [BLUE, RED, GREEN], "#555"),
    ]
    .concat()
}

/// The pixel (r, g, b) at (x, y) of the first page, rendered at 72 dpi (one pixel per point), where
/// the 100 x 100 pixels of the SVG are 75 x 75 pixels.
fn pixel(pdf: &std::path::Path, x: usize, y: usize) -> Option<[u8; 3]> {
    let out = Command::new("pdftoppm").args(["-r", "72", "-singlefile", "-aa", "no", "-aaVector", "no"]).arg(pdf).output().ok()?;
    let ppm = out.stdout;
    // P6 header: magic, width, height, maxval
    let mut parts = ppm.splitn(5, |b| b.is_ascii_whitespace());
    parts.next()?;
    let width: usize = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    parts.next()?;
    parts.next()?;
    let data = parts.next()?;
    let at = (y * width + x) * 3;
    Some([data[at], data[at + 1], data[at + 2]])
}

fn has_pdftoppm() -> bool {
    Command::new("pdftoppm").arg("-v").output().is_ok()
}

fn close(a: [u8; 3], b: [f64; 3], tolerance: f64) -> bool {
    a.iter().zip(b).all(|(&a, b)| (f64::from(a) - b * 255.0).abs() <= tolerance)
}

#[test]
fn the_group_is_one_type_4_shading() {
    let r = render("one_shading", &mesh(GOURAUD, &three_triangles()));
    assert_eq!(shading_types(&r.pdf), [4], "{}", r.log);
    assert_eq!(count(&r.pdf, "sh"), 1, "{}", r.pdf);
    // no flat triangle is drawn as well
    assert_eq!(count(&r.pdf, "f"), 0, "{}", r.pdf);
    assert!(!r.log.contains("Gouraud") && !r.log.contains("data-umbruch-mesh"), "{}", r.log);
}

#[test]
fn without_the_attribute_the_triangles_are_flat() {
    let r = render("flat", &mesh("", &three_triangles()));
    assert!(shading_types(&r.pdf).is_empty(), "{}", r.pdf);
    assert_eq!(count(&r.pdf, "sh"), 0);
    assert_eq!(count(&r.pdf, "f"), 3, "{}", r.pdf);
}

#[test]
fn the_colors_are_interpolated() {
    if !has_pdftoppm() {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    }
    let r = render("pixels", &mesh(GOURAUD, &three_triangles()));
    // the centroid of the first triangle ((10,10), (50,10), (10,50) in pixels; 0.75 points per pixel)
    let (cx, cy) = ((10.0 + 50.0 + 10.0) / 3.0 * 0.75, (10.0 + 10.0 + 50.0) / 3.0 * 0.75);
    let centroid = pixel(&r.path, cx as usize, cy as usize).unwrap();
    assert!(close(centroid, [1.0 / 3.0; 3], 12.0), "{:?}", centroid);
    // close to the red vertex
    let near = pixel(&r.path, 8, 8).unwrap();
    assert!(near[0] > 225 && near[1] < 25 && near[2] < 25, "{:?}", near);

    // the same triangles without the extension have the flat color everywhere
    let flat = render("pixels_flat", &mesh("", &three_triangles()));
    let flat_near = pixel(&flat.path, 8, 8).unwrap();
    assert!(close(flat_near, [0x55 as f64 / 255.0; 3], 3.0), "{:?}", flat_near);
}

#[test]
fn equal_vertex_colors_match_a_flat_fill() {
    if !has_pdftoppm() {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    }
    let color = "0.2,0.4,0.8,1";
    let triangle = |fill: &str| tri([(10.0, 10.0), (90.0, 10.0), (10.0, 90.0)], [color; 3], fill);
    let shaded = render("equal", &mesh(GOURAUD, &triangle("#3366cc")));
    let flat = render("equal_flat", &mesh("", &triangle("#3366cc")));
    assert_eq!(shading_types(&shaded.pdf), [4]);
    for (x, y) in [(15, 15), (30, 20), (40, 40)] {
        let a = pixel(&shaded.path, x, y).unwrap();
        let b = pixel(&flat.path, x, y).unwrap();
        assert!(a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 1), "({x}, {y}): {a:?} {b:?}");
    }
}

#[test]
fn a_constant_alpha_is_the_fill_opacity_of_the_shading() {
    let half = "1,0,0,0.5";
    let r = render(
        "alpha",
        &mesh(GOURAUD, &tri([(10.0, 10.0), (90.0, 10.0), (10.0, 90.0)], [half, "0,1,0,0.5", "0,0,1,0.5"], "#555")),
    );
    assert_eq!(shading_types(&r.pdf), [4]);
    assert!(r.pdf.contains("/ca 0.5"), "{}", r.pdf);
    assert!(!r.log.contains("Gouraud") && !r.log.contains("data-umbruch-mesh"), "{}", r.log);
}

#[test]
fn an_alpha_that_changes_falls_back_to_flat_triangles_with_a_warning() {
    let r = render(
        "alpha_varies",
        &mesh(GOURAUD, &tri([(10.0, 10.0), (90.0, 10.0), (10.0, 90.0)], ["1,0,0,1", "0,1,0,0.5", "0,0,1,1"], "#555")),
    );
    assert!(shading_types(&r.pdf).is_empty(), "{}", r.pdf);
    assert_eq!(count(&r.pdf, "f"), 1, "{}", r.pdf);
    assert!(r.log.contains("alpha"), "{}", r.log);
}

#[test]
fn an_invalid_group_is_painted_like_any_group() {
    let good = tri([(10.0, 10.0), (50.0, 10.0), (10.0, 50.0)], [RED, GREEN, BLUE], "#555");
    let cases: [(&str, String); 7] = [
        ("empty", String::new()),
        ("not_a_triangle", "<path d=\"M10 10L50 10L50 50L10 50Z\" fill=\"#555\" data-colors=\"1,0,0;0,1,0;0,0,1\"/>".to_owned()),
        ("a_curve", "<path d=\"M10 10Q50 10 10 50Z\" fill=\"#555\" data-colors=\"1,0,0;0,1,0;0,0,1\"/>".to_owned()),
        ("no_colors", "<path d=\"M10 10L50 10L10 50Z\" fill=\"#555\"/>".to_owned()),
        ("short_colors", "<path d=\"M10 10L50 10L10 50Z\" fill=\"#555\" data-colors=\"1,0,0;0,1,0\"/>".to_owned()),
        ("bad_number", "<path d=\"M10 10L50 10L10 50Z\" fill=\"#555\" data-colors=\"1,0,x;0,1,0;0,0,1\"/>".to_owned()),
        ("a_rect", "<rect width=\"10\" height=\"10\" fill=\"#555\"/>".to_owned()),
    ];
    for (name, bad) in cases {
        let body = if name == "empty" { bad } else { format!("{good}{bad}") };
        let r = render(&format!("invalid_{name}"), &mesh(GOURAUD, &body));
        assert!(shading_types(&r.pdf).is_empty(), "{name}: {}", r.pdf);
        assert!(name == "empty" || count(&r.pdf, "f") >= 1, "{name}: {}", r.pdf);
    }
}

#[test]
fn non_finite_coordinates_do_not_fail_the_render() {
    let bad = tri([(f64::NAN, 10.0), (50.0, 10.0), (10.0, 50.0)], [RED, GREEN, BLUE], "#555");
    let r = render("nan", &mesh(GOURAUD, &bad));
    assert!(shading_types(&r.pdf).is_empty());
}

#[test]
fn the_transforms_of_the_elements_apply() {
    if !has_pdftoppm() {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    }
    // the same triangle, once in the group and once moved by the `transform` of the child
    let plain = tri([(10.0, 10.0), (50.0, 10.0), (10.0, 50.0)], [RED, GREEN, BLUE], "#555");
    let moved = plain.replace("<path ", "<path transform=\"translate(40 40)\" ");
    let a = render("transform_a", &format!("<svg {NS} width=\"100\" height=\"100\"><g transform=\"translate(40 40)\" {GOURAUD}>{plain}</g></svg>"));
    let b = render("transform_b", &mesh(GOURAUD, &moved));
    for (x, y) in [(40, 40), (45, 45), (60, 40)] {
        assert_eq!(pixel(&a.path, x, y), pixel(&b.path, x, y), "({x}, {y})");
    }
}

#[test]
fn a_big_mesh_is_split_into_several_shadings() {
    let mut body = String::new();
    // a grid of 120 x 120 cells of two triangles: 28800 triangles, below the limit of one shading
    // when it is one, and above it for a second group
    for i in 0..120 {
        for j in 0..120 {
            let (x, y) = (f64::from(i) * 0.8, f64::from(j) * 0.8);
            body += &tri([(x, y), (x + 0.8, y), (x, y + 0.8)], [RED, GREEN, BLUE], "#555");
            body += &tri([(x + 0.8, y), (x + 0.8, y + 0.8), (x, y + 0.8)], [GREEN, BLUE, RED], "#555");
        }
    }
    let r = render("big", &mesh(GOURAUD, &body));
    assert_eq!(shading_types(&r.pdf), [4], "{}", r.log);
    let mut more = body.clone();
    more += &body;
    let r = render("bigger", &mesh(GOURAUD, &more));
    assert_eq!(shading_types(&r.pdf), [4, 4], "{}", r.log);
    assert!(r.raw.len() > 0);
}

// -- the compact form: one element with the vertices and the triangles as indices --

/// The vertices (x y r g b a) and the triangles of the three triangles of `three_triangles`, as the
/// compact form writes them.
fn compact(vertices: &str, triangles: &str) -> String {
    mesh(&format!("{GOURAUD} data-vertices=\"{vertices}\" data-triangles=\"{triangles}\""), "")
}

const COMPACT_VERTICES: &str = "10 10 1 0 0 1, 50 10 0 1 0 1, 10 50 0 0 1 1, 60 10 0 1 0 1, 90 10 0 0 1 1, 60 40 1 0 0 1";

#[test]
fn the_compact_form_is_one_shading_and_looks_like_the_triangle_paths() {
    let r = render("compact", &compact(COMPACT_VERTICES, "0 1 2 3 4 5"));
    assert_eq!(shading_types(&r.pdf), [4], "{}", r.log);
    assert_eq!(count(&r.pdf, "sh"), 1);
    assert_eq!(count(&r.pdf, "f"), 0);
    if !has_pdftoppm() {
        return;
    }
    let paths = tri([(10.0, 10.0), (50.0, 10.0), (10.0, 50.0)], [RED, GREEN, BLUE], "#555")
        + &tri([(60.0, 10.0), (90.0, 10.0), (60.0, 40.0)], [GREEN, BLUE, RED], "#555");
    let reference = render("compact_reference", &mesh(GOURAUD, &paths));
    for (x, y) in [(8, 8), (15, 15), (20, 12), (50, 15), (52, 22)] {
        assert_eq!(pixel(&r.path, x, y), pixel(&reference.path, x, y), "({x}, {y})");
    }
}

#[test]
fn triangles_share_their_vertices() {
    // a square of two triangles that share the diagonal: four vertices, six indices
    let r = render(
        "compact_shared",
        &compact("10 10 1 0 0 1, 90 10 0 1 0 1, 90 90 0 0 1 1, 10 90 1 1 0 1", "0 1 2 0 2 3"),
    );
    assert_eq!(shading_types(&r.pdf), [4], "{}", r.log);
}

#[test]
fn a_compact_mesh_that_is_malformed_is_not_drawn_and_does_not_fail() {
    let cases = [
        ("index_out_of_range", COMPACT_VERTICES, "0 1 9"),
        ("not_a_multiple_of_3", COMPACT_VERTICES, "0 1"),
        ("negative_index", COMPACT_VERTICES, "0 1 -1"),
        ("vertex_numbers_missing", "10 10 1 0 0, 50 10 0 1 0 1, 10 50 0 0 1 1", "0 1 2"),
        ("not_a_number", "10 10 1 0 0 x, 50 10 0 1 0 1, 10 50 0 0 1 1", "0 1 2"),
        ("nan", "NaN 10 1 0 0 1, 50 10 0 1 0 1, 10 50 0 0 1 1", "0 1 2"),
        ("no_triangles", COMPACT_VERTICES, ""),
        ("degenerate", COMPACT_VERTICES, "0 0 1"),
    ];
    for (name, vertices, triangles) in cases {
        let r = render(&format!("compact_{name}"), &compact(vertices, triangles));
        assert!(shading_types(&r.pdf).is_empty(), "{name}: {}", r.pdf);
    }
    let r = render("compact_warns", &compact(COMPACT_VERTICES, "0 1 9"));
    assert!(r.log.contains("malformed"), "{}", r.log);
}

#[test]
fn a_compact_mesh_with_alpha_uses_the_constant_opacity() {
    let r = render("compact_alpha", &compact("10 10 1 0 0 0.5, 90 10 0 1 0 0.5, 10 90 0 0 1 0.5", "0 1 2"));
    assert_eq!(shading_types(&r.pdf), [4]);
    assert!(r.pdf.contains("/ca 0.5"), "{}", r.pdf);
    let r = render("compact_alpha_varies", &compact("10 10 1 0 0 1, 90 10 0 1 0 0.5, 10 90 0 0 1 1", "0 1 2"));
    // no flat triangles to fall back to: nothing is drawn, with a warning
    assert!(shading_types(&r.pdf).is_empty());
    assert!(r.log.contains("alpha"), "{}", r.log);
}

#[test]
fn a_translucent_gradient_mesh_is_lightened_in_poppler() {
    // poppler ignores the alpha constant of a gradient shading, so the mesh has to be painted as a
    // transparency group: near the red vertex, 50% red over white is pink, not red
    if !has_pdftoppm() {
        eprintln!("skipped: pdftoppm is not installed");
        return;
    }
    let svg = format!(
        "<svg {NS} width=\"100\" height=\"100\"><rect width=\"100\" height=\"100\" fill=\"#fff\"/>{}</svg>",
        format!(
            "<g {GOURAUD} data-vertices=\"10 10 1 0 0 0.5, 90 10 0 1 0 0.5, 10 90 0 0 1 0.5\" data-triangles=\"0 1 2\"/>"
        )
    );
    let r = render("translucent", &svg);
    assert_eq!(shading_types(&r.pdf), [4], "{}", r.log);
    let near = pixel(&r.path, 8, 8).unwrap();
    assert!(near[0] > 225 && near[1] > 100 && near[2] > 100, "{:?}", near);
}
