//! Media queries (https://www.w3.org/TR/mediaqueries-4/): `@media` rules and the `media`
//! attribute of `<style>` and `<link>`.
//!
//! Helium sets documents for print, so the media type is `print` and the other types never
//! match. The queries are evaluated when the style sheet is read, against the size of the page.

use cssparser::{Delimiter, ParseError, Parser, Token};

use super::device::Device;

/// What the media queries are evaluated against.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct MediaEnv {
    /// The size of the page in CSS pixels.
    pub width: f32,
    pub height: f32,
}

impl Default for MediaEnv {
    /// An A4 page.
    fn default() -> Self {
        MediaEnv { width: 793.7, height: 1123.0 }
    }
}

impl From<Device> for MediaEnv {
    fn from(device: Device) -> Self {
        // the dummy device of a document that is cropped has no size
        if device.width.0 > 0.0 && device.height.0 > 0.0 {
            MediaEnv { width: device.width.0, height: device.height.0 }
        } else {
            MediaEnv::default()
        }
    }
}

/// The result of a test that may not be known: `(unknown-feature)` is neither true nor false and
/// counts as false at the end.
type Tri = Option<bool>;

fn not(value: Tri) -> Tri {
    value.map(|value| !value)
}

fn and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn or(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

type Result<T> = std::result::Result<T, ParseError<()>>;

/// Whether a list of media queries (the prelude of `@media`, the value of a `media` attribute)
/// matches. An empty list matches everything; a query that is not valid does not match.
pub fn matches(input: &mut Parser, env: &MediaEnv) -> bool {
    if input.is_exhausted() {
        return true;
    }
    let mut any = false;
    loop {
        let query = input.parse_until_before(Delimiter::Comma, |input| query(input, env));
        any |= matches!(query, Ok(Some(true)));
        if input.next().is_err() {
            return any;
        }
    }
}

/// Whether the media queries written in `text` match.
pub fn matches_text(text: &str, env: &MediaEnv) -> bool {
    matches(&mut Parser::new(text), env)
}

fn query<'i>(input: &mut Parser<'i>, env: &MediaEnv) -> Result<Tri> {
    let state = input.state();
    // a media condition, without a type
    if let Ok(value) = condition(input, env, true) {
        input.expect_exhausted()?;
        return Ok(value);
    }
    input.reset(&state);
    let negated = input
        .try_parse(|input| {
            let ident = input.expect_ident()?.clone();
            match_ignore_ascii_case! { &ident,
                "not" => Ok(true),
                "only" => Ok(false),
                _ => Err(ParseError::<()>::custom(())),
            }
        })
        .unwrap_or(false);
    let ident = input.expect_ident()?.clone();
    let of_type = match_ignore_ascii_case! { &ident,
        "all" | "print" => true,
        "screen" | "speech" | "tty" | "tv" | "projection" | "handheld" | "braille" | "embossed"
        | "aural" => false,
        _ => false,
    };
    let mut value = Some(of_type);
    if input.try_parse(|input| input.expect_ident_matching("and")).is_ok() {
        value = and(value, condition(input, env, false)?);
    }
    input.expect_exhausted()?;
    Ok(if negated { not(value) } else { value })
}

/// `<media-condition>`: `not (...)`, or `(...)` joined by `and`, or by `or` if `allow_or`.
fn condition<'i>(input: &mut Parser<'i>, env: &MediaEnv, allow_or: bool) -> Result<Tri> {
    if input.try_parse(|input| input.expect_ident_matching("not")).is_ok() {
        return Ok(not(in_parens(input, env)?));
    }
    let mut value = in_parens(input, env)?;
    #[derive(PartialEq, Copy, Clone)]
    enum Joiner {
        And,
        Or,
    }
    let mut joiner = None;
    loop {
        let next = if input.try_parse(|input| input.expect_ident_matching("and")).is_ok() {
            Joiner::And
        } else if allow_or && input.try_parse(|input| input.expect_ident_matching("or")).is_ok() {
            Joiner::Or
        } else {
            return Ok(value);
        };
        // `and` and `or` do not mix without parentheses
        if joiner.get_or_insert(next) != &next {
            return Err(ParseError::custom(()));
        }
        let operand = in_parens(input, env)?;
        value = if next == Joiner::And { and(value, operand) } else { or(value, operand) };
    }
}

