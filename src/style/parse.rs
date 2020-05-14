//! This module provides CSS declaration parsing using the `cssparser` and `selectors` crates.

use super::properties::specified::{ParseDeclaration, PropertyDeclaration};
use crate::tree::ElementRef;

use cssparser::{
    AtRuleParser, AtRuleType, CowRcStr, ParseError, Parser, ParserInput, QualifiedRuleParser,
    RuleListParser, SourceLocation, ToCss,
};
use html5ever::{LocalName, Namespace};
use log::error;
use selectors::context::QuirksMode;
use selectors::matching;
use selectors::parser::{NonTSPseudoClass, SelectorImpl, SelectorList, SelectorParseErrorKind};

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
        let mut input = ParserInput::new(input);
        let mut parser = Parser::new(&mut input);

        let qualified_rule_iter =
            RuleListParser::new_for_stylesheet(&mut parser, TopLevelParser::default());

        for rule in qualified_rule_iter {
            match rule {
                Err(error) => error!("Could not parse CSS rule: {:?}", error),
                Ok(rule) => self.rules.push(rule),
            }
        }
    }

    pub fn add_stylesheet(&mut self, href: &str) {
        error!("add_stylesheet unimplemented");
        error!("Could not read stylesheet \"{}\"", href)
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
        let mut context = matching::MatchingContext::new(
            matching::MatchingMode::Normal,
            None,
            None,
            QuirksMode::NoQuirks,
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

    pub fn declarations(&self) -> impl ExactSizeIterator<Item = &PropertyDeclaration> {
        self.declarations.iter()
    }
}

#[derive(Debug, Default)]
struct TopLevelParser {}

impl<'i> QualifiedRuleParser<'i> for TopLevelParser {
    type Prelude = SelectorList<Selectors>;
    type QualifiedRule = QualifiedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let selector_parser = SelectorParser::default();
        let selector_list =
            SelectorList::parse(&selector_parser, input).map_err(|_| input.new_custom_error(()))?;
        Ok(selector_list)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _location: SourceLocation,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        let declaration_parser = StyleDeclarationParser {};
        let declaration_list_parser =
            cssparser::DeclarationListParser::new(input, declaration_parser);

        let mut declaration_block = PropertyDeclarationBlock::new();

        for declaration in declaration_list_parser {
            match declaration {
                Ok(declaration) => declaration_block.declarations.push(declaration),
                Err(err) => error!("Error while parsing declaration: {:?}", err),
            };
        }

        Ok(QualifiedRule::new(prelude, declaration_block))
    }
}

impl<'i> AtRuleParser<'i> for TopLevelParser {
    type PreludeNoBlock = ();
    type PreludeBlock = ();
    type AtRule = QualifiedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<AtRuleType<Self::PreludeNoBlock, Self::PreludeBlock>, ParseError<'i, Self::Error>>
    {
        if name.eq_ignore_ascii_case("page") {
            Ok(AtRuleType::WithBlock(()))
        } else {
            let _ = input;
            Err(input.new_error(cssparser::BasicParseErrorKind::AtRuleInvalid(name)))
        }
    }

    fn parse_block<'t>(
        &mut self,
        _prelude: Self::PreludeBlock,
        _location: SourceLocation,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        let declaration_parser = StyleDeclarationParser {};
        let declaration_list_parser =
            cssparser::DeclarationListParser::new(input, declaration_parser);

        let mut declaration_block = PropertyDeclarationBlock::new();

        for declaration in declaration_list_parser {
            match declaration {
                Ok(declaration) => declaration_block.declarations.push(declaration),
                Err(err) => error!("Error while parsing declaration: {:?}", err),
            };
        }

        Ok(QualifiedRule {
            selectors: GeneralizedSelector::AtPage,
            properties: declaration_block,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Selectors;

impl SelectorImpl for Selectors {
    type AttrValue = String;
    type Identifier = LocalName;
    type ClassName = LocalName;
    type LocalName = LocalName;
    type NamespacePrefix = LocalName;
    type NamespaceUrl = Namespace;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = LocalName;

    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElement;

    type ExtraMatchingData = ();
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
    type Impl = Selectors;

    fn is_active_or_hover(&self) -> bool {
        match self {
            PseudoClass::Active | PseudoClass::Hover => true,
            _ => false,
        }
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

impl selectors::parser::PseudoElement for PseudoElement {
    type Impl = Selectors;
}

#[derive(Default)]
struct SelectorParser;

impl<'i> selectors::Parser<'i> for SelectorParser {
    type Impl = Selectors;
    type Error = SelectorParseErrorKind<'i>;

    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<PseudoClass, ParseError<'i, SelectorParseErrorKind<'i>>> {
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
            Err(
                location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                    name,
                )),
            )
        }
    }
}

struct StyleDeclarationParser {}

impl<'i> cssparser::DeclarationParser<'i> for StyleDeclarationParser {
    type Declaration = PropertyDeclaration<'static>;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        PropertyDeclaration::parse_declaration(name, input)
    }
}

impl<'i> AtRuleParser<'i> for StyleDeclarationParser {
    type PreludeNoBlock = ();
    type PreludeBlock = ();
    type AtRule = PropertyDeclaration<'static>;
    type Error = ();
}
