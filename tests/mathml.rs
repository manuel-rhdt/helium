//! Simple MathML tests, expressed against the layout dump.

mod common;

use serde_json::Value;

fn dump(name: &str, html: &str) -> Value {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join(format!("{}.html", name));
    std::fs::write(&input, html).unwrap();
    std::fs::create_dir_all(dir.join("wpt-pdfs")).unwrap();
    let output = common::umbruch()
        .unwrap()
        .arg("--dump-layout")
        .arg("-")
        .arg(&input)
        .arg(dir.join("wpt-pdfs").join(format!("mathml__{}.pdf", name)))
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

fn height(dump: &Value, id: &str) -> f64 {
    find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {}", id))["border_box"]
        ["height"]
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
    let tags: Vec<&str> = math["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["tag"].as_str().unwrap())
        .collect();
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
    let dump = dump(
        "font",
        r#"<p>a <math><mi>x</mi><mo>+</mo><mn>1</mn></math> b</p>"#,
    );
    let fonts = fonts(&dump);
    assert!(
        fonts.iter().any(|f| f.contains("Math")),
        "no math font in {:?}",
        fonts
    );
    assert!(
        fonts.iter().any(|f| !f.contains("Math")),
        "surrounding text should keep its font: {:?}",
        fonts
    );
}

#[test]
fn text_after_math_flows_on() {
    let dump = dump(
        "flow",
        r#"<style>body { margin: 0 }</style>
           <p id="p" style="margin: 0"><math><mi>x</mi></math></p><div id="d">after</div>"#,
    );
    let y = |id: &str| {
        find(&dump["document"], id).unwrap()["border_box"]["y"]
            .as_f64()
            .unwrap()
    };
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
    assert!(
        (px - none - 48.0).abs() < 0.01,
        "none: {}, 48px: {}",
        none,
        px
    );
}

#[test]
fn mspace_supports_em_and_named_spaces() {
    let none = gap_between_a_and_b("mspace_none2", "0");
    // the font size is 16px
    let em = gap_between_a_and_b("mspace_em", "2em");
    assert!(
        (em - none - 32.0).abs() < 0.01,
        "none: {}, 2em: {}",
        none,
        em
    );
    let named = gap_between_a_and_b("mspace_named", "thickmathspace");
    assert!(
        (named - none - 16.0 * 5.0 / 18.0).abs() < 0.05,
        "none: {}, thick: {}",
        none,
        named
    );
}

#[test]
fn mspace_with_negative_width_moves_back() {
    let none = gap_between_a_and_b("mspace_none3", "0");
    let negative = gap_between_a_and_b("mspace_negative", "-8px");
    assert!(
        (none - negative - 8.0).abs() < 0.01,
        "none: {}, -8px: {}",
        none,
        negative
    );
}

/// The glyph ids of the `TJ` operators of the text, in order (the numbers between them are kerns).
fn glyph_ids(dump: &Value) -> Vec<u64> {
    let mut ids = Vec::new();
    for run in dump["paint"]["text"].as_array().unwrap() {
        for op in run["ops"].as_array().unwrap() {
            if op["op"] == "kerned" {
                for item in op["items"].as_array().unwrap() {
                    if let Some(glyphs) = item.as_array() {
                        ids.extend(glyphs.iter().map(|g| g.as_u64().unwrap()));
                    }
                }
            }
        }
    }
    ids
}

fn glyph_count_in_math_font(dump: &Value) -> usize {
    glyph_ids(dump).len()
}

#[test]
fn multi_letter_identifiers_are_upright_unless_a_variant_is_given() {
    // the glyph ids of the same letters in italic and in normal style differ
    let glyphs = |name: &str, math: &str| {
        glyph_ids(&dump(name, &format!("<p><math>{}</math></p>", math)))
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
    assert_eq!(
        glyph_count_in_math_font(&dump("mi_count", "<math><mi>sin</mi></math>")),
        3
    );
}

#[test]
fn fractions_have_tex_like_space_on_both_sides() {
    // In TeX a fraction is flanked by \nulldelimiterspace (0.12em), so `\frac{a}{b}e^x` does not
    // touch the fraction rule. The glyph after the fraction must move by 2 * 0.12em when the padding
    // is present, compared to a fraction whose padding is removed.
    let x_of_e = |name: &str, style: &str| {
        let dump = dump(
            name,
            &format!(
                r#"<style>body {{ margin: 0 }} {}</style>
                   <p style="margin: 0"><math><mfrac><mi>a</mi><mi>b</mi></mfrac><msup><mi>e</mi><mi>x</mi></msup></math></p>"#,
                style
            ),
        );
        let xs = math_glyph_xs(&dump);
        assert_eq!(xs.len(), 4, "expected a, b, e and x: {:?}", xs);
        // glyph order is a, b, e, x
        xs[2]
    };
    let padded = x_of_e("mfrac_padded", "");
    let bare = x_of_e("mfrac_bare", "mfrac { padding-inline: 0 }");
    // the font size is 16px
    assert!(
        (padded - bare - 2.0 * 0.12 * 16.0).abs() < 0.05,
        "padded: {}, bare: {}",
        padded,
        bare
    );
}

fn math_height(name: &str, math: &str) -> f64 {
    let dump = dump(
        name,
        &format!(
            r#"<style>body {{ margin: 0 }}</style><p style="margin: 0"><math id="m" display="inline">{}</math></p>"#,
            math
        ),
    );
    height(&dump, "m")
}

fn math_width(name: &str, math: &str) -> f64 {
    let dump = dump(
        name,
        &format!(r#"<style>body {{ margin: 0 }}</style><p style="margin: 0"><math id="m" display="inline">{}</math></p>"#, math),
    );
    find(&dump["document"], "m").unwrap()["border_box"]["width"].as_f64().unwrap()
}

#[test]
fn displaystyle_attribute_selects_the_display_style() {
    // a large operator is only large in display style
    let sum = |attr: &str| format!(r#"<mstyle {}><mo>∑</mo></mstyle>"#, attr);
    let inline = math_height("ds_default", &sum(""));
    let display = math_height("ds_true", &sum(r#"displaystyle="true""#));
    let off = math_height("ds_false", &sum(r#"displaystyle="false""#));
    assert!(display > inline + 1.0, "display: {}, inline: {}", display, inline);
    assert!((off - inline).abs() < 0.01, "off: {}, inline: {}", off, inline);
}

#[test]
fn scriptlevel_attribute_sets_the_script_level() {
    let width = |name: &str, attr: &str| {
        math_width(name, &format!(r#"<mstyle {}><mi>x</mi><mi>x</mi></mstyle>"#, attr))
    };
    let base = width("sl_none", "");
    let one = width("sl_one", r#"scriptlevel="1""#);
    let plus = width("sl_plus", r#"scriptlevel="+1""#);
    let zero = width("sl_zero", r#"scriptlevel="0""#);
    assert!(one < base - 1.0, "one: {}, base: {}", one, base);
    assert!((plus - one).abs() < 0.01, "plus: {}, one: {}", plus, one);
    assert!((zero - base).abs() < 0.01, "zero: {}, base: {}", zero, base);
    // an invalid value is ignored
    assert!((width("sl_bad", r#"scriptlevel="big""#) - base).abs() < 0.01);
}

#[test]
fn mathsize_attribute_sets_the_font_size() {
    let width = |name: &str, attr: &str| {
        math_width(name, &format!(r#"<mstyle {}><mi>x</mi></mstyle>"#, attr))
    };
    let base = width("ms_none", "");
    let double = width("ms_200", r#"mathsize="200%""#);
    let em = width("ms_em", r#"mathsize="2em""#);
    assert!((double / base - 2.0).abs() < 0.02, "double: {}, base: {}", double, base);
    assert!((em - double).abs() < 0.01, "em: {}, double: {}", em, double);
    assert!((width("ms_bad", r#"mathsize="huge""#) - base).abs() < 0.01);
}

#[test]
fn table_cells_have_padding() {
    // MathML Core: `mtd { padding: 0.5ex 0.4em }`
    let cell = |name: &str, style: &str| {
        math_width(
            name,
            &format!(r#"<mtable><mtr><mtd {}><mi>x</mi></mtd></mtr></mtable>"#, style),
        )
    };
    let padded = cell("mtd_padded", "");
    let bare = cell("mtd_bare", r#"style="padding: 0""#);
    // the font size is 16px
    assert!(
        (padded - bare - 2.0 * 0.4 * 16.0).abs() < 0.05,
        "padded: {}, bare: {}",
        padded,
        bare
    );
}

#[test]
fn annotation_xml_is_not_rendered() {
    // an HTML integration point: its content is parsed as HTML, but it is annotation, not content
    let dump = dump(
        "annotation_xml",
        r#"<style>body { margin: 0 }</style>
           <p style="margin: 0"><math><semantics><mi>x</mi><annotation-xml encoding="text/html"><img id="a" width="20" height="20" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='20'%3E%3Crect width='20' height='20'/%3E%3C/svg%3E"></annotation-xml></semantics></math></p>"#,
    );
    let img = find(&dump["document"], "a");
    assert!(
        img.map_or(true, |img| img["border_box"].is_null()),
        "the image in annotation-xml was laid out: {:?}",
        img
    );
}

const RED_40X30: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='30'%3E%3Crect width='40' height='30' fill='%23c33'/%3E%3C/svg%3E";

fn box_of(dump: &Value, id: &str) -> [f64; 4] {
    let b = &find(&dump["document"], id).unwrap_or_else(|| panic!("no element with id {}", id))["border_box"];
    ["x", "y", "width", "height"].map(|key| b[key].as_f64().unwrap())
}

fn formula(name: &str, math: &str) -> Value {
    dump(
        name,
        &format!(
            r#"<style>body {{ margin: 0 }}</style><p style="margin: 0"><math id="m" display="inline">{}</math></p>"#,
            math
        ),
    )
}

#[test]
fn an_image_in_mtext_is_laid_out_in_the_formula() {
    let without = formula("img_none", "<mi>x</mi><mtext></mtext><mi>y</mi>");
    let with = formula(
        "img_mtext",
        &format!(r#"<mi>x</mi><mtext><img id="i" src="{}"></mtext><mi>y</mi>"#, RED_40X30),
    );
    let [_, _, width_without, _] = box_of(&without, "m");
    let [math_x, math_y, width, height] = box_of(&with, "m");
    let [x, y, image_width, image_height] = box_of(&with, "i");
    // the formula is as wide as the image makes it, and as high
    assert!((width - width_without - 40.0).abs() < 0.01, "{} {}", width, width_without);
    assert!(height >= 30.0 - 0.01, "height: {}", height);
    assert!((image_width - 40.0).abs() < 0.01 && (image_height - 30.0).abs() < 0.01);
    // the image is inside of the formula, between x and y
    assert!(x > math_x && x + image_width < math_x + width, "x: {}, math: {}", x, math_x);
    assert!(y >= math_y - 0.01 && y + image_height <= math_y + height + 0.01);
}

#[test]
fn the_image_stands_on_the_baseline_between_text() {
    let with = formula(
        "img_text",
        &format!(r#"<mtext id="t">ab<img id="i" src="{}">cd</mtext>"#, RED_40X30),
    );
    let text_only = formula("img_text_only", r#"<mtext id="t">abcd</mtext>"#);
    let [t_x, _, t_width, _] = box_of(&with, "t");
    let [i_x, i_y, _, i_height] = box_of(&with, "i");
    let [_, _, plain_width, _] = box_of(&text_only, "t");
    // the image is somewhere in the middle of the text
    assert!(i_x > t_x && i_x < t_x + t_width - 1.0, "i: {}, t: {} + {}", i_x, t_x, t_width);
    assert!((t_width - plain_width - 40.0).abs() < 0.5, "{} {}", t_width, plain_width);
    // its bottom edge is the baseline of the token, and the text is on it
    let baseline = i_y + i_height;
    let glyph_ys: Vec<f64> = math_glyph_ys(&with);
    assert!(!glyph_ys.is_empty());
    for glyph_y in glyph_ys {
        assert!((glyph_y - baseline).abs() < 0.5, "glyph: {}, baseline: {}", glyph_y, baseline);
    }
}

fn math_glyph_ys(dump: &Value) -> Vec<f64> {
    let mut ys = Vec::new();
    for run in dump["paint"]["text"].as_array().unwrap() {
        for op in run["ops"].as_array().unwrap() {
            if op["op"] == "transform" {
                ys.push(op["matrix"][5].as_f64().unwrap());
            }
        }
    }
    ys
}

#[test]
fn an_image_in_mi_and_its_css_size() {
    let sized = formula(
        "img_css",
        &format!(r#"<mi><img id="i" style="width: 20px" src="{}"></mi>"#, RED_40X30),
    );
    let [_, _, width, height] = box_of(&sized, "i");
    // the ratio of the image is kept when only one side is given
    assert!((width - 20.0).abs() < 0.01 && (height - 15.0).abs() < 0.01, "{} x {}", width, height);
    let [_, _, math_width, _] = box_of(&sized, "m");
    assert!((math_width - 20.0).abs() < 0.01, "math: {}", math_width);
}

#[test]
fn other_html_in_a_token_is_left_out() {
    let plain = formula("html_plain", "<mtext>ab</mtext>");
    let with_span = formula(
        "html_span",
        r#"<mtext>a<span id="s" style="border: 1px solid; padding: 2px">zzz</span>b</mtext>"#,
    );
    assert_eq!(box_of(&plain, "m")[2], box_of(&with_span, "m")[2]);
    assert!(find(&with_span["document"], "s").unwrap()["border_box"].is_null());
}

#[test]
fn inline_svg_in_a_fraction_and_a_square_root() {
    let svg = |id: &str| {
        format!(
            r##"<svg id="{}" width="30" height="20" viewBox="0 0 30 20"><rect width="30" height="20" fill="#639"/></svg>"##,
            id
        )
    };
    let plain = formula("svg_plain", "<mfrac><mtext>a</mtext><mtext>b</mtext></mfrac>");
    let with = formula(
        "svg_fraction",
        &format!(
            "<mfrac><mtext>{}</mtext><mtext>{}</mtext></mfrac><msqrt><mtext>{}</mtext></msqrt>",
            svg("top"), svg("bottom"), svg("root")
        ),
    );
    let [_, _, _, plain_height] = box_of(&plain, "m");
    let [top_x, top_y, top_width, top_height] = box_of(&with, "top");
    let [_, bottom_y, _, _] = box_of(&with, "bottom");
    let [_, _, root_width, root_height] = box_of(&with, "root");
    let [math_x, math_y, math_width, math_height] = box_of(&with, "m");
    assert_eq!((top_width, top_height), (30.0, 20.0));
    assert_eq!((root_width, root_height), (30.0, 20.0));
    // the numerator is above the denominator, and the fraction is taller than with text
    assert!(top_y + top_height <= bottom_y, "{} {}", top_y + top_height, bottom_y);
    assert!(math_height > plain_height + 10.0, "{} {}", math_height, plain_height);
    // everything is inside of the formula
    assert!(top_x >= math_x && top_x + top_width <= math_x + math_width);
    assert!(top_y >= math_y - 0.01 && top_y + top_height <= math_y + math_height + 0.01);
}

#[test]
fn vertical_align_moves_an_image_off_the_baseline() {
    // the distance from the baseline of the text up to the bottom edge of the image
    let raised = |name: &str, style: &str| {
        let dump = formula(
            name,
            &format!(r#"<mtext>ab<img id="i" style="{}" src="{}">cd</mtext>"#, style, RED_40X30),
        );
        let [_, y, _, height] = box_of(&dump, "i");
        let [_, math_y, _, math_height] = box_of(&dump, "m");
        let baseline = math_glyph_ys(&dump)[0];
        // the formula always has room for the image
        assert!(y >= math_y - 0.01 && y + height <= math_y + math_height + 0.01, "{}: {:?}", style, (y, height, math_y, math_height));
        (baseline - (y + height), y + height / 2.0, baseline)
    };
    let close = |a: f64, b: f64, what: &str| assert!((a - b).abs() < 0.01, "{}: {} != {}", what, a, b);

    close(raised("va_baseline", "").0, 0.0, "baseline");
    close(raised("va_up", "vertical-align: 6px").0, 6.0, "6px up");
    close(raised("va_down", "vertical-align: -10px").0, -10.0, "10px down");
    // a percentage is of the line-height
    close(raised("va_percent", "line-height: 20px; vertical-align: 50%").0, 10.0, "50% of 20px");
    // `top`, `sub` and the like refer to a line box that a formula does not have
    close(raised("va_top", "vertical-align: top").0, 0.0, "top");
    // `middle` centers on the math axis: above the baseline, by a fraction of the font size
    let (_, middle, baseline) = raised("va_middle", "vertical-align: middle");
    let axis = baseline - middle;
    assert!(axis > 2.0 && axis < 8.0, "axis: {}", axis);
}
