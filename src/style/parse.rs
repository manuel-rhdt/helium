//! This module provides CSS declaration parsing using the `cssparser` and `selectors` crates.

use super::properties::specified::{CssWideDeclaration, Font, ParseDeclaration, PropertyDeclaration};
use crate::tree::ElementRef;

use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss,
};
use html5ever::{LocalName, Namespace};
use log::warn;
use precomputed_hash::PrecomputedHash;
use selectors::context::{MatchingForInvalidation, NeedsSelectorFlags, QuirksMode, SelectorCaches};
use selectors::matching;
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, SelectorImpl, SelectorList, SelectorParseErrorKind,
};
use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StyleParseError {
    #[error("unknown property {name}")]
    UnknownProperty { name: String },
    #[error("unsupported value")]
    InvalidValue,
    #[error("selector parser error {0:?}")]
    SelectorParserError(SelectorParseErrorKind),
}

impl From<SelectorParseErrorKind> for StyleParseError {
    fn from(err: SelectorParseErrorKind) -> Self {
        StyleParseError::SelectorParserError(err)
    }
}

#[derive(Debug)]
pub struct Styles {
    rules: Vec<QualifiedRule>,
}

impl Default for Styles {
    fn default() -> Self {
        let ua_style = include_str!("../../resources/user-agent.css");
        let mut styles = Styles { rules: vec![] };
        styles.parse_css(ua_style);
        styles
    }
}

impl Styles {
    pub fn parse_css(&mut self, input: &str) {
        let mut parser = Parser::new(input);
        let mut top_level = TopLevelParser::default();

        for rule in StyleSheetParser::new(&mut parser, &mut top_level) {
            match rule {
                Err((error, text, _location)) => {
                    warn!("Could not parse CSS rule {:?}: {:?}", text, error)
                }
                Ok(rule) => self.rules.push(rule),
            }
        }
    }

    pub fn add_stylesheet(&mut self, href: &str, base_dir: &Path) {
        if href.starts_with("//")
            || href.starts_with("http://")
            || href.starts_with("https://")
            || href.starts_with("data:")
        {
            warn!("Skipping non-local stylesheet \"{}\"", href);
            return;
        }

        let path = Path::new(href);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        };

        match std::fs::read_to_string(&path) {
            Ok(css) => self.parse_css(&css),
            Err(error) => warn!(
                "Could not read stylesheet \"{}\": {}",
                path.display(),
                error
            ),
        }
    }

    pub fn rules_iter(&self) -> impl DoubleEndedIterator<Item = &QualifiedRule> {
        self.rules.iter()
    }
}

#[derive(Debug)]
pub enum GeneralizedSelector {
    AtPage(PageSelector),
    Selectors(SelectorList<Selectors>),
}

/// The page pseudo-class of an `@page` rule.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PageSelector {
    Any,
    First,
    Left,
    Right,
}

