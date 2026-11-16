//! The `@font-face` rule (CSS Fonts 4, § 4): the descriptors that Helium reads.
//!
//! Supported descriptors: `font-family`, `src` (`local()` and `url()` sources, with `format()`
//! and `tech()`), `font-weight` (also a range) and `font-style`. `font-stretch`,
//! `unicode-range`, `font-display`, `font-feature-settings`, `font-variation-settings` and the
//! metric overrides are accepted and ignored, like the unknown descriptors.

use super::parse::StyleParseError;
use super::properties::specified::{FontFamily, FontStyle, PropertyValue};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser,
    ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, Token,
};
use log::warn;
use std::path::{Path, PathBuf};

/// A place that the font of a `@font-face` rule can come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontSource {
    /// `local("Name")`: an installed font, by its PostScript or family name.
    Local(String),
    /// `url(...)`: the text of the URL, a file next to the document or a `data:` URI.
    Url(String),
}

/// A `@font-face` rule: a named face of a font family that the document defines.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFaceRule {
    /// The family name that the rule defines (`font-family: "Name"` in the rule).
    pub family: String,
    /// The sources in order of preference.
    pub sources: Vec<FontSource>,
    /// The range of weights that the face is for (equal ends for one weight).
    pub weight: (f32, f32),
    pub style: FontStyle,
    /// The directory that relative URLs of the `src` refer to: the one of the style sheet.
    pub base_dir: PathBuf,
}

enum Descriptor {
    Family(String),
    Src(Vec<FontSource>),
    Weight(f32, f32),
    Style(FontStyle),
    Ignored,
}

/// Parses the block of a `@font-face` rule. `None` if the rule is invalid: a rule needs a
/// `font-family` and a `src`.
pub fn parse_font_face(input: &mut Parser, base_dir: &Path) -> Option<FontFaceRule> {
    let mut family = None;
    let mut sources = None;
    let mut weight = (400.0, 400.0);
    let mut style = FontStyle::Normal;
    for item in RuleBodyParser::new(input, &mut DescriptorParser) {
        match item {
            Ok(Descriptor::Family(name)) => family = Some(name),
            Ok(Descriptor::Src(list)) => sources = Some(list),
            Ok(Descriptor::Weight(lo, hi)) => weight = (lo, hi),
            Ok(Descriptor::Style(parsed)) => style = parsed,
            Ok(Descriptor::Ignored) => {}
            Err((error, text, _location)) => {
                warn!("Could not parse @font-face descriptor {:?}: {:?}", text, error)
            }
        }
    }
    match (family, sources) {
        (Some(family), Some(sources)) => Some(FontFaceRule {
            family,
            sources,
            weight,
            style,
            base_dir: base_dir.to_path_buf(),
        }),
        _ => {
            warn!("Ignoring a @font-face rule without a valid font-family and src");
            None
        }
    }
}

fn invalid() -> ParseError<StyleParseError> {
    ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue) }
}

struct DescriptorParser;

impl<'i> DeclarationParser<'i> for DescriptorParser {
    type Declaration = Descriptor;
    type Error = StyleParseError;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<Descriptor, ParseError<StyleParseError>> {
        let descriptor = match_ignore_ascii_case! { &name,
            "font-family" => match FontFamily::from_css_property_value(input)? {
                FontFamily::Named(name) => Descriptor::Family(name),
                // a generic family cannot be defined
                _ => return Err(invalid()),
            },
            "src" => Descriptor::Src(parse_src(input)),
            "font-weight" => {
                let (lo, hi) = parse_weight(input)?;
                Descriptor::Weight(lo, hi)
            },
            "font-style" => Descriptor::Style(parse_style(input)?),
            _ => {
                // accepted and ignored
                while input.next().is_ok() {}
                Descriptor::Ignored
            },
        };
        input.expect_exhausted()?;
        Ok(descriptor)
    }
}

impl<'i> AtRuleParser<'i> for DescriptorParser {
    type Prelude = ();
    type AtRule = Descriptor;
    type Error = StyleParseError;
}

impl<'i> QualifiedRuleParser<'i> for DescriptorParser {
    type Prelude = ();
    type QualifiedRule = Descriptor;
    type Error = StyleParseError;
}

impl<'i> RuleBodyItemParser<'i, Descriptor, StyleParseError> for DescriptorParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `normal | bold | <number [1,1000]>{1,2}`: the weights of the face as a range.
fn parse_weight<'i>(input: &mut Parser<'i>) -> Result<(f32, f32), ParseError<StyleParseError>> {
    if let Ok(ident) = input.try_parse(|input| input.expect_ident().cloned()) {
        return match_ignore_ascii_case! { &ident,
            "normal" => Ok((400.0, 400.0)),
            "bold" => Ok((700.0, 700.0)),
            _ => Err(invalid()),
        };
    }
    let number = |input: &mut Parser<'i>| -> Result<f32, ParseError<StyleParseError>> {
        let value = input.expect_number()?;
        if (1.0..=1000.0).contains(&value) { Ok(value) } else { Err(invalid()) }
    };
    let first = number(input)?;
    let second = input.try_parse(number).unwrap_or(first);
    Ok((first.min(second), first.max(second)))
}

/// `normal | italic | oblique <angle>{0,2}`
fn parse_style<'i>(input: &mut Parser<'i>) -> Result<FontStyle, ParseError<StyleParseError>> {
    let ident = input.expect_ident()?.clone();
    let style = match_ignore_ascii_case! { &ident,
        "normal" => FontStyle::Normal,
        "italic" => FontStyle::Italic,
        "oblique" => {
            // the angles do not matter: an oblique face is one face
            while input.next().is_ok() {}
            FontStyle::Oblique
        },
        _ => return Err(invalid()),
    };
    Ok(style)
}

