//! A document that nests elements deeper than the limit is flattened there instead of overflowing
//! the stack.

mod common;

#[test]
fn elements_nested_deeper_than_the_limit_are_flattened() {
    let depth = 800;
    let html = format!("{}deep text{}", "<div>".repeat(depth), "</div>".repeat(depth));
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("nesting.html");
    std::fs::write(&input, html).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("nesting.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("deep text"));
}
