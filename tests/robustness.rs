//! Documents that made Umbruch panic: they must be rendered (or at least rejected with an
//! error), never abort.

mod common;

use std::path::Path;

/// Renders the document and returns the text of the glyph runs that it paints.
fn render(name: &str, html: &str) -> serde_json::Value {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("robustness__{}.html", name));
    let output = dir.join(format!("robustness__{}.pdf", name));
    std::fs::write(&input, html).unwrap();
    let out = common::umbruch()
        .unwrap()
        .args(["--no-system-fonts", "--dump-layout", "-"])
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{}", stderr);
    assert!(out.status.success(), "{}", stderr);
    serde_json::from_slice(&out.stdout).unwrap()
}

fn painted_runs(dump: &serde_json::Value) -> usize {
    dump["paint"]["text"].as_array().map_or(0, Vec::len)
}

#[test]
fn a_root_element_without_a_box_is_a_blank_page() {
    let dump = render(
        "root_display_none",
        "<style>html { display: none; background: green }</style><body><p>text</p></body>",
    );
    assert_eq!(painted_runs(&dump), 0);
    assert!(dump["paint"]["rects"].as_array().unwrap().is_empty());
}

#[test]
fn a_comment_next_to_the_head_is_not_a_box() {
    let dump = render(
        "comment_in_html",
        "<html><!-- a comment --><head><style>p { margin: 0 }</style></head><body><p>text</p></body></html>",
    );
    assert_eq!(painted_runs(&dump), 1);
}

#[test]
fn a_megabyte_of_comments_does_not_overflow_the_stack() {
    // dropping the DOM used to recurse once per sibling
    let html = format!("<p>text</p>{}", "<!---->".repeat(150_000));
    let dump = render("many_comments", &html);
    assert_eq!(painted_runs(&dump), 1);
    // and one comment of a megabyte
    let html = format!("<p>text</p><!--{}--><p>more</p>", "a".repeat(1 << 20));
    let dump = render("big_comment", &html);
    assert_eq!(painted_runs(&dump), 2);
}

#[test]
fn a_comment_between_blocks_is_not_a_box() {
    let dump = render(
        "comment_between_blocks",
        "<div>one</div><!-- a comment --><div>two</div><?php echo 1 ?>",
    );
    assert_eq!(painted_runs(&dump), 2);
}

#[test]
fn the_content_of_a_template_is_not_rendered() {
    let dump = render(
        "template",
        "<div>shown</div><template><p>hidden</p></template>\
         <div><template shadowrootmode=open><slot></slot></template>also shown</div>",
    );
    assert_eq!(painted_runs(&dump), 2, "{}", dump["paint"]["text"]);
}
