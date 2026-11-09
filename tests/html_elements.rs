//! `<hr>`, form controls and `<details>`, expressed against the layout dump.

mod common;

use serde_json::{Value, json};

const PAGE: &str = "@page{width:300pt;height:300pt;margin:20pt}body{margin:0}";

fn dump(name: &str, body: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let input = dir.join(format!("elements_{}.html", name));
    std::fs::write(&input, format!("<style>{}</style>{}", PAGE, body)).unwrap();
    let output = common::helium()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join(format!("elements_{}.pdf", name)))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn rects(dump: &Value) -> Vec<Value> {
    dump["paint"]["rects"].as_array().unwrap().clone()
}

fn colored(rects: &[Value], color: [u8; 4]) -> Vec<Value> {
    rects
        .iter()
        .filter(|rect| rect["color"] == json!(color))
        .cloned()
        .collect()
}

fn f(rect: &Value, key: &str) -> f64 {
    rect[key].as_f64().unwrap()
}

/// The element with the given `id`, searched depth first.
fn node<'a>(dump: &'a Value, id: &str) -> &'a Value {
    fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
        if node["id"] == id {
            return Some(node);
        }
        node["children"]
            .as_array()
            .and_then(|children| children.iter().find_map(|child| find(child, id)))
    }
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element for #{id}"))
}

fn box_of<'a>(dump: &'a Value, id: &str, key: &str) -> f64 {
    node(dump, id)["content_box"][key].as_f64().unwrap()
}

/// The text of every text node in the document, in document order.
fn texts(dump: &Value) -> Vec<String> {
    fn walk(node: &Value, out: &mut Vec<String>) {
        if let Some(text) = node["text"].as_str() {
            out.push(text.to_string());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, out);
        }
    }
    let mut out = vec![];
    walk(&dump["document"], &mut out);
    out
}

#[test]
fn a_default_hr_draws_a_rule() {
    let dump = dump("hr_default", "<p id=a>a</p><hr id=r><p id=b>b</p>");
    // 1px at the top and 1px at the bottom of an empty box, abutting to a 2px line
    let rules = colored(&rects(&dump), [128, 128, 128, 255]);
    assert_eq!(rules.len(), 2, "{rules:?}");
    for rule in &rules {
        assert!((f(rule, "height") - 1.0).abs() < 0.01, "{rules:?}");
        // 300pt page with 20pt margins is 346.67px wide
        assert!((f(rule, "width") - 346.67).abs() < 0.1, "{rules:?}");
    }
    let ys: Vec<f64> = rules.iter().map(|rule| f(rule, "y")).collect();
    assert!((ys[0] - ys[1]).abs() < 1.01, "the two rules are apart: {rules:?}");

    // and the paragraphs are still there, one above and one below the rule
    assert!(box_of(&dump, "a", "y") < ys[0] && box_of(&dump, "b", "y") > ys[0]);
}

#[test]
fn an_hr_with_a_border_is_one_contiguous_bar() {
    let dump = dump("hr_border", "<hr style=\"border:2pt solid red\">");
    let borders = colored(&rects(&dump), [255, 0, 0, 255]);
    let horizontal: Vec<Value> = borders
        .iter()
        .filter(|rect| f(rect, "width") > f(rect, "height"))
        .cloned()
        .collect();
    assert_eq!(horizontal.len(), 2, "{borders:?}");

    let (first, second) = (&horizontal[0], &horizontal[1]);
    let (top, bottom) = if f(first, "y") < f(second, "y") {
        (first, second)
    } else {
        (second, first)
    };
    let gap = f(bottom, "y") - (f(top, "y") + f(top, "height"));
    assert!(gap.abs() < 0.01, "the borders leave a {gap}px gap: {borders:?}");
}

#[test]
fn form_controls_are_boxes_with_their_own_text() {
    let dump = dump(
        "form",
        "<input id=i value=\"text\"><button id=b>Btn</button>\
         <select id=s><option>o1</option></select><textarea id=t>ta</textarea>\
         <input id=c type=\"checkbox\" checked>",
    );

    // every control has a border (its box) and its text is not merged with the next control
    let borders = colored(&rects(&dump), [118, 118, 118, 255]);
    assert!(
        !borders.is_empty(),
        "no control was drawn: {}",
        rects(&dump).len()
    );
    let texts = texts(&dump);
    for expected in ["Btn", "o1", "ta"] {
        assert!(
            texts.iter().any(|text| text == expected),
            "{expected} is missing from {texts:?}"
        );
    }

    // the value of an `<input>` is not a text node of the document, so it shows up only in the
    // width of the box (an empty control is 10px wide: two 4px padding and two 1px borders)
    let value_width = box_of(&dump, "i", "width");
    assert!(
        value_width > 20.0,
        "the value of the input is not rendered, its box is {value_width}px wide"
    );
    let checkbox_width = box_of(&dump, "c", "width");
    assert!(
        checkbox_width > 5.0,
        "the checkbox does not show its state, its box is {checkbox_width}px wide"
    );

    // the controls sit next to each other, not on top of each other
    let mut xs = vec![];
    for id in ["i", "b", "s", "t", "c"] {
        xs.push(box_of(&dump, id, "x"));
    }
    assert!(
        xs.windows(2).all(|pair| pair[0] < pair[1]),
        "the controls are not laid out in a row: {xs:?}"
    );
}

#[test]
fn details_is_not_flattened_into_one_line() {
    let dump = dump(
        "details",
        "<details><summary id=s>Sum</summary><p id=b>Body</p></details>",
    );
    assert!(
        box_of(&dump, "s", "y") < box_of(&dump, "b", "y"),
        "the summary and the content are on the same line"
    );
}