fn in_parens<'i>(input: &mut Parser<'i>, env: &MediaEnv) -> Result<Tri> {
    let token = input.next()?.clone();
    match token {
        Token::ParenthesisBlock => input.parse_nested_block(|input| {
            // a condition in parentheses, or a feature
            let state = input.state();
            if let Ok(value) = condition(input, env, true) {
                if input.is_exhausted() {
                    return Ok(value);
                }
            }
            input.reset(&state);
            feature(input, env)
        }),
        // `selector()` and the like are not known: neither true nor false
        Token::Function(_) => {
            input.parse_nested_block(|input| {
                while input.next().is_ok() {}
                Ok::<_, ParseError<()>>(())
            })?;
            Ok(None)
        }
        _ => Err(ParseError::custom(())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Operator {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Operator {
    fn compare(self, a: f64, b: f64) -> bool {
        const EPSILON: f64 = 1e-4;
        match self {
            Operator::Lt => a < b - EPSILON,
            Operator::Le => a <= b + EPSILON,
            Operator::Gt => a > b + EPSILON,
            Operator::Ge => a >= b - EPSILON,
            Operator::Eq => (a - b).abs() <= EPSILON,
        }
    }

    fn flipped(self) -> Self {
        match self {
            Operator::Lt => Operator::Gt,
            Operator::Le => Operator::Ge,
            Operator::Gt => Operator::Lt,
            Operator::Ge => Operator::Le,
            Operator::Eq => Operator::Eq,
        }
    }
}

/// What a feature of the media can be compared with.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Ident(String),
}

/// A feature of the media as it is written in the query: a name, or a value.
#[derive(Debug, Clone, PartialEq)]
enum Operand {
    Name(String),
    Value(Value),
}

fn feature<'i>(input: &mut Parser<'i>, env: &MediaEnv) -> Result<Tri> {
    let first = operand(input)?;
    // `(name)`
    if input.is_exhausted() {
        return Ok(match first {
            Operand::Name(name) => feature_in_boolean_context(&name, env),
            Operand::Value(_) => None,
        });
    }
    // `(name: value)`, also with the prefixes `min-` and `max-`
    if input.try_parse(|input| input.expect_colon()).is_ok() {
        let Operand::Name(name) = first else {
            return Err(ParseError::custom(()));
        };
        let value = match operand(input)? {
            Operand::Value(value) => value,
            Operand::Name(ident) => Value::Ident(ident),
        };
        input.expect_exhausted()?;
        return Ok(feature_with_value(&name, &value, env));
    }
    // `(a < name)`, `(name >= b)`, `(a <= name <= b)`
    let mut operands = vec![first];
    let mut operators = vec![];
    while !input.is_exhausted() {
        operators.push(operator(input)?);
        operands.push(operand(input)?);
    }
    let value_of = |operand: &Operand| match operand {
        Operand::Value(Value::Number(number)) => Some(*number),
        _ => None,
    };
    let mut result = Some(true);
    let mut feature_name = None;
    for (index, operand) in operands.iter().enumerate() {
        if let Operand::Name(name) = operand {
            if feature_name.replace((index, name.clone())).is_some() {
                return Err(ParseError::custom(()));
            }
        }
    }
    let Some((at, name)) = feature_name else {
        return Err(ParseError::custom(()));
    };
    if operators.len() > 2 || (operators.len() == 2 && at != 1) {
        return Err(ParseError::custom(()));
    }
    let Some(actual) = numeric_feature(&name, env) else {
        return Ok(None);
    };
    for (index, operator) in operators.iter().enumerate() {
        // the name is on the side of the operator that is `at`
        let (other, operator) = if index + 1 == at {
            (&operands[index], operator.flipped())
        } else {
            (&operands[index + 1], *operator)
        };
        let Some(other) = value_of(other) else {
            return Err(ParseError::custom(()));
        };
        result = and(result, Some(operator.compare(actual, other)));
    }
    Ok(result)
}

fn operator<'i>(input: &mut Parser<'i>) -> Result<Operator> {
    let token = input.next()?.clone();
    match token {
        Token::Delim('=') => Ok(Operator::Eq),
        Token::Delim(c @ ('<' | '>')) => {
            // `<=` and `>=` are two tokens
            let or_equal = input.try_parse(|input| input.expect_delim('=')).is_ok();
            Ok(match (c, or_equal) {
                ('<', false) => Operator::Lt,
                ('<', true) => Operator::Le,
                (_, false) => Operator::Gt,
                (_, true) => Operator::Ge,
            })
        }
        _ => Err(ParseError::custom(())),
    }
}

fn operand<'i>(input: &mut Parser<'i>) -> Result<Operand> {
    let token = input.next()?.clone();
    match token {
        Token::Ident(ident) => Ok(Operand::Name(ident.to_ascii_lowercase())),
        Token::Number { value, .. } => {
            // a ratio: `16/9`
            let ratio = input.try_parse(|input| {
                input.expect_delim('/')?;
                input.expect_number()
            });
            Ok(Operand::Value(Value::Number(match ratio {
                Ok(denominator) if denominator != 0.0 => (value / denominator) as f64,
                Ok(_) => f64::INFINITY,
                Err(_) => value as f64,
            })))
        }
        Token::Dimension { value, ref unit, .. } => {
            let value = value as f64;
            let converted = match_ignore_ascii_case! { unit,
                "px" | "dppx" | "x" => value,
                "em" | "rem" => value * 16.0,
                "pt" => value / 0.75,
                "pc" => value * 16.0,
                "in" => value * 96.0,
                "cm" => value * 96.0 / 2.54,
                "mm" => value * 96.0 / 25.4,
                "q" => value * 96.0 / 101.6,
                // resolutions are in `dppx`
                "dpi" => value / 96.0,
                "dpcm" => value * 2.54 / 96.0,
                _ => return Err(ParseError::custom(())),
            };
            Ok(Operand::Value(Value::Number(converted)))
        }
        _ => Err(ParseError::custom(())),
    }
}

