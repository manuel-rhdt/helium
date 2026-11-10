//! A linear text form of MathML for the `/Alt` of `Formula` structure elements.
//!
//! The text content of a formula loses its structure: `<msqrt><mi>π</mi></msqrt>` reads as
//! `π` and `<msup><mi>e</mi><mn>2</mn></msup>` as `e2`. The linear form keeps it, in the
//! notation of calculators and computer algebra: `sqrt(π)`, `e^2`, `x_i`, `(a + b)/(c)` with
//! parentheses around every operand that is more than one token, so that the result is
//! unambiguous when it is read aloud.

use crate::parser::{ElementData, ParsedData};
use crate::tree::{DomRef, TreeLinked};

use super::{attribute, collapse_whitespace, text_content};

/// The linear form of a `<math>` element (or any MathML element).
pub fn linearize(node: &DomRef) -> String {
    let linear = Linear::of(node);
    collapse_whitespace(&linear.text)
}

/// A piece of the linear form.
#[derive(Debug, Clone)]
struct Linear {
    text: String,
    /// The piece is a single token or is enclosed in fences, so it needs no parentheses as an
    /// operand.
    atomic: bool,
    /// The piece is a base with scripts: `^` and `_` bind tighter than `/`, so it needs no
    /// parentheses as the numerator or denominator of a fraction.
    scripted: bool,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A token or an expression.
    Operand,
    /// An infix or prefix operator that is set off by spaces (`=`, `+`, `∈`).
    Spaced,
    /// An operator that is written without spaces (`/`, `!`, `'`).
    Tight,
    OpenFence,
    CloseFence,
    /// `,` and `;`, followed by a space.
    Separator,
    Fraction,
    /// Nothing: invisible operators, `mphantom`, annotations.
    Empty,
}

impl Linear {
    fn new(text: impl Into<String>, atomic: bool, kind: Kind) -> Self {
        Linear {
            text: text.into(),
            atomic,
            scripted: false,
            kind,
        }
    }

    fn empty() -> Self {
        Linear::new("", true, Kind::Empty)
    }

    fn operand(text: impl Into<String>, atomic: bool) -> Self {
        Linear::new(text, atomic, Kind::Operand)
    }

    /// The text as an operand: in parentheses unless it is atomic.
    fn grouped(&self) -> String {
        if self.atomic || self.text.is_empty() {
            self.text.clone()
        } else {
            format!("({})", self.text)
        }
    }

    /// The text as the numerator or denominator of a fraction.
    fn fraction_part(&self) -> String {
        if self.scripted {
            self.text.clone()
        } else {
            self.grouped()
        }
    }

    fn of(node: &DomRef) -> Linear {
        match &node.html_data {
            ParsedData::Text(text) => {
                let text = collapse_whitespace(&text.borrow());
                if text.is_empty() {
                    Linear::empty()
                } else {
                    Linear::operand(text, true)
                }
            }
            ParsedData::Element(element) => Linear::of_element(node, element),
            _ => Linear::empty(),
        }
    }

