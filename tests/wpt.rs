//! Runs the vendored web-platform-tests layout tests (`checkLayout`) through
//! `helium --check-layout`.

mod common;

use assert_cmd::prelude::*;
use common::*;
use std::process::Command;

#[test]
fn wpt_layout_checks() {
    let known_failures = read_expectations("expectations.txt");

    let mut tests = Vec::new();
    collect_tests(&wpt_root(), &mut tests);
    // reftests and helium's own files live next to the checkLayout tests
    tests.retain(|t| std::fs::read_to_string(t).map_or(false, |s| s.contains("data-expected-")));
    tests.sort();
    assert!(!tests.is_empty(), "no WPT layout tests found in {}", wpt_root().display());

    let mut problems = stale_expectations(&known_failures);
    for test in &tests {
        let name = test_name(test);
        let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
            .unwrap()
            .arg("--check-layout")
            .arg(test)
            .arg(pdf_path(test))
            .output()
            .unwrap();
        match (output.status.success(), known_failures.contains(&name)) {
            (true, false) | (false, true) => {}
            (false, false) => problems.push(format!(
                "FAIL {}\n{}",
                name,
                String::from_utf8_lossy(&output.stderr)
            )),
            (true, true) => problems.push(format!(
                "UNEXPECTED PASS {} (remove it from tests/wpt/expectations.txt)",
                name
            )),
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
