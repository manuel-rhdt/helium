//! Runs vendored web-platform-tests reftests: a test and its `<link rel="match">` reference must
//! produce the same painted output.
//!
//! Helium has no rasterizer, so instead of comparing pixels we compare the `paint` section of
//! `helium --dump-layout`: the solid rectangles (backgrounds and borders) and the glyph runs with
//! their positions. This is independent of how the two documents are structured, which is what
//! reftests rely on. Following the WPT convention that "red means failure", a test must also not
//! paint anything pure red.

//! Runs vendored web-platform-tests reftests: a test and its `<link rel="match">` reference must
//! look the same.
//!
//! Both documents are turned into PDFs, which `pdftoppm` (poppler-utils) rasterizes at 96 dpi, so
//! that a pixel is a CSS pixel; the pages must be the same size and no pixel may differ by more
//! than a little. Following the WPT convention that "red means failure", a test must also not
//! paint pure red.

mod common;

use common::*;
use std::path::{Path, PathBuf};

/// The most that a channel of a pixel may differ for the pixels to be the same.
const FUZZ: u8 = 24;

struct Page {
    width: usize,
    height: usize,
    /// RGB, row by row.
    pixels: Vec<u8>,
}

/// Reads a binary PPM file (what `pdftoppm` writes without an option for another format).
fn read_ppm(data: &[u8]) -> Result<Page, String> {
    let mut at = 0;
    let mut token = || -> Result<String, String> {
        while at < data.len() && (data[at].is_ascii_whitespace() || data[at] == b'#') {
            if data[at] == b'#' {
                while at < data.len() && data[at] != b'\n' {
                    at += 1;
                }
            } else {
                at += 1;
            }
        }
        let start = at;
        while at < data.len() && !data[at].is_ascii_whitespace() {
            at += 1;
        }
        String::from_utf8(data[start..at].to_vec()).map_err(|e| e.to_string())
    };
    if token()? != "P6" {
        return Err("not a PPM file".into());
    }
    let width: usize = token()?.parse().map_err(|_| "bad width")?;
    let height: usize = token()?.parse().map_err(|_| "bad height")?;
    token()?;
    // one white space separates the header from the pixels
    let pixels = data[at + 1..].to_vec();
    if pixels.len() != width * height * 3 {
        return Err("the PPM file is cut off".into());
    }
    Ok(Page { width, height, pixels })
}

/// Renders the document to a PDF and rasterizes its pages.
fn render(input: &Path) -> Result<Vec<Page>, String> {
    // every rendering has files of its own: the tests run on all cores
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wpt-raster");
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join(format!("{}.pdf", COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let mut command = common::helium().unwrap();
    // (`--font-dir=` because the option takes several values, which would be the files)
    command
        .arg(format!("--font-dir={}", common::wpt_fonts().display()))
        .arg(format!("--resource-root={}", wpt_root().display()))
        .arg(input)
        .arg(&pdf);
    let output = output_within(command, 30).ok_or("helium took too long")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("helium failed: {}", stderr.lines().last().unwrap_or("")));
    }
    let prefix = pdf.with_extension("raster");
    let mut command = std::process::Command::new("pdftoppm");
    command.args(["-r", "96"]).arg(&pdf).arg(&prefix);
    let output = output_within(command, 60)
        .ok_or("pdftoppm took too long (is it installed? it is in poppler-utils)")?;
    if !output.status.success() {
        return Err(format!("pdftoppm failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    // `prefix-1.ppm`, `prefix-2.ppm`, ... (the number has as many digits as the page count needs)
    let name = prefix.file_name().unwrap().to_string_lossy().into_owned();
    let mut files: Vec<PathBuf> = std::fs::read_dir(prefix.parent().unwrap())
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name().is_some_and(|f| {
                let f = f.to_string_lossy();
                f.starts_with(&format!("{name}-")) && f.ends_with(".ppm")
            })
        })
        .collect();
    files.sort();
    let pages = files
        .iter()
        .map(|file| {
            let page = read_ppm(&std::fs::read(file).unwrap());
            std::fs::remove_file(file).ok();
            page
        })
        .collect::<Result<Vec<_>, _>>();
    std::fs::remove_file(&pdf).ok();
    let pages = pages?;
    if pages.is_empty() {
        return Err("no page was rendered".into());
    }
    Ok(pages)
}

/// Finds the reference of a test from its `<link rel="match" href="...">`.
fn find_reference(test: &Path) -> Option<PathBuf> {
    let html = std::fs::read_to_string(test).ok()?;
    let mut rest = html.as_str();
    while let Some(start) = rest.find("<link") {
        let tag_end = rest[start..].find('>')? + start;
        let tag = &rest[start..tag_end];
        rest = &rest[tag_end..];
        if tag.contains("rel=\"match\"") {
            let href = tag.split("href=\"").nth(1)?.split('"').next()?;
            return Some(test.parent()?.join(href));
        }
    }
    None
}

fn paints_red(pages: &[Page]) -> bool {
    pages
        .iter()
        .any(|page| page.pixels.chunks(3).any(|p| p[0] >= 250 && p[1] <= 5 && p[2] <= 5))
}

fn is_blank(pages: &[Page]) -> bool {
    pages.iter().all(|page| page.pixels.iter().all(|&channel| channel >= 250))
}

/// Whether the pixel at `index` of `a` is like one of the pixels around it in `b`. The edges of
/// an image are drawn to whole pixels and the edges of a filled rectangle are antialiased, so the
/// same box differs by a pixel at its fractional position (the margins of the page are 2.5 cm).
fn matches_nearby(a: &Page, b: &Page, index: usize) -> bool {
    let (x, y) = ((index % a.width) as isize, (index / a.width) as isize);
    let pixel = &a.pixels[index * 3..index * 3 + 3];
    for dy in -1..=1isize {
        for dx in -1..=1isize {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= b.width as isize || ny >= b.height as isize {
                continue;
            }
            let at = (ny as usize * b.width + nx as usize) * 3;
            if pixel.iter().zip(&b.pixels[at..at + 3]).all(|(p, q)| p.abs_diff(*q) <= FUZZ) {
                return true;
            }
        }
    }
    false
}

fn run_reftest(test: &Path, expected: &Result<Vec<Page>, String>) -> Result<(), String> {
    let actual = render(test)?;
    let expected = expected.as_ref().map_err(|why| format!("the reference: {}", why))?;

    if paints_red(&actual) {
        return Err("paints red".into());
    }
    if is_blank(&actual) && is_blank(expected) {
        return Err("nothing was painted, the comparison is vacuous".into());
    }
    if actual.len() != expected.len() {
        return Err(format!("{} pages, the reference has {}", actual.len(), expected.len()));
    }
    for (number, (a, b)) in actual.iter().zip(expected).enumerate() {
        if (a.width, a.height) != (b.width, b.height) {
            return Err(format!(
                "page {} is {}x{} pixels, the reference's {}x{}",
                number + 1,
                a.width,
                a.height,
                b.width,
                b.height
            ));
        }
        let mut differing = 0;
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
        for (index, (p, q)) in a.pixels.chunks(3).zip(b.pixels.chunks(3)).enumerate() {
            if p.iter().zip(q).any(|(x, y)| x.abs_diff(*y) > FUZZ)
                && !matches_nearby(a, b, index)
                && !matches_nearby(b, a, index)
            {
                differing += 1;
                let (x, y) = (index % a.width, index / a.width);
                (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x), bottom.max(y));
            }
        }
        if differing > 0 {
            return Err(format!(
                "page {} differs in {} pixels, within x {}..{} and y {}..{}",
                number + 1,
                differing,
                left,
                right,
                top,
                bottom
            ));
        }
    }
    Ok(())
}

