//! A standalone SVG file as the input.
//!
//! The file is read as XML (see `parser::generate_render_tree_xml`), and the layout, which expects
//! an `html` element with a `body`, gets one around the `<svg>` (`parser::into_html_document`).
//! What is added here is the style sheet of that shell: one page of the size of the root element,
//! with no margin, and the `<svg>` as a block so that no line box surrounds it.

/// The size of a page in CSS pixels (1/96 inch) when the root element does not say.
const DEFAULT_WIDTH: f64 = 300.0;
const DEFAULT_HEIGHT: f64 = 150.0;

/// The style sheet that makes the SVG the whole page, from the `width`, `height` and `viewBox`
/// attributes of the root element.
pub fn page_css(width: Option<&str>, height: Option<&str>, view_box: Option<&str>) -> String {
    let (width, height) = root_size(width, height, view_box);
    format!(
        "@page {{ size: {width}px {height}px; margin: 0 }} \
         html, body {{ margin: 0; padding: 0 }} \
         body > svg {{ display: block }}",
        width = number(width),
        height = number(height),
    )
}

fn number(value: f64) -> String {
    let text = format!("{:.4}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The size of the root element in CSS pixels: `width` and `height`, else the `viewBox`, else the
/// size that browsers use for an SVG that says nothing.
fn root_size(width: Option<&str>, height: Option<&str>, view_box: Option<&str>) -> (f64, f64) {
    let view_box = view_box.and_then(|value| {
        let numbers: Vec<f64> = value
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect();
        match numbers[..] {
            [_, _, w, h] if w > 0.0 && h > 0.0 => Some((w, h)),
            _ => None,
        }
    });
    match (width.and_then(parse_length), height.and_then(parse_length), view_box) {
        (Some(w), Some(h), _) => (w, h),
        // one length and the proportions of the view box
        (Some(w), None, Some((vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((vw, vh))) => (h * vw / vh, h),
        (None, None, Some(size)) => size,
        (Some(w), None, None) => (w, DEFAULT_HEIGHT),
        (None, Some(h), None) => (DEFAULT_WIDTH, h),
        (None, None, None) => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    }
}

/// A length in CSS pixels. Percentages and relative units are not known without a context.
fn parse_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let end = value
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(value.len());
    // `e` of a unit such as `em` must not be taken for an exponent
    let (digits, unit) = match value[..end].strip_suffix(['e', 'E']) {
        Some(digits) if !value[end..].is_empty() => (digits, &value[end - 1..]),
        _ => (&value[..end], &value[end..]),
    };
    let number: f64 = digits.parse().ok()?;
    let factor = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        _ => return None,
    };
    let size = number * factor;
    (size.is_finite() && size > 0.0).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_from_width_and_height() {
        assert_eq!(root_size(Some("200"), Some("100px"), None), (200.0, 100.0));
        let (w, h) = root_size(Some("2in"), Some("72pt"), None);
        assert!((w - 192.0).abs() < 1e-9 && (h - 96.0).abs() < 1e-9);
    }

    #[test]
    fn size_from_view_box() {
        assert_eq!(root_size(None, None, Some("0 0 40 20")), (40.0, 20.0));
        assert_eq!(root_size(Some("100"), None, Some("0,0,40,20")), (100.0, 50.0));
        // percentages do not give a size
        assert_eq!(root_size(Some("100%"), Some("100%"), Some("0 0 40 20")), (40.0, 20.0));
        assert_eq!(root_size(None, None, None), (300.0, 150.0));
    }

    #[test]
    fn em_is_not_an_exponent() {
        assert_eq!(parse_length("2em"), None);
        assert_eq!(parse_length("1e2"), Some(100.0));
    }

    #[test]
    fn the_page_css_has_the_size() {
        assert!(page_css(Some("10"), Some("20"), None).contains("size: 10px 20px"));
    }
}