/// The margin boxes that can hold headers and footers.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MarginBoxKind {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl MarginBoxKind {
    pub const ALL: [MarginBoxKind; 6] = [
        MarginBoxKind::TopLeft,
        MarginBoxKind::TopCenter,
        MarginBoxKind::TopRight,
        MarginBoxKind::BottomLeft,
        MarginBoxKind::BottomCenter,
        MarginBoxKind::BottomRight,
    ];

    fn from_name(name: &str) -> Option<Self> {
        Some(match_ignore_ascii_case! { name,
            "top-left" => MarginBoxKind::TopLeft,
            "top-center" => MarginBoxKind::TopCenter,
            "top-right" => MarginBoxKind::TopRight,
            "bottom-left" => MarginBoxKind::BottomLeft,
            "bottom-center" => MarginBoxKind::BottomCenter,
            "bottom-right" => MarginBoxKind::BottomRight,
            _ => return None,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CounterStyle {
    Decimal,
    DecimalLeadingZero,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CounterName {
    Page,
    Pages,
}

/// One component of the value of the `content` property of a margin box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentItem {
    String(String),
    Counter(CounterName, CounterStyle),
}

/// A margin at-rule such as `@bottom-center { content: counter(page) }` inside of `@page`.
#[derive(Debug)]
pub struct MarginBoxRule {
    pub kind: MarginBoxKind,
    pub properties: PropertyDeclarationBlock,
    pub content: Option<Vec<ContentItem>>,
}

#[derive(Debug)]
pub struct QualifiedRule {
    selectors: GeneralizedSelector,
    pub properties: PropertyDeclarationBlock,
    pub margin_boxes: Vec<MarginBoxRule>,
}

impl QualifiedRule {
    pub fn new(selectors: SelectorList<Selectors>, properties: PropertyDeclarationBlock) -> Self {
        QualifiedRule {
            selectors: GeneralizedSelector::Selectors(selectors),
            properties,
            margin_boxes: vec![],
        }
    }

    pub fn matches(&self, elem: &ElementRef) -> bool {
        self.matching_specificity(elem).is_some()
    }

    /// The specificity of the most specific selector of this rule that matches `elem`, or `None`
    /// if the rule does not match. Used to order the cascade: rules with higher specificity are
    /// applied later and win over earlier declarations of the same property.
    pub fn matching_specificity(&self, elem: &ElementRef) -> Option<u32> {
        let GeneralizedSelector::Selectors(list) = &self.selectors else {
            return None;
        };
        let mut caches = SelectorCaches::default();
        let mut context = matching::MatchingContext::new(
            matching::MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        list.slice()
            .iter()
            .filter(|selector| matching::matches_selector(selector, 0, None, elem, &mut context))
            .map(|selector| selector.specificity())
            .max()
    }

    pub fn is_page_declaration(&self) -> bool {
        self.page_selector().is_some()
    }

    /// The page pseudo-class if this is an `@page` rule.
    pub fn page_selector(&self) -> Option<PageSelector> {
        match &self.selectors {
            GeneralizedSelector::AtPage(selector) => Some(*selector),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct PropertyDeclarationBlock {
    declarations: Vec<PropertyDeclaration<'static>>,
}

impl PropertyDeclarationBlock {
    pub fn new() -> Self {
        Default::default()
    }

    /// Parses the value of a `style` attribute (a declaration list without surrounding braces).
    pub fn parse_inline(input: &str) -> Self {
        let mut parser = Parser::new(input);
        Self::parse_declarations(&mut parser)
    }

    /// Parses a declaration list up to the end of the input.
    fn parse_declarations(input: &mut Parser) -> Self {
        let mut declaration_parser = StyleDeclarationParser {};
        let mut block = Self::new();
        for declaration in RuleBodyParser::new(input, &mut declaration_parser) {
            match declaration {
                Ok(declaration) => block.declarations.push(declaration),
                Err((error, text, _location)) => {
                    warn!("Error while parsing declaration {:?}: {:?}", text, error)
                }
            }
        }
        block
    }

    pub fn declarations(&self) -> impl Iterator<Item = &PropertyDeclaration<'_>> {
        let iter1 = self.declarations.iter().filter(|decl| match decl {
            PropertyDeclaration::Font(Font::FontSize(_)) => true,
            _ => false,
        });
        let iter2 = self.declarations.iter().filter(|decl| match decl {
            PropertyDeclaration::Font(Font::FontSize(_)) => false,
            _ => true,
        });
        iter1.chain(iter2)
    }
}

#[derive(Debug, Default)]
struct TopLevelParser {}

impl<'i> QualifiedRuleParser<'i> for TopLevelParser {
    type Prelude = SelectorList<Selectors>;
    type QualifiedRule = QualifiedRule;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let selector_parser = SelectorParser::default();
        let selector_list = SelectorList::parse(&selector_parser, input, ParseRelative::No)
            .map_err(|err| err.into::<StyleParseError>())?;
        Ok(selector_list)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        Ok(QualifiedRule::new(
            prelude,
            PropertyDeclarationBlock::parse_declarations(input),
        ))
    }
}

impl<'i> AtRuleParser<'i> for TopLevelParser {
    type Prelude = PageSelector;
    type AtRule = QualifiedRule;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        if !name.eq_ignore_ascii_case("page") {
            return Err(ParseError::from_basic_kind(
                BasicParseErrorKind::AtRuleInvalid,
            ));
        }
        if input.is_exhausted() {
            return Ok(PageSelector::Any);
        }
        input.expect_colon()?;
        let ident = input.expect_ident()?.clone();
        Ok(match_ignore_ascii_case! { &ident,
            "first" => PageSelector::First,
            "left" => PageSelector::Left,
            "right" => PageSelector::Right,
            _ => return Err(ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue) }),
        })
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        let body = parse_page_body(input);
        Ok(QualifiedRule {
            selectors: GeneralizedSelector::AtPage(prelude),
            properties: body.properties,
            margin_boxes: body.margin_boxes,
        })
    }
}

#[derive(Default)]
struct PageBody {
    properties: PropertyDeclarationBlock,
    content: Option<Vec<ContentItem>>,
    margin_boxes: Vec<MarginBoxRule>,
}

enum PageBodyItem {
    Declaration(PropertyDeclaration<'static>),
    Content(Vec<ContentItem>),
    MarginBox(MarginBoxRule),
}

/// Parses the contents of an `@page` rule or of a margin at-rule.
fn parse_page_body(input: &mut Parser) -> PageBody {
    let mut body = PageBody::default();
    for item in RuleBodyParser::new(input, &mut PageBodyParser) {
        match item {
            Ok(PageBodyItem::Declaration(decl)) => body.properties.declarations.push(decl),
            Ok(PageBodyItem::Content(content)) => body.content = Some(content),
            Ok(PageBodyItem::MarginBox(margin_box)) => body.margin_boxes.push(margin_box),
            Err((error, text, _location)) => {
                warn!("Error while parsing declaration {:?}: {:?}", text, error)
            }
        }
    }
    body
}

fn parse_content<'i>(
    input: &mut Parser<'i>,
) -> Result<Vec<ContentItem>, ParseError<StyleParseError>> {
    let mut items = vec![];
    while !input.is_exhausted() {
        let token = input.next()?.clone();
        match token {
            cssparser::Token::QuotedString(string) => {
                items.push(ContentItem::String(string.to_string()))
            }
            cssparser::Token::Ident(ref ident)
                if ident.eq_ignore_ascii_case("none") || ident.eq_ignore_ascii_case("normal") => {}
            cssparser::Token::Function(ref name) if name.eq_ignore_ascii_case("counter") => {
                let item = input.parse_nested_block(|input| {
                    let counter = input.expect_ident()?.clone();
                    let counter = match_ignore_ascii_case! { &counter,
                        "page" => CounterName::Page,
                        "pages" => CounterName::Pages,
                        _ => return Err(ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue) }),
                    };
                    let style = if input.try_parse(|input| input.expect_comma()).is_ok() {
                        let style = input.expect_ident()?.clone();
                        match_ignore_ascii_case! { &style,
                            "decimal" => CounterStyle::Decimal,
                            "decimal-leading-zero" => CounterStyle::DecimalLeadingZero,
                            "lower-roman" => CounterStyle::LowerRoman,
                            "upper-roman" => CounterStyle::UpperRoman,
                            "lower-alpha" => CounterStyle::LowerAlpha,
                            "lower-latin" => CounterStyle::LowerAlpha,
                            "upper-alpha" => CounterStyle::UpperAlpha,
                            "upper-latin" => CounterStyle::UpperAlpha,
                            // an unsupported style must not remove the whole margin box: fall
                            // back to decimal, which is what the CSS spec prescribes
                            _ => {
                                warn!("Unsupported counter style {:?}, using decimal instead", &*style);
                                CounterStyle::Decimal
                            }
                        }
                    } else {
                        CounterStyle::Decimal
                    };
                    Ok(ContentItem::Counter(counter, style))
                })?;
                items.push(item);
            }
            // `string()`, `attr()` and other functions are not supported; skip one item rather
            // than dropping the whole declaration
            cssparser::Token::Function(ref name) => {
                input.parse_nested_block(|_| Ok::<_, ParseError<StyleParseError>>(()))?;
                warn!("Unsupported content function {:?}, ignoring it", name);
            }
            _ => warn!("Unsupported value in `content`, ignoring it"),
        }
    }
    Ok(items)
}

