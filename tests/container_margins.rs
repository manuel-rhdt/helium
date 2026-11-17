//! The margins between the containers with columns of a page are counted by the paginator: the
//! layout of the document has the box that follows a container below its margin, and the paginator
//! counts what the layout puts on a page (a mismatch is logged as a warning, and content can
//! then go below the page).

mod common;

#[test]
fn the_margin_between_two_containers_with_columns_is_counted() {
    let paragraph = |word: &str| format!("<p>{}</p>", format!("{word} ").repeat(60));
    let html = format!(
        "<!DOCTYPE html><html lang=en><title>t</title><style>@page {{ size: 600px 400px; margin: 20px }}
         body {{ margin: 0; font: 14px/20px serif }} p {{ margin: 0 }}
         .cols {{ column-count: 2; column-gap: 20px; margin: 0 0 20px }}</style>
         <div class=cols>{}</div><div class=cols>{}</div><div class=cols>{}</div>",
        paragraph("aaa"),
        paragraph("bbb"),
        paragraph("ccc")
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("container_margins.html");
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .env("RUST_LOG", "warn")
        .args(["--no-system-fonts"])
        .arg(&input)
        .arg(dir.join("container_margins.pdf"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("paginator counted"), "{stderr}");
}
