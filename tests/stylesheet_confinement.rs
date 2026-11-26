//! `<link rel=stylesheet>` reads its file like every other reference of a document: only inside
//! of the directory of the document, and with an error that tells nothing about the machine.
//! (It used to read any file, absolute paths included, and to print the text of one that is not
//! CSS in a warning.)

use std::path::PathBuf;
use std::process::Command;

struct Run {
    log: String,
    pdf: String,
}

/// Renders `html` in `<tmp>/<name>/root/`, next to which is `outside.css`.
fn render(name: &str, html: &str) -> Run {
    let top = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("stylesheet_{name}"));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(top.join("root")).unwrap();
    std::fs::write(top.join("outside.css"), "p { color: #ff0000 }\n").unwrap();
    std::fs::write(top.join("outside.txt"), "TOP-SECRET-TOKEN-12345\nsecond line of the secret\n").unwrap();
    std::fs::write(top.join("root/inside.css"), "p { color: #0000ff }\n").unwrap();
    std::fs::write(top.join("root/doc.html"), format!("<!DOCTYPE html><title>t</title>{html}<p>hello</p>")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_umbruch"))
        .args(["--no-compression", "doc.html", "doc.pdf"])
        .current_dir(top.join("root"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    Run {
        log: String::from_utf8_lossy(&output.stderr).into_owned(),
        pdf: String::from_utf8_lossy(&std::fs::read(top.join("root/doc.pdf")).unwrap()).into_owned(),
    }
}

const RED: &str = "1 0 0 rg";
const BLUE: &str = "0 0 1 rg";

#[test]
fn a_style_sheet_inside_of_the_directory_is_used() {
    let r = render("inside", r#"<link rel=stylesheet href="inside.css">"#);
    assert!(r.pdf.contains(BLUE), "{}", r.log);
}

#[test]
fn a_style_sheet_outside_of_the_directory_is_not_read() {
    let top = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stylesheet_outside/outside.css");
    let absolute = top.to_string_lossy().replace('\\', "/");
    for (name, href) in [("relative", "../outside.css".to_owned()), ("absolute", absolute)] {
        let r = render(&format!("outside_{name}"), &format!(r#"<link rel=stylesheet href="{href}">"#));
        assert!(!r.pdf.contains(RED), "{name}: the style sheet was used");
        assert!(r.log.contains("Could not read stylesheet"), "{name}: {}", r.log);
        assert!(r.log.contains("outside of the directory"), "{name}: {}", r.log);
    }
}

#[test]
fn what_is_in_a_file_that_is_not_css_is_not_printed() {
    let r = render("secret", r#"<link rel=stylesheet href="../outside.txt"><link rel=stylesheet href="/etc/passwd">"#);
    assert!(!r.log.contains("TOP-SECRET"), "{}", r.log);
    assert!(!r.log.contains("second line"), "{}", r.log);
    assert!(!r.log.contains("root:"), "{}", r.log);
}

#[test]
fn a_style_sheet_that_does_not_exist_says_the_same_as_one_that_is_outside() {
    let exists = render("exists", r#"<link rel=stylesheet href="../outside.css">"#).log;
    let missing = render("missing", r#"<link rel=stylesheet href="../nothing.css">"#).log;
    let message = |log: &str| {
        log.lines()
            .find(|line| line.contains("Could not read stylesheet"))
            .map(|line| line.split("Could not read stylesheet").nth(1).unwrap().replace("outside.css", "X").replace("nothing.css", "X"))
            .unwrap_or_default()
    };
    assert!(!message(&exists).is_empty(), "{exists}");
    assert_eq!(message(&exists), message(&missing));
}

#[cfg(unix)]
#[test]
fn a_link_to_a_style_sheet_outside_is_not_followed() {
    let top = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("stylesheet_symlink");
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(top.join("root")).unwrap();
    std::fs::write(top.join("outside.css"), "p { color: #ff0000 }\n").unwrap();
    std::os::unix::fs::symlink("../outside.css", top.join("root/link.css")).unwrap();
    std::fs::write(top.join("root/doc.html"), r#"<!DOCTYPE html><title>t</title><link rel=stylesheet href="link.css"><p>hello</p>"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_umbruch"))
        .args(["--no-compression", "doc.html", "doc.pdf"])
        .current_dir(top.join("root"))
        .env("RUST_LOG", "warn")
        .output()
        .unwrap();
    assert!(output.status.success());
    let pdf = String::from_utf8_lossy(&std::fs::read(top.join("root/doc.pdf")).unwrap()).into_owned();
    assert!(!pdf.contains(RED));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside of the directory"));
}