/// The value of a feature that is a number (`width` in pixels, `resolution` in `dppx`, ...).
fn numeric_feature(name: &str, env: &MediaEnv) -> Option<f64> {
    Some(match name {
        "width" | "device-width" => env.width as f64,
        "height" | "device-height" => env.height as f64,
        "aspect-ratio" | "device-aspect-ratio" => (env.width / env.height) as f64,
        // a page is laid out at 96 dpi
        "resolution" => 1.0,
        "color" => 8.0,
        "monochrome" | "color-index" | "grid" => 0.0,
        _ => return None,
    })
}

/// The value that a feature that is a keyword has for a document that is printed.
fn keyword_feature(name: &str, env: &MediaEnv) -> Option<&'static str> {
    Some(match name {
        "orientation" => {
            if env.height >= env.width {
                "portrait"
            } else {
                "landscape"
            }
        }
        "hover" | "any-hover" | "pointer" | "any-pointer" | "scripting" | "update" => "none",
        "overflow-block" => "paged",
        "overflow-inline" => "none",
        "prefers-color-scheme" => "light",
        "prefers-reduced-motion" | "prefers-contrast" | "prefers-reduced-transparency" => "no-preference",
        "color-gamut" => "srgb",
        "display-mode" => "browser",
        _ => return None,
    })
}

fn feature_in_boolean_context(name: &str, env: &MediaEnv) -> Tri {
    if let Some(value) = numeric_feature(name, env) {
        return Some(value != 0.0);
    }
    keyword_feature(name, env).map(|value| value != "none")
}