    fn of_element(node: &DomRef, element: &ElementData) -> Linear {
        let children: Vec<DomRef> = TreeLinked::children(node)
            .filter(|child| matches!(child.html_data, ParsedData::Element(_)))
            .collect();
        let child = |index: usize| {
            children
                .get(index)
                .map_or_else(Linear::empty, Linear::of)
        };
        match &*element.name.local {
            "mi" | "mn" | "mtext" | "ms" => {
                let text = token_text(node);
                if text.is_empty() {
                    Linear::empty()
                } else {
                    Linear::operand(text, true)
                }
            }
            "mo" => operator(&token_text(node)),
            "mspace" => Linear::empty(),
            "mphantom" | "annotation" | "annotation-xml" | "none" => Linear::empty(),
            "semantics" | "maction" => child(0),
            "msqrt" => Linear::operand(format!("sqrt({})", row(&children).text), true),
            "mroot" => Linear::operand(
                format!("root({}, {})", child(1).text, child(0).text),
                true,
            ),
            "mfrac" => Linear::new(
                format!("{}/{}", child(0).fraction_part(), child(1).fraction_part()),
                false,
                Kind::Fraction,
            ),
            "msup" | "mover" => script(&child(0), &[("^", child(1))]),
            "msub" | "munder" => script(&child(0), &[("_", child(1))]),
            "msubsup" | "munderover" => {
                script(&child(0), &[("_", child(1)), ("^", child(2))])
            }
            "mmultiscripts" => multiscripts(&children),
            "mtable" => {
                let rows: Vec<String> = children.iter().map(|row| Linear::of(row).text).collect();
                Linear::operand(format!("[{}]", rows.join("; ")), true)
            }
            "mtr" | "mlabeledtr" => {
                let cells: Vec<String> = children.iter().map(|cell| Linear::of(cell).text).collect();
                Linear::operand(cells.join(", "), false)
            }
            "mfenced" => {
                let open = attribute(element, "open").unwrap_or_else(|| "(".to_owned());
                let close = attribute(element, "close").unwrap_or_else(|| ")".to_owned());
                let separators: Vec<char> = attribute(element, "separators")
                    .unwrap_or_else(|| ",".to_owned())
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                let mut text = open;
                for (index, child) in children.iter().enumerate() {
                    if index > 0 {
                        let separator = separators
                            .get(index - 1)
                            .or(separators.last())
                            .copied()
                            .unwrap_or(',');
                        text.push(separator);
                        text.push(' ');
                    }
                    text.push_str(&Linear::of(child).text);
                }
                text.push_str(&close);
                Linear::operand(text, true)
            }
            // `math`, `mrow`, `mstyle`, `mpadded`, `menclose`, `merror`, `mtd` and anything
            // unknown: a row of its children
            _ => row(&children),
        }
    }
}

/// The text of a token element.
fn token_text(node: &DomRef) -> String {
    let mut text = String::new();
    text_content(node, &mut text);
    text.trim().to_owned()
}

fn operator(text: &str) -> Linear {
    let kind = match text {
        "" => return Linear::empty(),
        // function application and invisible times: the neighbours are spaced like operands
        "\u{2061}" | "\u{2062}" => return Linear::empty(),
        "\u{2063}" => return Linear::new(",", true, Kind::Separator),
        "\u{2064}" => return Linear::new("+", true, Kind::Spaced),
        "(" | "[" | "{" | "⟨" | "⌈" | "⌊" | "‖" | "|" | "⟦" => Kind::OpenFence,
        ")" | "]" | "}" | "⟩" | "⌉" | "⌋" | "⟧" => Kind::CloseFence,
        "," | ";" => Kind::Separator,
        "/" | "!" | "'" | "′" | "″" | "." | ":" | "^" | "_" => Kind::Tight,
        _ if is_large_operator(text) => Kind::Operand,
        _ if text.chars().all(|c| !c.is_alphanumeric()) => Kind::Spaced,
        // an operator written with letters (`lim`, `mod`)
        _ => Kind::Operand,
    };
    Linear::new(text, true, kind)
}

fn is_large_operator(text: &str) -> bool {
    matches!(
        text,
        "∫" | "∬" | "∭" | "∮" | "∑" | "∏" | "∐" | "⋃" | "⋂" | "⋁" | "⋀" | "⨁" | "⨂"
    )
}

/// A base with scripts, like `x_i^2`.
fn script(base: &Linear, scripts: &[(&str, Linear)]) -> Linear {
    let mut text = base.grouped();
    for (mark, script) in scripts {
        if script.text.is_empty() {
            continue;
        }
        text.push_str(mark);
        text.push_str(&script.grouped());
    }
    Linear {
        scripted: true,
        ..Linear::operand(text, false)
    }
}

/// `<mmultiscripts>`: the base, then pairs of subscripts and superscripts, then
/// `<mprescripts/>` and pairs of prescripts.
fn multiscripts(children: &[DomRef]) -> Linear {
    let is_prescripts = |node: &DomRef| {
        matches!(&node.html_data, ParsedData::Element(e) if &*e.name.local == "mprescripts")
    };
    let Some((base, rest)) = children.split_first() else {
        return Linear::empty();
    };
    let split = rest.iter().position(is_prescripts).unwrap_or(rest.len());
    let pairs = |nodes: &[DomRef]| -> Vec<(&'static str, Linear)> {
        nodes
            .chunks(2)
            .flat_map(|pair| {
                let sub = pair.first().map_or_else(Linear::empty, Linear::of);
                let sup = pair.get(1).map_or_else(Linear::empty, Linear::of);
                [("_", sub), ("^", sup)]
            })
            .collect()
    };
    let post = pairs(&rest[..split]);
    let pre = pairs(rest.get(split + 1..).unwrap_or(&[]));
    let mut text = String::new();
    for (mark, script) in &pre {
        if !script.text.is_empty() {
            text.push_str(mark);
            text.push_str(&script.grouped());
        }
    }
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(&script(&Linear::of(base), &post).text);
    Linear::operand(text, false)
}

