//! A container with columns that goes over pages offers the pages fragments that end earlier than
//! the one that fills the page (`UMBRUCH_EARLIER_FRAGMENTS`), and the pages take them where that
//! leaves less room unused. The document is a generated one (`tools/looseness_bench.py`, the kind
//! `headings`): columns with headings that must not end a column, over pages.

mod common;

use serde_json::Value;

/// The pages of the document, and the cube of the room (in lines of 20px) that is left on the
/// pages that do not end at a forced break and are not the last: the cost that the pages pay for it.
fn room(earlier: Option<u32>) -> (usize, f64) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let report = dir.join(format!("earlier_fragments_{}.json", earlier.map_or("default".into(), |n| n.to_string())));
    let mut command = common::umbruch().unwrap();
    command
        .args(["--no-system-fonts", "--dump-pagination"])
        .arg(&report)
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/earlier_fragments.html"))
        .arg(dir.join("earlier_fragments.pdf"));
    if let Some(earlier) = earlier {
        command.env("UMBRUCH_EARLIER_FRAGMENTS", earlier.to_string());
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let pages = report["pages"].as_array().unwrap();
    let cube = pages[..pages.len() - 1]
        .iter()
        .filter(|page| page["ends_with"] != "Forced")
        .map(|page| (page["room"].as_f64().unwrap() / 20.0).powi(3))
        .sum();
    (pages.len(), cube)
}

#[test]
fn a_container_that_ends_a_fragment_earlier_leaves_less_room_unused() {
    let (pages, only_the_full_one) = room(Some(0));
    let (more_pages, with_earlier_ones) = room(None);
    assert_eq!(pages, more_pages);
    assert!(with_earlier_ones < only_the_full_one, "{only_the_full_one} and {with_earlier_ones}");
}

#[test]
fn the_fragments_are_chosen_the_same_way_every_time() {
    assert_eq!(room(None), room(None));
}