fn feature_with_value(name: &str, value: &Value, env: &MediaEnv) -> Tri {
    // the prefixes `min-` and `max-` compare to a number
    let (base, operator) = if let Some(base) = name.strip_prefix("min-") {
        (base, Operator::Ge)
    } else if let Some(base) = name.strip_prefix("max-") {
        (base, Operator::Le)
    } else {
        (name, Operator::Eq)
    };
    if let Some(actual) = numeric_feature(base, env) {
        return match value {
            Value::Number(number) => Some(operator.compare(actual, *number)),
            // a keyword is not a valid value of a number
            Value::Ident(_) => None,
        };
    }
    if base == name {
        if let (Some(actual), Value::Ident(wanted)) = (keyword_feature(name, env), value) {
            return Some(actual == wanted);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(query: &str) -> bool {
        matches_text(query, &MediaEnv { width: 800.0, height: 1000.0 })
    }

    #[test]
    fn the_media_type_is_print() {
        assert!(matches("print"));
        assert!(matches("all"));
        assert!(matches(""));
        assert!(matches("only print"));
        assert!(!matches("screen"));
        assert!(!matches("speech"));
        assert!(!matches("unknown"));
        assert!(!matches("not print"));
        assert!(matches("not screen"));
    }

    #[test]
    fn a_list_matches_if_any_query_does() {
        assert!(matches("screen, print"));
        assert!(!matches("screen, speech"));
        // a query that is not valid does not match, the others do
        assert!(matches("foo bar, print"));
        assert!(matches("print, foo bar"));
    }

    #[test]
    fn features_of_the_size_of_the_page() {
        assert!(matches("(min-width: 800px)"));
        assert!(matches("(min-width: 700px)"));
        assert!(!matches("(min-width: 801px)"));
        assert!(matches("(max-width: 800px)"));
        assert!(!matches("(max-width: 799px)"));
        assert!(matches("(width: 800px)"));
        assert!(matches("(min-width: 10cm) and (max-height: 1000px)"));
        assert!(matches("print and (min-height: 1000px)"));
        assert!(!matches("screen and (min-height: 1000px)"));
        assert!(matches("(min-width: 50em)"));
        assert!(!matches("(min-width: 51em)"));
    }

    #[test]
    fn ranges() {
        assert!(matches("(width >= 800px)"));
        assert!(!matches("(width > 800px)"));
        assert!(matches("(width < 900px)"));
        assert!(matches("(700px < width)"));
        assert!(matches("(700px < width < 900px)"));
        assert!(!matches("(900px < width < 1000px)"));
        assert!(matches("(700px <= width <= 800px)"));
        assert!(matches("(width = 800px)"));
        assert!(matches("(height >= 1000px) and (width < 1000px)"));
    }

    #[test]
    fn orientation_and_ratio() {
        assert!(matches("(orientation: portrait)"));
        assert!(!matches("(orientation: landscape)"));
        assert!(matches("(min-aspect-ratio: 3/4)"));
        assert!(!matches("(min-aspect-ratio: 1/1)"));
        assert!(matches("(max-aspect-ratio: 1)"));
    }

    #[test]
    fn a_printer_has_no_pointer_or_hover() {
        assert!(matches("(hover: none)"));
        assert!(matches("(pointer: none)"));
        assert!(!matches("(hover: hover)"));
        assert!(!matches("(hover)"));
        assert!(matches("(color)"));
        assert!(!matches("(monochrome)"));
        assert!(matches("(prefers-color-scheme: light)"));
        assert!(!matches("(prefers-color-scheme: dark)"));
        assert!(matches("(min-resolution: 96dpi)"));
        assert!(!matches("(min-resolution: 300dpi)"));
        assert!(matches("(min-resolution: 1dppx)"));
    }

    #[test]
    fn logic() {
        assert!(matches("not (min-width: 900px)"));
        assert!(!matches("not (min-width: 700px)"));
        assert!(matches("(min-width: 900px) or (max-width: 900px)"));
        assert!(matches("((min-width: 900px) or (max-width: 900px)) and (min-height: 10px)"));
        assert!(matches("print and (not (min-width: 900px))"));
        assert!(matches("not screen and (min-width: 10px)"));
        // `and` and `or` do not mix
        assert!(!matches("(min-width: 1px) and (min-width: 2px) or (min-width: 3px)"));
    }

    #[test]
    fn what_is_not_known_does_not_match() {
        assert!(!matches("(unknown-feature)"));
        assert!(!matches("(unknown-feature: 1)"));
        assert!(!matches("not (unknown-feature)"));
        assert!(!matches("print and (unknown-feature: 1)"));
        assert!(matches("(unknown-feature) or (min-width: 1px)"));
        assert!(!matches("(unknown-feature) or (min-width: 9999px)"));
        assert!(!matches("(min-width: foo)"));
        assert!(!matches("(min-width: 1parsec)"));
    }
}