/// A row of elements, joined with the spacing of their kinds.
fn row(children: &[DomRef]) -> Linear {
    let mut items: Vec<Linear> = children
        .iter()
        .map(Linear::of)
        .filter(|item| item.kind != Kind::Empty)
        .collect();
    if items.len() == 1 {
        return items.pop().unwrap();
    }
    // A fraction next to an operand is put in parentheses: `(1/2) x`, not `1/2 x`.
    for index in 0..items.len() {
        if items[index].kind == Kind::Fraction {
            let operand_next = |item: Option<&Linear>| {
                item.is_some_and(|item| matches!(item.kind, Kind::Operand | Kind::Fraction))
            };
            let before = index.checked_sub(1).and_then(|i| items.get(i));
            if operand_next(before) || operand_next(items.get(index + 1)) {
                items[index].text = format!("({})", items[index].text);
                items[index].atomic = true;
            }
            items[index].kind = Kind::Operand;
        }
    }

    let mut text = String::new();
    let mut previous: Option<Kind> = None;
    for item in &items {
        let space = match (previous, item.kind) {
            (None, _) => false,
            (Some(Kind::OpenFence | Kind::Tight), _) => false,
            (_, Kind::CloseFence | Kind::Separator | Kind::Tight) => false,
            // a prefix operator sticks to its operand: `−x`, `(−x)`, `= −x`
            (Some(Kind::Spaced), _) => !is_prefix(&text),
            (Some(Kind::Separator), _) => true,
            (_, Kind::Spaced) => true,
            (Some(Kind::Operand | Kind::Fraction | Kind::CloseFence), Kind::Operand) => {
                needs_space(&text, &item.text)
            }
            (Some(_), Kind::OpenFence) => false,
            _ => false,
        };
        if space {
            text.push(' ');
        }
        text.push_str(&item.text);
        previous = Some(item.kind);
    }
    let atomic = is_fenced(&items);
    Linear::operand(text, atomic)
}

/// Whether the operator at the end of `text` is a prefix operator: one at the start, after a
/// fence, a separator or another operator.
fn is_prefix(text: &str) -> bool {
    let mut chars = text.trim_end().chars();
    chars.next_back();
    let before = chars.as_str().trim_end();
    match before.chars().last() {
        None => true,
        Some(c) => "([{⟨,;=<>≤≥≠≈∈∉⊂⊆→⇒⇔+−-×·±∓∗".contains(c),
    }
}

/// Whether two adjacent operands are separated by a space: `d x`, `σ sqrt(2π)`, but `2π`
/// and `f(x)`.
fn needs_space(before: &str, after: &str) -> bool {
    let last = before.chars().last();
    let first = after.chars().next();
    match (last, first) {
        (Some(_), Some('(' | '[')) => false,
        (Some(last), Some(first)) => !(last.is_ascii_digit() && !first.is_ascii_digit()),
        _ => false,
    }
}

