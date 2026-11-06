//! Layout tests for `position: absolute` that are easier to express against the layout dump than
//! with `data-expected-*` attributes, for example because they involve several pages.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("absolute__{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| find(child, id))
}

fn border_box(dump: &Value, id: &str) -> (i64, f64, f64, f64, f64) {
    let node = find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {}", id));
    let b = &node["border_box"];
    (
        node["page"].as_i64().unwrap(),
        b["x"].as_f64().unwrap(),
        b["y"].as_f64().unwrap(),
        b["width"].as_f64().unwrap(),
        b["height"].as_f64().unwrap(),
    )
}

#[test]
fn box_with_static_position_stays_on_the_page_of_its_flow() {
    // six blocks of 300px do not fit on one page
    let blocks: String = (0..6)
        .map(|i| {
            format!(
                r#"<div id="block{i}" style="height: 300px; background: #ddd">
                     <div id="abs{i}" style="position: absolute; width: 50px; height: 20px; background: blue"></div>
                   </div>"#
            )
        })
        .collect();
    let dump = dump(
        "abs_static_pagination",
        &format!("<style>body {{ margin: 0 }}</style>{}", blocks),
    );

    let pages: Vec<i64> = (0..6)
        .map(|i| border_box(&dump, &format!("block{}", i)).0)
        .collect();
    assert!(
        pages[0] == 0 && pages[5] > 0,
        "the blocks should end up on different pages: {:?}",
        pages
    );

    // every box is where its static position is: at the top left corner of its block
    for i in 0..6 {
        let block = border_box(&dump, &format!("block{}", i));
        let abs = border_box(&dump, &format!("abs{}", i));
        assert_eq!(
            (abs.0, abs.1, abs.2),
            (block.0, block.1, block.2),
            "box {}",
            i
        );
    }
}

#[test]
fn box_with_explicit_offsets_is_placed_on_the_first_page() {
    let blocks: String = (0..6)
        .map(|_| r#"<div style="height: 300px"></div>"#)
        .collect();
    let dump = dump(
        "abs_explicit_first_page",
        &format!(
            r#"<style>body {{ margin: 0 }}</style>
               <div id="first" style="height: 1px"></div>{}
               <div id="abs" style="position: absolute; left: 10px; top: 20px; width: 5px; height: 5px"></div>"#,
            blocks
        ),
    );
    let first = border_box(&dump, "first");
    let abs = border_box(&dump, "abs");
    assert_eq!(abs.0, 0, "the box is placed on the first page");
    // relative to the content area of the first page
    assert_eq!((abs.1 - first.1, abs.2 - first.2), (10.0, 20.0));
}

#[test]
fn absolute_boxes_do_not_take_up_space_in_the_flow() {
    let dump = dump(
        "abs_out_of_flow",
        r#"<style>body { margin: 0 }</style>
           <div id="a" style="height: 10px"></div>
           <div id="abs" style="position: absolute; height: 500px"></div>
           <div id="b" style="height: 10px"></div>"#,
    );
    let (_, _, a_y, _, _) = border_box(&dump, "a");
    let (_, _, b_y, _, _) = border_box(&dump, "b");
    let (_, _, abs_y, _, abs_h) = border_box(&dump, "abs");
    assert_eq!(b_y - a_y, 10.0);
    assert_eq!(abs_h, 500.0);
    // static position: directly below `a`
    assert_eq!(abs_y - a_y, 10.0);
}

#[test]
fn display_inline_is_blockified() {
    let dump = dump(
        "abs_blockified",
        r#"<style>body { margin: 0 }</style>
           <span id="s" style="position: absolute; left: 0; top: 0; width: 40px; height: 30px; background: red"></span>"#,
    );
    let (_, _, _, w, h) = border_box(&dump, "s");
    assert_eq!((w, h), (40.0, 30.0));
}
