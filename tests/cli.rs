mod common;

use assert_cmd::prelude::*;
use predicates::prelude::*;

#[test]
fn file_doesnt_exist() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = common::helium()?;
    cmd.args(&["test/file/doesnt/exist", "never.pdf"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn converts_loremipsum_to_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/documents/loremipsum.html"
    );
    let output = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("loremipsum.pdf");
    // make sure a stale file from an earlier run cannot make the test pass
    if output.exists() {
        std::fs::remove_file(&output)?;
    }

    let mut cmd = common::helium()?;
    cmd.arg(input).arg(&output);
    cmd.assert().success();

    let pdf = std::fs::read(&output)?;
    assert!(
        pdf.starts_with(b"%PDF-"),
        "output does not start with a PDF header"
    );
    assert!(
        pdf.windows(5).rev().take(1024).any(|w| w == b"%%EOF"),
        "output has no %%EOF trailer"
    );

    Ok(())
}

/// Reads the page count from the PDF's page tree and checks it against the number of
/// page objects.
fn pdf_page_count(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    let count_pos = text.find("/Count ").expect("no /Count in page tree") + "/Count ".len();
    let declared: usize = text[count_pos..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .expect("/Count is not a number");
    let page_objects = text.matches("/Type /Page\n").count() + text.matches("/Type /Page ").count();
    assert_eq!(
        declared, page_objects,
        "/Count does not match number of page objects"
    );
    declared
}

fn convert_to_pdf(name: &str, html: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    let output = dir.join(format!("{}.pdf", name));
    std::fs::write(&input, html)?;
    if output.exists() {
        std::fs::remove_file(&output)?;
    }

    common::helium()?
        .arg(&input)
        .arg(&output)
        .assert()
        .success();

    Ok(common::expand_object_streams(&std::fs::read(&output)?))
}

#[test]
fn long_document_is_paginated() -> Result<(), Box<dyn std::error::Error>> {
    let paragraph = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                     tempor incididunt ut labore et dolore magna aliqua. "
        .repeat(8);
    let document = |paragraphs: usize| {
        let body: String = (0..paragraphs)
            .map(|_| format!("<p>{}</p>", paragraph))
            .collect();
        format!(
            "<!DOCTYPE html><html><body><h1>Long</h1>{}</body></html>",
            body
        )
    };

    let short = convert_to_pdf("pagination_short", &document(1))?;
    let long = convert_to_pdf("pagination_long", &document(5))?;

    assert!(short.starts_with(b"%PDF-") && long.starts_with(b"%PDF-"));
    assert_eq!(
        pdf_page_count(&short),
        1,
        "a single paragraph should fit on one page"
    );
    assert!(
        pdf_page_count(&long) >= 2,
        "five paragraphs should not fit on a single page"
    );

    Ok(())
}

#[test]
fn dumps_layout_as_json() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("dump_layout.html");
    std::fs::write(
        &input,
        r#"<!DOCTYPE html><div id="a" style="width: 100px; height: 20px; padding: 5px; margin: 3px"></div>"#,
    )?;

    let mut cmd = common::helium()?;
    cmd.arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("dump_layout.pdf"));
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""id": "a""#))
        // border box includes the padding, the margin is reported separately
        // (the Taffy layout reports the specified margins, not the used `margin-right` of an
        // over-constrained block)
        .stdout(predicate::str::contains(r#""width": 110, "height": 30}"#))
        .stdout(predicate::str::contains(
            r#""margin": {"top": 3, "right": 3, "bottom": 3, "left": 3}"#,
        ));

    Ok(())
}

#[test]
fn loads_relative_external_stylesheets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let assets = dir.join("external_stylesheet_assets");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(
        assets.join("print.css"),
        "#styled { width: 123px; height: 20px; }",
    )?;

    let input = dir.join("external_stylesheet.html");
    std::fs::write(
        &input,
        "<html><head><link rel=\"stylesheet\" href=\"external_stylesheet_assets/print.css\"></head><body><div id=\"styled\"></div></body></html>",
    )?;

    let mut cmd = common::helium()?;
    cmd.arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("external_stylesheet.pdf"));
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"width\": 123, \"height\": 20"));

    Ok(())
}