#[test]
fn wpt_reftests() {
    let known_failures = read_expectations("reftest-expectations.txt");

    let mut tests = Vec::new();
    collect_tests(&wpt_root(), &mut tests);
    tests.retain(|t| {
        let is_reference = t
            .file_stem()
            .and_then(|s| s.to_str())
            .map_or(false, |s| s.ends_with("-ref"));
        !is_reference && find_reference(t).is_some()
    });
    tests.sort();
    // the tests that need what helium does not implement on purpose are not run
    let before = tests.len();
    tests.retain(|test| {
        let reference = find_reference(test).unwrap();
        unsupported_feature(&[test, &reference]).is_none()
    });
    eprintln!("{} reftests need unsupported features (tests/wpt/unsupported.txt) and are skipped", before - tests.len());
    assert!(
        !tests.is_empty(),
        "no WPT reftests found in {}",
        wpt_root().display()
    );

    // the tests that share a reference are run together, with the reference rendered once
    let mut groups: std::collections::BTreeMap<PathBuf, Vec<usize>> = Default::default();
    for (index, test) in tests.iter().enumerate() {
        groups.entry(find_reference(test).unwrap()).or_default().push(index);
    }
    let groups: Vec<(PathBuf, Vec<usize>)> = groups.into_iter().collect();
    let grouped = par_map(&groups, |(reference, members)| {
        let expected = render(reference);
        members
            .iter()
            .map(|&index| (index, run_reftest(&tests[index], &expected)))
            .collect::<Vec<_>>()
    });
    let mut results: Vec<Option<Result<(), String>>> = tests.iter().map(|_| None).collect();
    for (index, result) in grouped.into_iter().flatten() {
        results[index] = Some(result);
    }
    let results: Vec<Result<(), String>> = results.into_iter().map(Option::unwrap).collect();
    if update_expectations() {
        let failing = tests
            .iter()
            .zip(&results)
            .filter(|(_, result)| result.is_err())
            .map(|(test, _)| test_name(test))
            .collect();
        write_expectations("reftest-expectations.txt", &failing);
        return;
    }
    let mut problems = stale_expectations(&known_failures);
    for (test, result) in tests.iter().zip(results) {
        let name = test_name(test);
        match (result, known_failures.contains(&name)) {
            (Ok(()), false) | (Err(_), true) => {}
            (Err(why), false) => problems.push(format!("FAIL {}: {}", name, why)),
            (Ok(()), true) => problems.push(format!(
                "UNEXPECTED PASS {} (remove it from tests/wpt/reftest-expectations.txt)",
                name
            )),
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
