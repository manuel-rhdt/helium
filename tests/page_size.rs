//! `@page { size }`, checked in the media box of the PDF (in points).

mod common;

/// The width and height of the first page, in points, rounded to tenths.
fn page_size(name: &str, css: &str) -> (f64, f64) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("page_size_{}.html", name));
    let output = dir.join(format!("page_size_{}.pdf", name));
    std::fs::write(&input, format!("<style>{}</style><p>text</p>", css)).unwrap();
    let result = common::helium()
        .unwrap()
        .arg("--no-compression")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let pdf = String::from_utf8_lossy(&common::expand_object_streams(&std::fs::read(&output).unwrap()))
        .into_owned();
    let start = pdf.find("/MediaBox").expect("a media box") + "/MediaBox".len();
    let numbers: Vec<f64> = pdf[start..]
        .split(']')
        .next()
        .unwrap()
        .trim_start_matches(|c: char| c == ' ' || c == '[')
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let round = |x: f64| (x * 10.0).round() / 10.0;
    (round(numbers[2] - numbers[0]), round(numbers[3] - numbers[1]))
}

const A4: (f64, f64) = (595.3, 841.9);

#[test]
fn the_default_is_a4() {
    assert_eq!(page_size("default", ""), A4);
    assert_eq!(page_size("auto", "@page { size: auto }"), A4);
}

#[test]
fn one_or_two_lengths() {
    assert_eq!(page_size("two", "@page { size: 300pt 200pt }"), (300.0, 200.0));
    assert_eq!(page_size("one", "@page { size: 250pt }"), (250.0, 250.0));
    assert_eq!(page_size("mm", "@page { size: 100mm 50mm }"), (283.5, 141.7));
    assert_eq!(page_size("px", "@page { size: 400px 200px }"), (300.0, 150.0));
}

#[test]
fn named_sizes_with_and_without_orientation() {
    assert_eq!(page_size("a4", "@page { size: A4 }"), A4);
    assert_eq!(page_size("a5", "@page { size: a5 }"), (419.5, 595.3));
    assert_eq!(page_size("letter", "@page { size: letter }"), (612.0, 792.0));
    assert_eq!(page_size("legal", "@page { size: legal }"), (612.0, 1008.0));
    assert_eq!(page_size("landscape", "@page { size: letter landscape }"), (792.0, 612.0));
    assert_eq!(page_size("order", "@page { size: landscape letter }"), (792.0, 612.0));
    assert_eq!(page_size("portrait", "@page { size: A3 portrait }"), (841.9, 1190.6));
}

#[test]
fn an_orientation_turns_the_default_size() {
    assert_eq!(page_size("only_landscape", "@page { size: landscape }"), (A4.1, A4.0));
    assert_eq!(page_size("only_portrait", "@page { size: portrait }"), A4);
}

#[test]
fn invalid_values_are_ignored() {
    for (name, value) in [
        ("word", "huge"),
        ("two_names", "A4 letter"),
        ("two_orientations", "landscape portrait"),
        ("three", "100pt 100pt 100pt"),
        ("percentage", "50%"),
    ] {
        assert_eq!(
            page_size(name, &format!("@page {{ size: {} }}", value)),
            A4,
            "size: {}",
            value
        );
    }
}

#[test]
fn size_applies_after_the_default_and_to_pages_selected_by_the_rule() {
    assert_eq!(
        page_size("later", "@page { width: 300pt; height: 300pt } @page { size: 200pt 100pt }"),
        (200.0, 100.0)
    );
    assert_eq!(page_size("only_width", "@page { width: 300pt }"), (300.0, A4.1));
}