/// Whether the row is enclosed in a pair of matching fences, like `(x − μ)`.
fn is_fenced(items: &[Linear]) -> bool {
    if items.len() < 2
        || items[0].kind != Kind::OpenFence
        || items[items.len() - 1].kind != Kind::CloseFence
    {
        return false;
    }
    let mut depth = 0;
    for (index, item) in items.iter().enumerate() {
        match item.kind {
            Kind::OpenFence => depth += 1,
            Kind::CloseFence => {
                depth -= 1;
                if depth == 0 && index != items.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(mathml: &str) -> String {
        let dom = crate::parser::generate_render_tree(&format!("<p>{mathml}</p>"));
        fn find_math(node: &DomRef) -> Option<DomRef> {
            if let ParsedData::Element(element) = &node.html_data {
                if &*element.name.local == "math" {
                    return Some(node.clone());
                }
            }
            TreeLinked::children(node).find_map(|child| find_math(&child))
        }
        linearize(&find_math(&dom).expect("no <math>"))
    }

    #[test]
    fn radicals_are_kept() {
        assert_eq!(
            linear("<math><mi>x</mi><mo>=</mo><msqrt><mi>π</mi></msqrt></math>"),
            "x = sqrt(π)"
        );
        assert_eq!(
            linear("<math><mi>σ</mi><msqrt><mrow><mn>2</mn><mi>π</mi></mrow></msqrt></math>"),
            "σ sqrt(2π)"
        );
        assert_eq!(
            linear("<math><msqrt><mi>det</mi><mo>&#x2061;</mo><mi>A</mi></msqrt></math>"),
            "sqrt(det A)"
        );
        assert_eq!(
            linear("<math><mroot><mi>x</mi><mn>3</mn></mroot></math>"),
            "root(3, x)"
        );
    }

    #[test]
    fn scripts_are_marked() {
        assert_eq!(
            linear("<math><msup><mi>e</mi><mrow><mo>−</mo><msup><mi>x</mi><mn>2</mn></msup></mrow></msup></math>"),
            "e^(−x^2)"
        );
        assert_eq!(
            linear("<math><msup><mi>ℝ</mi><mrow><mi>n</mi><mo>×</mo><mi>n</mi></mrow></msup></math>"),
            "ℝ^(n × n)"
        );
        assert_eq!(
            linear("<math><msubsup><mo>∫</mo><mn>0</mn><mrow><mn>2</mn><mi>π</mi></mrow></msubsup><mi>f</mi><mi>d</mi><mi>θ</mi></math>"),
            "∫_0^(2π) f d θ"
        );
        assert_eq!(
            linear("<math><msubsup><mo>∫</mo><mrow><mo>−</mo><mi>∞</mi></mrow><mi>∞</mi></msubsup><msup><mi>e</mi><mrow><mo>−</mo><msup><mi>x</mi><mn>2</mn></msup></mrow></msup><mi>d</mi><mi>x</mi></math>"),
            "∫_(−∞)^∞ e^(−x^2) d x"
        );
        assert_eq!(
            linear("<math><msup><mrow><mo>(</mo><mi>x</mi><mo>−</mo><mi>μ</mi><mo>)</mo></mrow><mn>2</mn></msup></math>"),
            "(x − μ)^2"
        );
    }

    #[test]
    fn fractions_are_delimited() {
        assert_eq!(
            linear("<math><mi>φ</mi><mrow><mo>(</mo><mi>x</mi><mo>)</mo></mrow><mo>=</mo><mfrac><mn>1</mn><mrow><mi>σ</mi><msqrt><mrow><mn>2</mn><mi>π</mi></mrow></msqrt></mrow></mfrac><msup><mi>e</mi><mi>y</mi></msup></math>"),
            "φ(x) = (1/(σ sqrt(2π))) e^y"
        );
        assert_eq!(
            linear("<math><mfrac><msup><mrow><mo>(</mo><mi>x</mi><mo>−</mo><mi>μ</mi><mo>)</mo></mrow><mn>2</mn></msup><mrow><mn>2</mn><msup><mi>σ</mi><mn>2</mn></msup></mrow></mfrac></math>"),
            "(x − μ)^2/(2σ^2)"
        );
        assert_eq!(
            linear("<math><mfrac><msup><mi>π</mi><mrow><mi>n</mi><mo>/</mo><mn>2</mn></mrow></msup><msqrt><mrow><mi>det</mi><mo>&#x2061;</mo><mi>A</mi></mrow></msqrt></mfrac></math>"),
            "π^(n/2)/sqrt(det A)"
        );
        assert_eq!(
            linear("<math><msup><mi>e</mi><mrow><mo>−</mo><mfrac><mi>a</mi><mi>b</mi></mfrac></mrow></msup></math>"),
            "e^(−a/b)"
        );
    }

    #[test]
    fn rows_and_fences() {
        assert_eq!(
            linear("<math><mrow><mo>(</mo><mi>r</mi><mo>,</mo><mi>θ</mi><mo>)</mo></mrow></math>"),
            "(r, θ)"
        );
        assert_eq!(
            linear("<math><mi>A</mi><mo>∈</mo><msub><mi>x</mi><mi>i</mi></msub></math>"),
            "A ∈ x_i"
        );
        assert_eq!(linear("<math><mi>n</mi><mo>=</mo><mn>1</mn></math>"), "n = 1");
        assert_eq!(linear("<math><mo>∎</mo></math>"), "∎");
        assert_eq!(
            linear("<math><mi>a</mi><mo>=</mo><mo>−</mo><mi>b</mi><mo>+</mo><mi>c</mi></math>"),
            "a = −b + c"
        );
    }
}