#[test]
fn loads_full_quarto_html_from_document() -> Result<(), Box<dyn std::error::Error>> {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/documents/quarto_stylesheet.html"
    );
    let output = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("quarto_stylesheet.pdf");
    let html = std::fs::read_to_string(input)?;
    assert!(
        html.contains("<math "),
        "Quarto output should contain MathML"
    );
    assert!(html.contains("<pre class=\"sourceCode python"));

    let mut cmd = common::helium()?;
    cmd.arg("--dump-layout").arg("-").arg(input).arg(&output);
    let result = cmd.output()?;
    assert!(
        result.status.success(),
        "Helium failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let layout: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    let layout_dump = String::from_utf8_lossy(&result.stdout);
    assert!(
        layout_dump.contains("\"margin\": {\"top\": 0, \"right\": 0, \"bottom\": 12, \"left\": 0}")
    );
    assert!(
        layout_dump.contains("This document was prepared for Regional Transit Reliability Review.")
    );
    assert!(layout_dump.contains("scheduled_trips"));
    assert!(layout_dump.contains("\"value\": [0, 92, 197, 255]"));
    assert!(layout_dump.contains("\"background\": [240, 243, 246, 255]"));

    fn find_node_by_id<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
        if node.get("id").and_then(serde_json::Value::as_str) == Some(id) {
            return Some(node);
        }
        node.get("children")?
            .as_array()?
            .iter()
            .find_map(|child| find_node_by_id(child, id))
    }

    let line_y = ["cb1-1", "cb1-2", "cb1-3"].map(|id| {
        find_node_by_id(&layout["document"], id)
            .and_then(|node| node.get("border_box"))
            .and_then(|box_| box_.get("y"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_else(|| panic!("missing layout position for code line {id}"))
    });
    assert!(
        line_y[0] < line_y[1] && line_y[1] < line_y[2],
        "preformatted code lines should have distinct baselines: {line_y:?}"
    );

    let pdf = std::fs::read(output)?;
    let pdf = common::expand_object_streams(&pdf);
    assert!(
        pdf_page_count(&pdf) >= 2,
        "the Quarto report should span multiple pages"
    );

    Ok(())
}

/// A higher-specificity rule must win over a later, lower-specificity rule for the same
/// property, regardless of source order. This used to be decided purely by source order.
#[test]
fn higher_specificity_beats_later_rule() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("specificity.html");
    std::fs::write(
        &input,
        "<html><head><style>\
         #header p { text-align: center } \
         p { text-align: justify } \
         </style></head>\
         <body><div id=\"header\"><p>Centered</p></div><p>Justified</p></body></html>",
    )
    .unwrap();
    let mut cmd = common::helium()?;
    cmd.arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("specificity.pdf"));
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let layout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Text runs appear in DOM order, so the first run is the centered paragraph and the
    // second is the justified one. Centered text starts right of the left margin,
    // justified text starts at it.
    let runs = layout["paint"]["text"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{:?}", runs);
    let centered = runs[0]["x"].as_f64().unwrap();
    let left_margin = runs[1]["x"].as_f64().unwrap();
    assert!(
        centered > left_margin,
        "the higher-specificity rule should center its text: centered at {centered}, \
         left margin at {left_margin}"
    );
    Ok(())
}

/// Selectors that look at sibling elements (`:first-of-type`, `+`, `~`) used to loop forever
/// because the sibling iterators included the element itself.
#[test]
fn sibling_selectors_terminate() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("sibling_selectors.html");
    std::fs::write(
        &input,
        "<html><head><style>h1:first-of-type { color: red } h1 + p { color: blue } h1 ~ h1 { color: green }</style></head>\
         <body><h1>One</h1><p>Hi</p><h1>Two</h1></body></html>",
    )
    .unwrap();
    common::helium()
        .unwrap()
        .arg(&input)
        .arg(dir.join("sibling_selectors.pdf"))
        .assert()
        .success();
}

/// The hyperlinks in a document are written as link annotations and attached to the pages they
/// are on. A link that is split over multiple lines produces one annotation per line.
#[test]
fn hyperlinks_are_written_to_the_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "hyperlinks",
        r#"<!DOCTYPE html>
<html><body>
<p>One <a href="https://example.com">link</a> here.</p>
<p>Another <a href="https://example.org/a/very/long/path">long link that will wrap over the end of this line because it is long enough to do so</a> over.</p>
</body></html>"#,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    let utf8_owned = text.into_owned();

    let link_annotations: Vec<&str> = utf8_owned.split("/Subtype /Link").skip(1).collect();
    assert_eq!(
        link_annotations.len(),
        3,
        "expected 3 link annotations (one is split over two lines)"
    );

    assert!(
        utf8_owned.contains("/URI (https://example.com)"),
        "missing example.com URI"
    );
    // the wrapped link appears once per line it is on
    assert_eq!(
        utf8_owned
            .matches("/URI (https://example.org/a/very/long/path)")
            .count(),
        2
    );
    // every page that has annotations must reference them
    assert!(
        utf8_owned.contains("/Annots ["),
        "page does not reference its annotations"
    );

    Ok(())
}

/// Cross-reference links (`<a href="#...">`) become `GoTo` link annotations that jump to the
/// element with the matching `id`, on whatever page it ends up on. Links to unknown fragments
/// stay URI links.
#[test]
fn cross_reference_links_jump_within_the_document() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "cross_references",
        r##"<!DOCTYPE html>
<html><body>
<p><a href="#target">jump down</a></p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p>filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler</p>
<p id="target">the target</p>
</body></html>"##,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    let utf8_owned = text.into_owned();

    assert_eq!(
        utf8_owned.matches("/Subtype /Link").count(),
        1,
        "expected one link"
    );
    // the link must be a GoTo action (not a URI) with a page-0 destination.
    // The dictionary serialization splits key-value pairs across lines, so
    // /S /GoTo and /D [...] are separate assertions.
    assert!(
        utf8_owned.contains("/S /GoTo"),
        "cross-reference is not a GoTo action"
    );
    assert!(
        utf8_owned.contains("/D [ 0 /XYZ"),
        "GoTo destination does not reference page 0"
    );
    // and it must not be a URI link
    assert!(
        !utf8_owned.contains("/S /URI"),
        "cross-reference should not be a URI link"
    );

    Ok(())
}

/// A link to an unknown fragment stays a URI link instead of being dropped.
#[test]
fn link_to_unknown_fragment_stays_a_uri_link() -> Result<(), Box<dyn std::error::Error>> {
    let pdf = convert_to_pdf(
        "unknown_fragment",
        r##"<!DOCTYPE html><html><body><p><a href="#missing">nothing</a></p></body></html>"##,
    )?;
    let text = String::from_utf8_lossy(&pdf);
    assert!(
        text.contains("/S /URI"),
        "expected a URI link for an unknown fragment"
    );
    assert!(
        !text.contains("/S /GoTo"),
        "unknown fragments must not become GoTo links"
    );
    Ok(())
}
