//! The values of the properties of generated content: `content`, `counter-reset`,
//! `counter-increment`, `counter-set` and `quotes`.
//!
//! https://www.w3.org/TR/css-content-3/ and https://www.w3.org/TR/css-lists-3/#auto-numbering

use super::super::parse::{CounterStyle, StyleParseError};
use super::specified::PropertyValue;
use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

fn invalid<T>() -> Result<T, ParseError<StyleParseError>> {
    Err(ParseError::custom(StyleParseError::InvalidValue))
}

/// One component of the value of `content`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentItem {
    String(String),
    /// `counter(name, style)`
    Counter(String, CounterStyle),
    /// `counters(name, separator, style)`
    Counters(String, String, CounterStyle),
    /// `attr(name)`: the value of an attribute of the originating element.
    Attr(String),
    OpenQuote,
    CloseQuote,
    /// Like `open-quote` and `close-quote`, but without a quotation mark.
    NoOpenQuote,
    NoCloseQuote,
}

/// The value of `content`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Content {
    /// The initial value. A pseudo-element with `normal` content has no box.
    #[default]
    Normal,
    None,
    Items(Vec<ContentItem>),
}

/// The name of a counter: a custom identifier that is not one of the keywords that CSS reserves.
fn counter_name<'i>(input: &mut Parser<'i>) -> Result<String, ParseError<StyleParseError>> {
    let ident = input.expect_ident()?.clone();
    match_ignore_ascii_case! { &ident,
        "none" | "inherit" | "initial" | "unset" | "default" => invalid(),
        _ => Ok(ident.to_string()),
    }
}

/// The style of a counter after the comma of `counter()` and `counters()`: `decimal` unless the
/// style is known (a style that is not supported would be an error in CSS; falling back to
/// decimal keeps the number at least).
pub(crate) fn counter_style<'i>(input: &mut Parser<'i>) -> Result<CounterStyle, ParseError<StyleParseError>> {
    if input.try_parse(|input| input.expect_comma()).is_err() {
        return Ok(CounterStyle::Decimal);
    }
    let style = input.expect_ident()?.clone();
    Ok(CounterStyle::from_name(&style).unwrap_or_else(|| {
        log::warn!("Unsupported counter style {:?}, using decimal instead", &*style);
        CounterStyle::Decimal
    }))
}

impl<'i> PropertyValue<'i> for Content {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|input| input.expect_ident_matching("normal")).is_ok() {
            input.expect_exhausted()?;
            return Ok(Content::Normal);
        }
        if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
            input.expect_exhausted()?;
            return Ok(Content::None);
        }
        let mut items = vec![];
        while !input.is_exhausted() {
            let token = input.next()?.clone();
            let item = match token {
                Token::QuotedString(string) => ContentItem::String(string.to_string()),
                Token::Ident(ref ident) => match_ignore_ascii_case! { ident,
                    "open-quote" => ContentItem::OpenQuote,
                    "close-quote" => ContentItem::CloseQuote,
                    "no-open-quote" => ContentItem::NoOpenQuote,
                    "no-close-quote" => ContentItem::NoCloseQuote,
                    _ => return invalid(),
                },
                Token::Function(ref name) => match_ignore_ascii_case! { name,
                    "counter" => input.parse_nested_block(|input| {
                        let name = counter_name(input)?;
                        let style = counter_style(input)?;
                        input.expect_exhausted()?;
                        Ok(ContentItem::Counter(name, style))
                    })?,
                    "counters" => input.parse_nested_block(|input| {
                        let name = counter_name(input)?;
                        input.expect_comma()?;
                        let separator = input.expect_string()?.to_string();
                        let style = counter_style(input)?;
                        input.expect_exhausted()?;
                        Ok(ContentItem::Counters(name, separator, style))
                    })?,
                    "attr" => input.parse_nested_block(|input| {
                        let name = input.expect_ident()?.to_string();
                        input.expect_exhausted()?;
                        Ok(ContentItem::Attr(name))
                    })?,
                    _ => return invalid(),
                },
                _ => return invalid(),
            };
            items.push(item);
        }
        if items.is_empty() {
            return invalid();
        }
        Ok(Content::Items(items))
    }
}

/// The counters of `counter-reset`, `counter-increment` and `counter-set`, with the integer that
/// was written (the default depends on the property: 0, 1 and 0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CounterList(pub Vec<(String, Option<i32>)>);

impl<'i> PropertyValue<'i> for CounterList {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
            input.expect_exhausted()?;
            return Ok(CounterList::default());
        }
        let mut counters = vec![];
        while !input.is_exhausted() {
            let name = counter_name(input)?;
            let value = input
                .try_parse(|input| match input.next()? {
                    Token::Number { int_value: Some(value), .. } => Ok(*value),
                    _ => invalid(),
                })
                .ok();
            counters.push((name, value));
        }
        if counters.is_empty() {
            return invalid();
        }
        Ok(CounterList(counters))
    }
}

/// The value of `quotes`: the pairs of quotation marks for the levels of nesting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Quotes {
    /// The initial value: the marks that the language of the text calls for.
    #[default]
    Auto,
    None,
    Pairs(Vec<(String, String)>),
}

impl<'i> PropertyValue<'i> for Quotes {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|input| input.expect_ident_matching("auto")).is_ok() {
            input.expect_exhausted()?;
            return Ok(Quotes::Auto);
        }
        if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
            input.expect_exhausted()?;
            return Ok(Quotes::None);
        }
        let mut pairs = vec![];
        while !input.is_exhausted() {
            let open = input.expect_string()?.to_string();
            let close = input.expect_string()?.to_string();
            pairs.push((open, close));
        }
        if pairs.is_empty() {
            return invalid();
        }
        Ok(Quotes::Pairs(pairs))
    }
}

impl Quotes {
    /// The marks for the given depth of nesting (0 is the outermost) of text in the language
    /// `lang` (a BCP 47 tag). The innermost pair repeats for deeper levels.
    pub fn marks(&self, depth: usize, lang: &str) -> Option<(String, String)> {
        match self {
            Quotes::None => None,
            Quotes::Pairs(pairs) => pairs.get(depth).or(pairs.last()).cloned(),
            Quotes::Auto => {
                let primary = lang.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
                // the outer and the inner pair of the language
                let ((open, close), (inner_open, inner_close)) = match primary.as_str() {
                    "de" | "cs" | "sk" | "sl" | "hr" | "et" | "lv" | "is" => {
                        (("\u{201e}", "\u{201c}"), ("\u{201a}", "\u{2018}"))
                    }
                    "fr" | "es" | "ca" | "pt" | "it" | "ru" | "uk" | "el" | "hy" | "bg" => {
                        (("\u{ab}", "\u{bb}"), ("\u{201c}", "\u{201d}"))
                    }
                    "sv" | "fi" => (("\u{201d}", "\u{201d}"), ("\u{2019}", "\u{2019}")),
                    "pl" | "nl" | "ro" | "hu" => (("\u{201e}", "\u{201d}"), ("\u{ab}", "\u{bb}")),
                    "ja" | "zh" => (("\u{300c}", "\u{300d}"), ("\u{300e}", "\u{300f}")),
                    _ => (("\u{201c}", "\u{201d}"), ("\u{2018}", "\u{2019}")),
                };
                let (open, close) = if depth % 2 == 0 { (open, close) } else { (inner_open, inner_close) };
                Some((open.to_owned(), close.to_owned()))
            }
        }
    }
}