/// The comma-separated list of sources. A source that is not valid or that is of a format that
/// Helium cannot read is left out (CSS Fonts 4 § 4.3: the others are still used).
fn parse_src(input: &mut Parser) -> Vec<FontSource> {
    let mut sources = vec![];
    loop {
        match input.parse_until_before(cssparser::Delimiter::Comma, parse_source) {
            Ok(Some(source)) => sources.push(source),
            Ok(None) => {}
            Err(error) => warn!("Skipping an invalid font source: {:?}", error),
        }
        if input.next().is_err() {
            break;
        }
    }
    sources
}

/// One source: `url(...) [format(...)]? [tech(...)]?` or `local(...)`. `None` for a source that
/// is valid but not usable.
fn parse_source<'i>(
    input: &mut Parser<'i>,
) -> Result<Option<FontSource>, ParseError<StyleParseError>> {
    let token = input.next()?.clone();
    let source = match token {
        Token::UnquotedUrl(url) => FontSource::Url(url.to_string()),
        Token::Function(function) if function.eq_ignore_ascii_case("url") => {
            FontSource::Url(input.parse_nested_block(|input| Ok(input.expect_string()?.to_string()))?)
        }
        Token::Function(function) if function.eq_ignore_ascii_case("local") => {
            let name = input.parse_nested_block(|input| {
                if let Ok(string) = input.try_parse(|input| input.expect_string().cloned()) {
                    return Ok(string.to_string());
                }
                // unquoted: identifiers joined by spaces
                let mut name = input.expect_ident()?.to_string();
                while let Ok(next) = input.try_parse(|input| input.expect_ident().cloned()) {
                    name.push(' ');
                    name.push_str(&next);
                }
                Ok(name)
            })?;
            FontSource::Local(name)
        }
        _ => return Err(invalid()),
    };

    let mut usable = true;
    while !input.is_exhausted() {
        let token = input.next()?.clone();
        match token {
            Token::Function(function) if function.eq_ignore_ascii_case("format") => {
                let format = input.parse_nested_block(|input| {
                    Ok(match input.next()? {
                        Token::QuotedString(s) | Token::Ident(s) => s.to_string(),
                        _ => return Err(invalid()),
                    })
                })?;
                usable &= is_supported_format(&format);
            }
            Token::Function(function) if function.eq_ignore_ascii_case("tech") => {
                input.parse_nested_block(|input| {
                    while input.next().is_ok() {}
                    Ok::<_, ParseError<StyleParseError>>(())
                })?;
            }
            _ => return Err(invalid()),
        }
    }
    // (a `local()` source has neither)
    if matches!(source, FontSource::Local(_)) && !usable {
        return Ok(None);
    }
    Ok(usable.then_some(source))
}

/// The font formats that can be embedded: TrueType and OpenType files and collections.
fn is_supported_format(format: &str) -> bool {
    matches!(
        format.to_ascii_lowercase().as_str(),
        "truetype" | "opentype" | "collection" | "truetype-variations" | "opentype-variations"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(css: &str) -> Option<FontFaceRule> {
        let mut parser = Parser::new(css);
        parse_font_face(&mut parser, Path::new("/doc"))
    }

    #[test]
    fn reads_the_descriptors() {
        let rule = parse(
            "font-family: 'My Font'; font-weight: 300 600; font-style: italic;
             src: local(Foo Bar), url(a.woff2) format('woff2'), url('a.ttf') format(truetype), url(b.otf);",
        )
        .unwrap();
        assert_eq!(rule.family, "My Font");
        assert_eq!(rule.weight, (300.0, 600.0));
        assert_eq!(rule.style, FontStyle::Italic);
        assert_eq!(
            rule.sources,
            [
                FontSource::Local("Foo Bar".into()),
                FontSource::Url("a.ttf".into()),
                FontSource::Url("b.otf".into()),
            ]
        );
        assert_eq!(rule.base_dir, Path::new("/doc"));
    }

    #[test]
    fn defaults_and_keywords() {
        let rule = parse("font-family: A B; src: url(x.ttf); font-weight: bold").unwrap();
        assert_eq!(rule.family, "A B");
        assert_eq!(rule.weight, (700.0, 700.0));
        assert_eq!(rule.style, FontStyle::Normal);
        let rule = parse("font-family: A; src: url(x.ttf); font-style: oblique 10deg 20deg").unwrap();
        assert_eq!(rule.style, FontStyle::Oblique);
        assert_eq!(rule.weight, (400.0, 400.0));
    }

    #[test]
    fn invalid_rules_are_dropped() {
        assert!(parse("src: url(x.ttf)").is_none());
        assert!(parse("font-family: A").is_none());
        // a generic family cannot be defined
        assert!(parse("font-family: serif; src: url(x.ttf)").is_none());
    }

    #[test]
    fn invalid_descriptors_do_not_spoil_the_others() {
        let rule = parse("font-family: A; src: url(x.ttf); font-weight: 5000; unicode-range: U+0-7F; font-display: swap")
            .unwrap();
        assert_eq!(rule.weight, (400.0, 400.0));
    }

    #[test]
    fn unusable_sources_are_skipped() {
        let rule = parse("font-family: A; src: url(x.woff) format('woff'), 12px, url(y.ttf)").unwrap();
        assert_eq!(rule.sources, [FontSource::Url("y.ttf".into())]);
    }
}
