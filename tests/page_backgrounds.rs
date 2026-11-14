//! Backgrounds of pages: `@page { background }` and the canvas (the background of the root
//! element or of the body), which are painted over the whole page.

mod common;

fn render(name: &str, html: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("page_backgrounds__{}.html", name));
    let output = dir.join(format!("page_backgrounds__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .args(["--no-compression", "--no-system-fonts"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&std::fs::read(output).unwrap()).into_owned()
}

/// The colours of the rectangles that cover the 300pt x 200pt page, in stream order.
fn page_fills(pdf: &str) -> Vec<String> {
    let mut fills = vec![];
    for (end, _) in pdf.match_indices(" 0 0 300 200 re f") {
        // the colour operands in front of `rg`
        let head = &pdf[..end];
        let rg = head.rfind(" rg").unwrap();
        let operands: Vec<&str> = head[..rg].split_whitespace().rev().take(3).collect();
        fills.push(format!("{} {} {}", operands[2], operands[1], operands[0]));
    }
    fills
}

const PAGE: &str = "@page { size: 300pt 200pt; margin: 20pt; }";

#[test]
fn a_page_rule_paints_the_whole_page() {
    let pdf = render(
        "page_rule",
        &format!("<style>{PAGE} @page {{ background: #102030; }} p {{ margin: 0; }}</style><p>text</p>"),
    );
    assert_eq!(page_fills(&pdf), ["0.06 0.13 0.19"]);
}

#[test]
fn a_named_page_has_its_own_background() {
    let pdf = render(
        "named",
        &format!(
            "<style>{PAGE} @page {{ background: #102030; }} @page tint {{ background: #304050; }}
             .tint {{ page: tint; }} p {{ margin: 0; }}</style>
             <p>one</p><div class=\"tint\"><p>two</p></div>"
        ),
    );
    assert_eq!(page_fills(&pdf), ["0.06 0.13 0.19", "0.19 0.25 0.31"]);
}

#[test]
fn the_background_of_the_body_is_the_background_of_the_canvas() {
    let pdf = render(
        "body",
        &format!("<style>{PAGE} body {{ background: #ffeecc; margin: 0; }} p {{ margin: 0; }}</style><p>text</p><p style=\"break-before: page\">more</p>"),
    );
    // one rectangle for every page, and none in the box of the body
    assert_eq!(page_fills(&pdf), ["1 0.93 0.8", "1 0.93 0.8"]);
    assert_eq!(pdf.matches("1 0.93 0.8 rg").count(), 2, "the body is not painted again");
}

#[test]
fn the_root_element_wins_over_the_body_and_the_body_keeps_its_own_background() {
    let pdf = render(
        "html",
        &format!("<style>{PAGE} html {{ background: #102030; }} body {{ background: #ffeecc; }}</style><p>text</p>"),
    );
    assert_eq!(page_fills(&pdf), ["0.06 0.13 0.19"]);
    // the background of the body is painted in the box of the body
    assert_eq!(pdf.matches("1 0.93 0.8 rg").count(), 1);
}

#[test]
fn the_canvas_is_painted_over_the_background_of_the_page() {
    let pdf = render(
        "order",
        &format!("<style>{PAGE} @page {{ background: #102030; }} body {{ background: #ffeecc; }}</style><p>text</p>"),
    );
    assert_eq!(page_fills(&pdf), ["0.06 0.13 0.19", "1 0.93 0.8"]);
}

#[test]
fn a_gradient_covers_the_page() {
    let pdf = render(
        "gradient",
        &format!("<style>{PAGE} @page {{ background: linear-gradient(#f00, #00f); }}</style><p>text</p>"),
    );
    // painted with the `sh` operator, over the clip of the page
    assert!(pdf.contains(" sh "), "the gradient is painted as a shading");
    assert!(pdf.contains("0 200 m 300 200 l 300 0 l 0 0 l h W n"), "and clipped to the page");
}

#[test]
fn a_document_without_backgrounds_paints_nothing() {
    let pdf = render("none", &format!("<style>{PAGE}</style><p>text</p>"));
    assert!(page_fills(&pdf).is_empty());
}
