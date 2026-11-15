//! Runs the vendored web-platform-tests layout tests (`checkLayout`) through
//! `helium --check-layout`.

mod common;

use common::*;

#[test]
fn wpt_layout_checks() {
    let known_failures = read_expectations("expectations.txt");

    let mut tests = Vec::new();
    collect_tests(&wpt_root(), &mut tests);
    // reftests and helium's own files live next to the checkLayout tests
    tests.retain(|t| std::fs::read_to_string(t).map_or(false, |s| s.contains("data-expected-")));
    tests.sort();
    // the tests that need what helium does not implement on purpose are not run
    let before = tests.len();
    tests.retain(|test| unsupported_feature(&[test]).is_none());
    eprintln!("{} layout tests need unsupported features (tests/wpt/unsupported.txt) and are skipped", before - tests.len());
    assert!(
        !tests.is_empty(),
        "no WPT layout tests found in {}",
        wpt_root().display()
    );

    let results = par_map(&tests, |test| {
        let mut command = common::helium().unwrap();
        command
            .arg("--font-dir")
            .arg(common::wpt_fonts())
            .arg("--check-layout")
            .arg(test)
            .arg(pdf_path(test));
        match output_within(command, 30) {
            Some(output) if output.status.success() => Ok(()),
            Some(output) => Err(String::from_utf8_lossy(&output.stderr).into_owned()),
            None => Err("helium took too long".to_owned()),
        }
    });
    if update_expectations() {
        let failing = tests
            .iter()
            .zip(&results)
            .filter(|(_, result)| result.is_err())
            .map(|(test, _)| test_name(test))
            .collect();
        write_expectations("expectations.txt", &failing);
        return;
    }
    let mut problems = stale_expectations(&known_failures);
    for (test, result) in tests.iter().zip(results) {
        let name = test_name(test);
        match (result, known_failures.contains(&name)) {
            (Ok(()), false) | (Err(_), true) => {}
            (Err(why), false) => problems.push(format!("FAIL {}\n{}", name, why)),
            (Ok(()), true) => problems.push(format!(
                "UNEXPECTED PASS {} (remove it from tests/wpt/expectations.txt)",
                name
            )),
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
