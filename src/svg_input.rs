//! A standalone SVG file as the input of the converter.
//!
//! The file is read like an HTML document: the HTML parser puts an `<svg>` element in the SVG
//! namespace, its `<foreignObject>` holds HTML, and `<math>` is MathML. All that is added here is
//! a shell that makes the drawing the whole page: one page of the size of the root element, with no
//! margin, and the `<svg>` as a block so that no line box surrounds it.
//!
//! Limits of reading XML as HTML: an element that is written as empty (`<div/>`, `<span/>`) inside
//! of a `foreignObject` is an open tag for the HTML parser; prefixed names (`<svg:svg>`) and entities
//! of an internal DTD are not understood.

/// The size of a page of a CSS pixel (1/96 inch) when the root element does not say.
const DEFAULT_WIDTH: f64 = 300.0;
const DEFAULT_HEIGHT: f64 = 150.0;

/// Whether the text is an SVG document: the first element is `<svg`.
pub fn looks_like_svg(text: &str) -> bool {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            match after.find("?>") {
                Some(end) => rest = &after[end + 2..],
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix("<!--") {
            match after.find("-->") {
                Some(end) => rest = &after[end + 3..],
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix("<!") {
            // the document type declaration (with an internal subset between brackets)
            let mut depth = 0usize;
            let mut end = None;
            for (i, c) in after.char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth = depth.saturating_sub(1),
                    '>' if depth == 0 => {
                        end = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            match end {
                Some(end) => rest = &after[end + 1..],
                None => return false,
            }
        } else {
            break;
        }
    }
    rest.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("<svg"))
        && rest[4..].chars().next().is_none_or(|c| c.is_whitespace() || c == '>' || c == '/')
}

/// The HTML document that draws the SVG text as one page.
pub fn wrap(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}');
    let (width, height) = root_size(text);
    format!(
        "<!DOCTYPE html><meta charset=\"utf-8\"><style>\
         @page {{ size: {width}px {height}px; margin: 0 }} \
         html, body {{ margin: 0; padding: 0 }} \
         body > svg {{ display: block }}\
         </style>{body}",
        width = number(width),
        height = number(height),
        body = expand_empty_elements(strip_prolog(text)),
    )
}

/// The HTML elements that have no end tag, which may be written `<br/>` as they are.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Writes every element that is empty in XML (`<div/>`) with an end tag (`<div></div>`).
///
/// The HTML parser ends only the elements of SVG and MathML at `/>`; `<div/>` and `<span/>` are
/// start tags to it and would hold everything that follows. For the elements of SVG and MathML
/// the two forms are the same, so all of them are expanded, except the void HTML elements.
/// Comments, `CDATA` sections, processing instructions, quoted attribute values and the content
/// of `<style>` and `<script>` are copied as they are.
fn expand_empty_elements(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 64);
    let mut i = 0;
    // copies up to and including `end` (or the rest of the text) and returns the new position
    let skip = |out: &mut String, from: usize, end: &str| {
        let to = text[from..].find(end).map_or(text.len(), |at| from + at + end.len());
        out.push_str(&text[from..to]);
        to
    };
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = text[i..].find('<').map_or(text.len(), |at| i + at);
            out.push_str(&text[i..next]);
            i = next;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i = skip(&mut out, i, "-->");
        } else if rest.starts_with("<![CDATA[") {
            i = skip(&mut out, i, "]]>");
        } else if rest.starts_with("<?") {
            i = skip(&mut out, i, "?>");
        } else if rest.starts_with("<!") || rest.starts_with("</") {
            i = skip(&mut out, i, ">");
        } else if rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            // a start tag: find its end, outside of quotes
            let mut end = None;
            let mut quote = None;
            for (at, c) in rest.char_indices().skip(1) {
                match (quote, c) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, '"' | '\'') => quote = Some(c),
                    (None, '>') => {
                        end = Some(at);
                        break;
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                out.push_str(rest);
                break;
            };
            let tag = &rest[..=end];
            let name: String = tag[1..]
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
                .collect();
            let lower = name.to_ascii_lowercase();
            if let Some(open) = tag.strip_suffix("/>") {
                if VOID.contains(&lower.as_str()) {
                    out.push_str(tag);
                } else {
                    out.push_str(open);
                    out.push_str("></");
                    out.push_str(&name);
                    out.push('>');
                }
                i += tag.len();
            } else {
                out.push_str(tag);
                i += tag.len();
                if lower == "style" || lower == "script" {
                    let close = format!("</{lower}");
                    let content = &text[i..];
                    let at = content
                        .to_ascii_lowercase()
                        .find(&close)
                        .unwrap_or(content.len());
                    out.push_str(&content[..at]);
                    i += at;
                }
            }
        } else {
            // a `<` that starts no tag
            out.push('<');
            i += 1;
        }
    }
    out
}

/// The text from the root element on: the XML declaration and the document type are left out
/// (the HTML parser would take the first for a comment and be confused by the second).
fn strip_prolog(text: &str) -> &str {
    let mut rest = text;
    loop {
        let trimmed = rest.trim_start();
        let after = if let Some(after) = trimmed.strip_prefix("<?") {
            after.find("?>").map(|end| &after[end + 2..])
        } else if trimmed.starts_with("<!") && !trimmed.starts_with("<!--") {
            let mut depth = 0usize;
            let mut end = None;
            for (i, c) in trimmed[2..].char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => depth = depth.saturating_sub(1),
                    '>' if depth == 0 => {
                        end = Some(i + 3);
                        break;
                    }
                    _ => {}
                }
            }
            end.map(|end| &trimmed[end..])
        } else {
            None
        };
        match after {
            Some(after) => rest = after,
            None => return trimmed,
        }
    }
}

