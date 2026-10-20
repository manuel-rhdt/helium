//! This module provides CSS declaration parsing using the `cssparser` and `selectors` crates.

use super::properties::specified::{Font, ParseDeclaration, PropertyDeclaration};
use crate::tree::ElementRef;

use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss,
};
use html5ever::{LocalName, Namespace};
use log::{error, warn};
use precomputed_hash::PrecomputedHash;
use selectors::context::{
    MatchingForInvalidation, NeedsSelectorFlags, QuirksMode, SelectorCaches,
};
use selectors::matching;
use selectors::parser::{
    NonTSPseudoClass, ParseRelative, SelectorImpl, SelectorList, SelectorParseErrorKind,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StyleParseError {
    #[error("unknown property {name}")]
    UnknownProperty { name: String },
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

    pub fn add_stylesheet(&mut self, href: &str) {
        error!("add_stylesheet unimplemented");
        warn!("Could not read stylesheet \"{}\"", href)
    }

    pub fn rules_iter(&self) -> impl DoubleEndedIterator<Item = &QualifiedRule> {
        self.rules.iter()
    }
}

#[derive(Debug)]
pub enum GeneralizedSelector {
    AtPage,
    Selectors(SelectorList<Selectors>),
}

#[derive(Debug)]
pub struct QualifiedRule {
    selectors: GeneralizedSelector,
    pub properties: PropertyDeclarationBlock,
}

impl QualifiedRule {
    pub fn new(selectors: SelectorList<Selectors>, properties: PropertyDeclarationBlock) -> Self {
        QualifiedRule {
            selectors: GeneralizedSelector::Selectors(selectors),
            properties,
        }
    }

    pub fn matches(&self, elem: &ElementRef) -> bool {
        let mut caches = SelectorCaches::default();
        let mut context = matching::MatchingContext::new(
            matching::MatchingMode::Normal,
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        if let GeneralizedSelector::Selectors(list) = &self.selectors {
            matching::matches_selector_list(list, elem, &mut context)
        } else {
            false
        }
    }

    pub fn is_page_declaration(&self) -> bool {
        if let GeneralizedSelector::AtPage = &self.selectors {
            true
        } else {
            false
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

    pub fn declarations(&self) -> impl Iterator<Item = &PropertyDeclaration> {
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
    type Prelude = ();
    type AtRule = QualifiedRule;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        _input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        if name.eq_ignore_ascii_case("page") {
            Ok(())
        } else {
            Err(ParseError::from_basic_kind(
                BasicParseErrorKind::AtRuleInvalid,
            ))
        }
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        Ok(QualifiedRule {
            selectors: GeneralizedSelector::AtPage,
            properties: PropertyDeclarationBlock::parse_declarations(input),
        })
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
        PropertyDeclaration::parse_declaration(name, input)
    }
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