/// Whether any component of a margin declaration is `auto`.
fn has_auto_margin(declaration: &PropertyDeclaration) -> bool {
    use super::properties::specified::{AutoOrQuantityOrPercentage, Position};

    fn is_auto(value: &AutoOrQuantityOrPercentage) -> bool {
        matches!(value, AutoOrQuantityOrPercentage::Auto)
    }

    match declaration {
        PropertyDeclaration::Position(Position::Margin(list)) => {
            list.elements().iter().any(is_auto)
        }
        PropertyDeclaration::Position(
            Position::MarginTop(value)
            | Position::MarginBottom(value)
            | Position::MarginLeft(value)
            | Position::MarginRight(value),
        ) => is_auto(value),
        _ => false,
    }
}

struct PageBodyParser;

impl<'i> cssparser::DeclarationParser<'i> for PageBodyParser {
    type Declaration = PageBodyItem;
    type Error = StyleParseError;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        if name.eq_ignore_ascii_case("content") {
            return Ok(PageBodyItem::Content(parse_content(input)?));
        }
        let declaration = parse_property(name, input)?;
        // `auto` is not a valid page margin (CSS Paged Media 3, § 7.1). It would resolve to a
        // margin of zero and silently remove the page margin, so the declaration is rejected and
        // the default margin from the user agent stylesheet stays in effect.
        if has_auto_margin(&declaration) {
            return Err(ParseError {
                kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue),
            });
        }
        Ok(PageBodyItem::Declaration(declaration))
    }
}