fn number(value: f64) -> String {
    let text = format!("{:.4}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The size of the root element in CSS pixels: `width` and `height`, else the `viewBox`, else the
/// size that browsers use for an SVG that says nothing.
fn root_size(text: &str) -> (f64, f64) {
    let attributes = root_attributes(text);
    let get = |name: &str| attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    let view_box = get("viewBox").and_then(|value| {
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
    let length = |name: &str| get(name).and_then(parse_length);
    let (width, height) = (length("width"), length("height"));
    match (width, height, view_box) {
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

/// The attributes of the start tag of the root element, as (name, value).
fn root_attributes(text: &str) -> Vec<(String, String)> {
    let rest = strip_prolog(text);
    let Some(tag) = rest.strip_prefix('<') else {
        return vec![];
    };
    let mut attributes = vec![];
    let mut chars = tag.char_indices().peekable();
    // the element name
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() || c == '>' || c == '/' {
            break;
        }
        chars.next();
    }
    loop {
        while chars.next_if(|&(_, c)| c.is_whitespace() || c == '/').is_some() {}
        let Some(&(start, c)) = chars.peek() else { break };
        if c == '>' {
            break;
        }
        let mut end = start;
        while let Some(&(i, c)) = chars.peek() {
            if c.is_whitespace() || c == '=' || c == '>' || c == '/' {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        let name = tag[start..end].to_owned();
        if name.is_empty() {
            chars.next();
            continue;
        }
        while chars.next_if(|&(_, c)| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if(|&(_, c)| c == '=').is_some() {
            while chars.next_if(|&(_, c)| c.is_whitespace()).is_some() {}
            match chars.peek().copied() {
                Some((_, quote @ ('"' | '\''))) => {
                    chars.next();
                    for (_, c) in chars.by_ref() {
                        if c == quote {
                            break;
                        }
                        value.push(c);
                    }
                }
                _ => {
                    while let Some(&(_, c)) = chars.peek() {
                        if c.is_whitespace() || c == '>' {
                            break;
                        }
                        value.push(c);
                        chars.next();
                    }
                }
            }
        }
        attributes.push((name, value));
    }
    attributes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_svg_documents() {
        assert!(looks_like_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(looks_like_svg("\u{feff}<?xml version=\"1.0\"?>\n<!-- c -->\n<!DOCTYPE svg PUBLIC \"x\" \"y\" [<!ENTITY a \"b\">]>\n<svg>"));
        assert!(!looks_like_svg("<!DOCTYPE html><html><body><svg></svg></body></html>"));
        assert!(!looks_like_svg("<svgx>"));
        assert!(!looks_like_svg("plain text"));
    }

    #[test]
    fn size_from_width_and_height() {
        assert_eq!(root_size("<svg width=\"200\" height='100px'>"), (200.0, 100.0));
        let (w, h) = root_size("<svg width=\"2in\" height=\"72pt\">");
        assert!((w - 192.0).abs() < 1e-9 && (h - 96.0).abs() < 1e-9);
    }

    #[test]
    fn size_from_view_box() {
        assert_eq!(root_size("<svg viewBox=\"0 0 40 20\">"), (40.0, 20.0));
        assert_eq!(root_size("<svg viewBox=\"0,0,40,20\" width=\"100\">"), (100.0, 50.0));
        // percentages do not give a size
        assert_eq!(root_size("<svg viewBox=\"0 0 40 20\" width=\"100%\" height=\"100%\">"), (40.0, 20.0));
        assert_eq!(root_size("<svg>"), (300.0, 150.0));
    }

    #[test]
    fn em_is_not_an_exponent() {
        assert_eq!(parse_length("2em"), None);
        assert_eq!(parse_length("1e2"), Some(100.0));
    }

    #[test]
    fn empty_elements_get_end_tags() {
        assert_eq!(expand_empty_elements("<div/><p a=\"x/>y\" />"), "<div></div><p a=\"x/>y\" ></p>");
        assert_eq!(expand_empty_elements("<rect x='1'/>"), "<rect x='1'></rect>");
        // void elements stay
        assert_eq!(expand_empty_elements("<br/><img src=\"a\"/>"), "<br/><img src=\"a\"/>");
        // what is not markup stays
        let verbatim = "<!-- <b/> --><![CDATA[ <i/> ]]><style>a>b{} <u/></style><?pi <s/> ?>";
        assert_eq!(expand_empty_elements(verbatim), verbatim);
        assert_eq!(expand_empty_elements("a < b <i/>"), "a < b <i></i>");
        // the name keeps its case for the end tag
        assert_eq!(expand_empty_elements("<linearGradient/>"), "<linearGradient></linearGradient>");
    }

    #[test]
    fn wrap_leaves_out_the_prolog_and_sets_the_page() {
        let html = wrap("<?xml version=\"1.0\"?>\n<!DOCTYPE svg>\n<svg width=\"10\" height=\"20\"></svg>");
        assert!(html.contains("size: 10px 20px"), "{html}");
        assert!(html.ends_with("<svg width=\"10\" height=\"20\"></svg>"), "{html}");
        assert!(!html.contains("<?xml"), "{html}");
    }
}
