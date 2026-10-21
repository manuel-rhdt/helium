//! Simple MathML tests, expressed against the layout dump.

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("mathml__{}.pdf", name)))
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

fn height(dump: &Value, id: &str) -> f64 {
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {}", id))["border_box"]["height"]
        .as_f64()
        .unwrap()
}

fn fonts(dump: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for run in dump["paint"]["text"].as_array().unwrap() {
        for op in run["ops"].as_array().unwrap() {
            if op["op"] == "font" {
                names.push(op["name"].as_str().unwrap().to_string());
            }
        }
    }
    names
}

#[test]
fn math_elements_are_kept_in_the_tree() {
    let dump = dump(
        "tree",
        r#"<p id="p"><math id="m"><mi>x</mi><mo>+</mo><mn id="n">1</mn></math></p>"#,
    );
    let math = find(&dump["document"], "m").unwrap();
    assert_eq!(math["tag"], "math");
    let tags: Vec<&str> = math["children"].as_array().unwrap().iter().map(|c| c["tag"].as_str().unwrap()).collect();
    assert_eq!(tags, ["mi", "mo", "mn"]);
    assert_eq!(find(&dump["document"], "n").unwrap()["tag"], "mn");
}

#[test]
fn inline_math_does_not_break_the_line() {
    let dump = dump(
        "inline",
        r#"<style>body { margin: 0 }</style>
           <p id="with" style="margin: 0">a <math><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>
           <p id="without" style="margin: 0">a x + 1 b</p>"#,
    );
    assert_eq!(height(&dump, "with"), height(&dump, "without"));
}

#[test]
fn math_text_uses_a_math_font() {
    let dump = dump("font", r#"<p>a <math><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>"#);
    let fonts = fonts(&dump);
    assert!(fonts.iter().any(|f| f.contains("Math")), "no math font in {:?}", fonts);
    assert!(fonts.iter().any(|f| !f.contains("Math")), "surrounding text should keep its font: {:?}", fonts);
}

#[test]
fn text_after_math_flows_on() {
    let dump = dump(
        "flow",
        r#"<style>body { margin: 0 }</style>
           <p id="p" style="margin: 0"><math><mi>x</mi></math></p><div id="d">after</div>"#,
    );
    let y = |id: &str| find(&dump["document"], id).unwrap()["border_box"]["y"].as_f64().unwrap();
    assert!(y("d") >= y("p") + height(&dump, "p") - 0.001);
}

/// The x positions of the glyphs of the math font, in drawing order.
fn math_glyph_xs(dump: &Value) -> Vec<f64> {
    let mut xs = Vec::new();
    for run in dump["paint"]["text"].as_array().unwrap() {
        for op in run["ops"].as_array().unwrap() {
            if op["op"] == "transform" {
                xs.push(op["matrix"][4].as_f64().unwrap());
            }
        }
    }
    xs
}

fn gap_between_a_and_b(name: &str, width: &str) -> f64 {
    let dump = dump(
        name,
        &format!(
            r#"<style>body {{ margin: 0 }}</style>
               <p style="margin: 0"><math><mi>a</mi><mspace width="{}"/><mi>b</mi></math></p>"#,
            width
        ),
    );
    let xs = math_glyph_xs(&dump);
    assert_eq!(xs.len(), 2, "expected exactly two glyphs: {:?}", xs);
    xs[1] - xs[0]
}

#[test]
fn mspace_adds_horizontal_space() {
    let none = gap_between_a_and_b("mspace_none", "0");
    let px = gap_between_a_and_b("mspace_px", "48px");
    assert!((px - none - 48.0).abs() < 0.01, "none: {}, 48px: {}", none, px);
}

#[test]
fn mspace_supports_em_and_named_spaces() {
    let none = gap_between_a_and_b("mspace_none2", "0");
    // the font size is 16px
    let em = gap_between_a_and_b("mspace_em", "2em");
    assert!((em - none - 32.0).abs() < 0.01, "none: {}, 2em: {}", none, em);
    let named = gap_between_a_and_b("mspace_named", "thickmathspace");
    assert!((named - none - 16.0 * 5.0 / 18.0).abs() < 0.05, "none: {}, thick: {}", none, named);
}

#[test]
fn mspace_with_negative_width_moves_back() {
    let none = gap_between_a_and_b("mspace_none3", "0");
    let negative = gap_between_a_and_b("mspace_negative", "-8px");
    assert!((none - negative - 8.0).abs() < 0.01, "none: {}, -8px: {}", none, negative);
}

fn glyph_count_in_math_font(dump: &Value) -> usize {
    dump["paint"]["text"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|run| run["ops"].as_array().unwrap())
        .filter(|op| op["op"] == "glyphs")
        .map(|op| op["glyphs"].as_array().unwrap().len())
        .sum()
}

#[test]
fn multi_letter_identifiers_are_upright_unless_a_variant_is_given() {
    // the glyph ids of the same letters in italic and in normal style differ
    let glyphs = |name: &str, math: &str| {
        let dump = dump(name, &format!("<p><math>{}</math></p>", math));
        let mut ids = Vec::new();
        for run in dump["paint"]["text"].as_array().unwrap() {
            for op in run["ops"].as_array().unwrap() {
                if op["op"] == "glyphs" {
                    ids.extend(op["glyphs"].as_array().unwrap().iter().map(|g| g.as_u64().unwrap()));
                }
            }
        }
        ids
    };
    let multi = glyphs("mi_multi", "<mi>sin</mi>");
    let upright = glyphs("mi_multi_normal", r#"<mi mathvariant="normal">sin</mi>"#);
    let italic = glyphs("mi_multi_italic", r#"<mi mathvariant="italic">sin</mi>"#);
    assert_eq!(multi, upright);
    assert_ne!(multi, italic);

    let single = glyphs("mi_single", "<mi>x</mi>");
    let single_italic = glyphs("mi_single_italic", r#"<mi mathvariant="italic">x</mi>"#);
    let single_normal = glyphs("mi_single_normal", r#"<mi mathvariant="normal">x</mi>"#);
    assert_eq!(single, single_italic);
    assert_ne!(single, single_normal);
    assert_eq!(glyph_count_in_math_font(&dump("mi_count", "<math><mi>sin</mi></math>")), 3);
}