impl<'i> AtRuleParser<'i> for PageBodyParser {
    type Prelude = MarginBoxKind;
    type AtRule = PageBodyItem;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        _input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        MarginBoxKind::from_name(&name)
            .ok_or_else(|| ParseError::from_basic_kind(BasicParseErrorKind::AtRuleInvalid))
    }

    fn parse_block<'t>(
        &mut self,
        kind: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        let body = parse_page_body(input);
        Ok(PageBodyItem::MarginBox(MarginBoxRule {
            kind,
            properties: body.properties,
            content: body.content,
        }))
    }
}

impl<'i> QualifiedRuleParser<'i> for PageBodyParser {
    type Prelude = ();
    type QualifiedRule = PageBodyItem;
    type Error = StyleParseError;
}

impl<'i> RuleBodyItemParser<'i, PageBodyItem, StyleParseError> for PageBodyParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// An identifier (`#id`, `.class`, ...) as it appears in a selector.
///
/// `selectors` needs its types to be serializable with [`ToCss`], which cannot be implemented for
/// the types of html5ever, so we wrap them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CssIdent(pub LocalName);

impl<'a> From<&'a str> for CssIdent {
    fn from(s: &'a str) -> Self {
        CssIdent(LocalName::from(s))
    }
}

impl ToCss for CssIdent {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl PrecomputedHash for CssIdent {
    fn precomputed_hash(&self) -> u32 {
        self.0.precomputed_hash()
    }
}

impl std::borrow::Borrow<LocalName> for CssIdent {
    fn borrow(&self) -> &LocalName {
        &self.0
    }
}

/// The namespace of a selector, see [`CssIdent`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CssNamespace(pub Namespace);

impl<'a> From<&'a str> for CssNamespace {
    fn from(s: &'a str) -> Self {
        CssNamespace(Namespace::from(s))
    }
}

impl ToCss for CssNamespace {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        cssparser::serialize_identifier(&self.0, dest)
    }
}

impl PrecomputedHash for CssNamespace {
    fn precomputed_hash(&self) -> u32 {
        self.0.precomputed_hash()
    }
}

impl std::borrow::Borrow<Namespace> for CssNamespace {
    fn borrow(&self) -> &Namespace {
        &self.0
    }
}

/// The value of an attribute selector, see [`CssIdent`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct CssString(pub String);

impl<'a> From<&'a str> for CssString {
    fn from(s: &'a str) -> Self {
        CssString(s.to_owned())
    }
}

impl AsRef<str> for CssString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ToCss for CssString {
    fn to_css<W: std::fmt::Write>(&self, dest: &mut W) -> std::fmt::Result {
        cssparser::serialize_string(&self.0, dest)
    }
}

#[derive(Debug, Clone)]
pub struct Selectors;

