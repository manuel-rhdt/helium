//! Tests for `display: contents`, expressed against the layout dump.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("contents_{name}.html"));
    std::fs::write(&input, html).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("contents_{name}.pdf")))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"].as_array()?.iter().find_map(|child| find(child, id))
}

fn border_box(dump: &Value, id: &str) -> Option<(f64, f64, f64, f64)> {
    let b = &find(&dump["document"], id)?["border_box"];
    Some((b["x"].as_f64()?, b["y"].as_f64()?, b["width"].as_f64()?, b["height"].as_f64()?))
}

#[test]
fn an_element_with_display_contents_has_no_box() {
    let dump = dump(
        "no_box",
        r#"<style>body { margin: 0 } #c { display: contents; border: 5px solid; padding: 20px; margin: 30px }
        p { margin: 0 }</style><div id="outer"><div id="c"><p id="a">a</p><p id="b">b</p></div></div>"#,
    );
    // no box, so no border, padding or margin: the children are the ones of the outer element
    assert_eq!(border_box(&dump, "c"), None);
    let (a, b) = (border_box(&dump, "a").unwrap(), border_box(&dump, "b").unwrap());
    let outer = border_box(&dump, "outer").unwrap();
    assert_eq!((a.0, a.1), (outer.0, outer.1));
    assert!((b.1 - (a.1 + a.3)).abs() < 0.01, "{a:?} {b:?}");
    assert!((outer.3 - (a.3 + b.3)).abs() < 0.01, "{outer:?} {a:?} {b:?}");
}

#[test]
fn the_children_of_a_contents_element_are_flex_items() {
    let dump = dump(
        "flex",
        r#"<style>body { margin: 0 } #f { display: flex; gap: 40px } #c { display: contents }
        span { width: 10px; display: block }</style>
        <div id="f"><div id="c"><span id="x">x</span><span id="y">y</span></div><span id="z">z</span></div>"#,
    );
    let xs: Vec<f64> = ["x", "y", "z"].iter().map(|id| border_box(&dump, id).unwrap().0).collect();
    let flex = border_box(&dump, "f").unwrap().0;
    assert_eq!(xs, [flex, flex + 50.0, flex + 100.0]);
}

#[test]
fn inherited_properties_pass_through_a_contents_element() {
    let dump = dump(
        "inherit",
        r#"<style>body { margin: 0 } #c { display: contents; width: 500px; height: 300px }
        #i { display: block }</style><div style="width: 100px"><div id="c"><div id="i">x</div></div></div>"#,
    );
    // the width and height of the element with no box do nothing; the child is as wide as the parent
    assert_eq!(border_box(&dump, "i").unwrap().2, 100.0);
}
