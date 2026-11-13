//! `letter-spacing` and `text-transform`.

mod common;

fn log_of(name: &str, html: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("text_properties__{}.html", name));
    std::fs::write(&input, html).unwrap();
    let out = common::helium()
        .unwrap()
        .arg("--no-system-fonts")
        .arg(&input)
        .arg(dir.join(format!("text_properties__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(out.status.success());
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_properties_are_accepted() {
    let log = log_of(
        "accepted",
        "<style>p{letter-spacing:0.1em;text-transform:capitalize}b{letter-spacing:normal;text-transform:none}\
         i{letter-spacing:-1px;text-transform:uppercase}u{text-transform:lowercase}</style>\
         <p>one <b>two</b> <i>three</i> <u>FOUR</u></p>",
    );
    assert!(!log.contains("Error while parsing"), "{}", log);
}