impl SelectorImpl for Selectors {
    type ExtraMatchingData<'a> = ();
    type AttrValue = CssString;
    type Identifier = CssIdent;
    type LocalName = CssIdent;
    type NamespacePrefix = CssIdent;
    type NamespaceUrl = CssNamespace;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = LocalName;

    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElement;
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum PseudoClass {
    AnyLink,
    Link,
    Visited,
    Active,
    Focus,
    Hover,
    Enabled,
    Disabled,
    Checked,
    Indeterminate,
}

impl NonTSPseudoClass for PseudoClass {
    fn is_active_or_hover(&self) -> bool {
        match self {
            PseudoClass::Active | PseudoClass::Hover => true,
            _ => false,
        }
    }
    fn is_user_action_state(&self) -> bool {
        false
    }
}

impl ToCss for PseudoClass {
    fn to_css<W>(&self, dest: &mut W) -> std::fmt::Result
    where
        W: std::fmt::Write,
    {
        dest.write_str(match *self {
            PseudoClass::AnyLink => ":any-link",
            PseudoClass::Link => ":link",
            PseudoClass::Visited => ":visited",
            PseudoClass::Active => ":active",
            PseudoClass::Focus => ":focus",
            PseudoClass::Hover => ":hover",
            PseudoClass::Enabled => ":enabled",
            PseudoClass::Disabled => ":disabled",
            PseudoClass::Checked => ":checked",
            PseudoClass::Indeterminate => ":indeterminate",
        })
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub enum PseudoElement {}

impl ToCss for PseudoElement {
    fn to_css<W>(&self, _dest: &mut W) -> std::fmt::Result
    where
        W: std::fmt::Write,
    {
        match *self {}
    }
}

impl selectors::parser::PseudoElement for PseudoElement {}

#[derive(Default)]
struct SelectorParser;

impl<'i> selectors::Parser<'i> for SelectorParser {
    type Impl = Selectors;
    type Error = SelectorParseErrorKind;

    fn parse_non_ts_pseudo_class(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<PseudoClass, ParseError<SelectorParseErrorKind>> {
        use self::PseudoClass::*;
        if name.eq_ignore_ascii_case("any-link") {
            Ok(AnyLink)
        } else if name.eq_ignore_ascii_case("link") {
            Ok(Link)
        } else if name.eq_ignore_ascii_case("visited") {
            Ok(Visited)
        } else if name.eq_ignore_ascii_case("active") {
            Ok(Active)
        } else if name.eq_ignore_ascii_case("focus") {
            Ok(Focus)
        } else if name.eq_ignore_ascii_case("hover") {
            Ok(Hover)
        } else if name.eq_ignore_ascii_case("enabled") {
            Ok(Enabled)
        } else if name.eq_ignore_ascii_case("disabled") {
            Ok(Disabled)
        } else if name.eq_ignore_ascii_case("checked") {
            Ok(Checked)
        } else if name.eq_ignore_ascii_case("indeterminate") {
            Ok(Indeterminate)
        } else {
            Err(ParseError::custom(
                SelectorParseErrorKind::UnsupportedPseudoClassOrElement,
            ))
        }
    }
}

struct StyleDeclarationParser {}

impl<'i> cssparser::DeclarationParser<'i> for StyleDeclarationParser {
    type Declaration = PropertyDeclaration<'static>;
    type Error = StyleParseError;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        parse_property(name, input)
    }
}

/// Parses the value of a property declaration, which may be a CSS-wide keyword like `inherit`.
fn parse_property<'i>(
    name: CowRcStr<'i>,
    input: &mut Parser<'i>,
) -> Result<PropertyDeclaration<'static>, ParseError<StyleParseError>> {
    if let Some(declaration) = CssWideDeclaration::parse(&name, input) {
        return Ok(PropertyDeclaration::CssWide(declaration));
    }
    PropertyDeclaration::parse_declaration(name, input)
}

impl<'i> AtRuleParser<'i> for StyleDeclarationParser {
    type Prelude = ();
    type AtRule = PropertyDeclaration<'static>;
    type Error = StyleParseError;
}

impl<'i> QualifiedRuleParser<'i> for StyleDeclarationParser {
    type Prelude = ();
    type QualifiedRule = PropertyDeclaration<'static>;
    type Error = StyleParseError;
}

impl<'i> RuleBodyItemParser<'i, PropertyDeclaration<'static>, StyleParseError>
    for StyleDeclarationParser
{
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}
