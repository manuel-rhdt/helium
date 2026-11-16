//! This module provides CSS declaration parsing using the `cssparser` and `selectors` crates.

use super::font_face::{FontFaceRule, parse_font_face};
use super::media::MediaEnv;
use crate::layout::text::font::FontContext;
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
    /// The number of rules at the start of `rules` that come from the user agent style sheet.
    user_agent_rules: usize,
    /// What the media queries of the style sheets are evaluated against.
    media: MediaEnv,
    /// The `@font-face` rules of the document, in order.
    font_faces: Vec<FontFaceRule>,
    /// The fonts of the document, made from `font_faces` when they are first needed.
    fonts: std::sync::OnceLock<std::sync::Arc<FontContext>>,
}

impl Default for Styles {
    fn default() -> Self {
        let ua_style = include_str!("../../resources/user-agent.css");
        let mut styles = Styles::empty();
        styles.parse_css(ua_style);
        styles.user_agent_rules = styles.rules.len();
        styles
    }
}

impl Styles {
    /// Style sheets without any rule, not even the ones of the user agent. Cheap, unlike
    /// `default()`, which parses the user agent style sheet.
    pub fn empty() -> Self {
        Styles {
            rules: vec![],
            user_agent_rules: 0,
            media: MediaEnv::default(),
            font_faces: vec![],
            fonts: Default::default(),
        }
    }

    /// The fonts of the document: the installed and built-in ones and the faces of its
    /// `@font-face` rules. The faces are loaded the first time this is called after the style
    /// sheets changed.
    pub fn font_context(&self) -> std::sync::Arc<FontContext> {
        self.fonts.get_or_init(|| FontContext::new(&self.font_faces)).clone()
    }

    /// Sets the page that the media queries of the style sheets read from now on are evaluated
    /// against.
    pub fn set_media(&mut self, media: MediaEnv) {
        self.media = media;
    }

    /// Whether the media queries of a `media` attribute match (no queries match everything).
    pub fn media_matches(&self, queries: &str) -> bool {
        super::media::matches_text(queries, &self.media)
    }

    /// Reads a style sheet whose relative URLs (`@font-face` sources) are relative to the
    /// current directory.
    pub fn parse_css(&mut self, input: &str) {
        self.parse_css_in(input, Path::new("."))
    }

    /// Reads a style sheet whose relative URLs are relative to `base_dir`.
    pub fn parse_css_in(&mut self, input: &str, base_dir: &Path) {
        // The content of a `<style>` of an XHTML document may be wrapped in a CDATA section,
        // which the HTML parser keeps as text.
        let unwrapped;
        let input = if input.contains("<![CDATA[") {
            unwrapped = input.replace("<![CDATA[", "").replace("]]>", "");
            &unwrapped
        } else {
            input
        };
        let mut parser = Parser::new(input);
        let mut top_level = TopLevelParser { media: self.media, base_dir: base_dir.to_path_buf() };
        self.fonts.take();

        for rule in StyleSheetParser::new(&mut parser, &mut top_level) {
            match rule {
                Err((error, text, _location)) => {
                    warn!("Could not parse CSS rule {:?}: {:?}", text, error)
                }
                Ok(rules) => {
                    for rule in rules {
                        match rule {
                            TopLevelRule::Style(rule) => self.rules.push(rule),
                            TopLevelRule::FontFace(rule) => self.font_faces.push(rule),
                        }
                    }
                }
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
            Ok(css) => self.parse_css_in(&css, path.parent().unwrap_or(base_dir)),
            Err(error) => warn!(
                "Could not read stylesheet \"{}\": {}",
                path.display(),
                error
            ),
        }
    }

    /// Whether the rule at `index` of `rules_iter` comes from the user agent style sheet.
    pub fn is_user_agent_rule(&self, index: usize) -> bool {
        index < self.user_agent_rules
    }

    /// Removes the rules of the document, to read its style sheets again.
    pub fn clear_author_rules(&mut self) {
        self.rules.truncate(self.user_agent_rules);
        self.font_faces.clear();
        self.fonts.take();
    }

    pub fn rules_iter(&self) -> impl DoubleEndedIterator<Item = &QualifiedRule> {
        self.rules.iter()
    }

    /// The rules of the document, without the ones of the user agent style sheet.
    pub fn author_rules_iter(&self) -> impl DoubleEndedIterator<Item = &QualifiedRule> {
        self.rules[self.user_agent_rules.min(self.rules.len())..].iter()
    }
}

#[derive(Debug)]
pub enum GeneralizedSelector {
    AtPage(PageSelector),
    Selectors(SelectorList<Selectors>),
}

/// The pseudo-class of an `@page` selector.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PagePseudo {
    First,
    Left,
    Right,
}

/// The selector of an `@page` rule: an optional page name (see the `page` property) and an
/// optional pseudo-class.
/// https://www.w3.org/TR/css-page-3/#page-selectors
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageSelector {
    pub name: Option<std::sync::Arc<str>>,
    pub pseudo: Option<PagePseudo>,
}

impl PageSelector {
    /// The selector of a plain `@page` rule.
    pub fn any() -> Self {
        Self::default()
    }

    pub fn pseudo(pseudo: PagePseudo) -> Self {
        PageSelector { name: None, pseudo: Some(pseudo) }
    }

    /// The specificity: the page name counts most, then `:first`, then `:left` and `:right`.
    /// https://www.w3.org/TR/css-page-3/#cascading-and-page-context
    pub fn specificity(&self) -> u32 {
        let name = if self.name.is_some() { 100 } else { 0 };
        let pseudo = match self.pseudo {
            None => 0,
            Some(PagePseudo::First) => 10,
            Some(PagePseudo::Left | PagePseudo::Right) => 1,
        };
        name + pseudo
    }
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
    /// ٠ ١ ٢ ٣ …
    ArabicIndic,
    /// The extended Arabic-Indic digits of Persian and Urdu: ۰ ۱ ۲ ۳ …
    Persian,
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
    pub fn page_selector(&self) -> Option<&PageSelector> {
        match &self.selectors {
            GeneralizedSelector::AtPage(selector) => Some(selector),
            _ => None,
        }
    }
}

/// A declaration as it is written: the name of the property and the text of its value. Every
/// declaration is kept, also those of properties that are not known, for the CSS of the SVG
/// elements, which Helium does not interpret itself.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDeclaration {
    pub name: String,
    /// The text of the value, without a trailing `!important`.
    pub value: String,
    pub important: bool,
}

/// A declaration and whether it is `!important`.
#[derive(Debug)]
pub struct ParsedDeclaration {
    declaration: PropertyDeclaration<'static>,
    important: bool,
}

#[derive(Debug, Default)]
pub struct PropertyDeclarationBlock {
    declarations: Vec<ParsedDeclaration>,
    raw: Vec<RawDeclaration>,
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
        let mut declaration_parser = StyleDeclarationParser::default();
        let mut block = Self::new();
        for declaration in RuleBodyParser::new(input, &mut declaration_parser) {
            match declaration {
                Ok(declaration) => block.declarations.push(declaration),
                Err((error, text, _location)) => {
                    // the properties of SVG elements are for the SVG parser
                    if !is_svg_property(text) {
                        warn!("Error while parsing declaration {:?}: {:?}", text, error)
                    }
                }
            }
        }
        block.raw = declaration_parser.raw;
        block
    }

    /// The declarations as they are written.
    pub fn raw_declarations(&self) -> &[RawDeclaration] {
        &self.raw
    }

    /// The declarations that are not `!important`, in the order to apply them: `font-size`
    /// first, so that the lengths in `em` of the other declarations are relative to it.
    pub fn normal_declarations(&self) -> impl Iterator<Item = &PropertyDeclaration<'_>> {
        self.ordered(false)
    }

    /// The `!important` declarations, in the order to apply them.
    pub fn important_declarations(&self) -> impl Iterator<Item = &PropertyDeclaration<'_>> {
        self.ordered(true)
    }

    /// All declarations in the order to apply them: the ones that are not `!important`, then the
    /// ones that are, so that the latter win within the block. The cascade of several rules
    /// applies the two kinds in separate passes.
    pub fn declarations(&self) -> impl Iterator<Item = &PropertyDeclaration<'_>> {
        self.normal_declarations().chain(self.important_declarations())
    }

    pub fn has_important(&self) -> bool {
        self.declarations.iter().any(|declaration| declaration.important)
    }

    fn ordered(&self, important: bool) -> impl Iterator<Item = &PropertyDeclaration<'_>> {
        let is_font_size = |decl: &PropertyDeclaration| {
            matches!(decl, PropertyDeclaration::Font(Font::FontSize(_)))
        };
        let of_kind = move |parsed: &&ParsedDeclaration| parsed.important == important;
        let first = self
            .declarations
            .iter()
            .filter(of_kind)
            .filter(move |parsed| is_font_size(&parsed.declaration));
        let rest = self
            .declarations
            .iter()
            .filter(of_kind)
            .filter(move |parsed| !is_font_size(&parsed.declaration));
        first.chain(rest).map(|parsed| &parsed.declaration)
    }
}

/// Reads the rules of a style sheet: a rule of a `@media` block gives its rules, if the queries
/// match, so every rule is a list.
#[derive(Debug, Default)]
struct TopLevelParser {
    media: MediaEnv,
    /// The directory of the style sheet, for the URLs in it.
    base_dir: std::path::PathBuf,
}

/// A rule of the top level of a style sheet, or in a `@media` block.
enum TopLevelRule {
    Style(QualifiedRule),
    FontFace(FontFaceRule),
}

impl<'i> QualifiedRuleParser<'i> for TopLevelParser {
    type Prelude = SelectorList<Selectors>;
    type QualifiedRule = Vec<TopLevelRule>;
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
        Ok(vec![TopLevelRule::Style(QualifiedRule::new(
            prelude,
            PropertyDeclarationBlock::parse_declarations(input),
        ))])
    }
}

/// What follows the name of an at-rule of the top level.
enum AtPrelude {
    Page(PageSelector),
    /// Whether the media queries match.
    Media(bool),
    FontFace,
}

impl<'i> AtRuleParser<'i> for TopLevelParser {
    type Prelude = AtPrelude;
    type AtRule = Vec<TopLevelRule>;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        if name.eq_ignore_ascii_case("media") {
            let matches = super::media::matches(input, &self.media);
            // what is left of an invalid query is skipped with the rule
            while input.next().is_ok() {}
            return Ok(AtPrelude::Media(matches));
        }
        if name.eq_ignore_ascii_case("font-face") {
            return Ok(AtPrelude::FontFace);
        }
        if !name.eq_ignore_ascii_case("page") {
            return Err(ParseError::from_basic_kind(
                BasicParseErrorKind::AtRuleInvalid,
            ));
        }
        let invalid = || ParseError {
            kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue),
        };
        let name = input
            .try_parse(|input| input.expect_ident().map(|ident| ident.clone()))
            .ok()
            .map(|ident| std::sync::Arc::<str>::from(&*ident));
        if input.is_exhausted() {
            return Ok(AtPrelude::Page(PageSelector { name, pseudo: None }));
        }
        input.expect_colon()?;
        let ident = input.expect_ident()?.clone();
        let pseudo = match_ignore_ascii_case! { &ident,
            "first" => PagePseudo::First,
            "left" => PagePseudo::Left,
            "right" => PagePseudo::Right,
            _ => return Err(invalid()),
        };
        if !input.is_exhausted() {
            return Err(invalid());
        }
        Ok(AtPrelude::Page(PageSelector { name, pseudo: Some(pseudo) }))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        let prelude = match prelude {
            AtPrelude::Page(prelude) => prelude,
            AtPrelude::Media(matches) => {
                // the rules of the block, nested blocks included
                let mut rules = vec![];
                for rule in StyleSheetParser::new(input, self) {
                    match rule {
                        Err((error, text, _location)) => {
                            warn!("Could not parse CSS rule {:?}: {:?}", text, error)
                        }
                        Ok(nested) => rules.extend(nested),
                    }
                }
                return Ok(if matches { rules } else { vec![] });
            }
            AtPrelude::FontFace => {
                return Ok(parse_font_face(input, &self.base_dir)
                    .map(TopLevelRule::FontFace)
                    .into_iter()
                    .collect());
            }
        };
        let body = parse_page_body(input);
        Ok(vec![TopLevelRule::Style(QualifiedRule {
            selectors: GeneralizedSelector::AtPage(prelude),
            properties: body.properties,
            margin_boxes: body.margin_boxes,
        })])
    }
}

#[derive(Default)]
struct PageBody {
    properties: PropertyDeclarationBlock,
    content: Option<Vec<ContentItem>>,
    margin_boxes: Vec<MarginBoxRule>,
}

enum PageBodyItem {
    Declaration(ParsedDeclaration),
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
                            "arabic-indic" => CounterStyle::ArabicIndic,
                            "persian" => CounterStyle::Persian,
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
                input.parse_nested_block(|input| {
                    // consume the arguments: `parse_nested_block` fails if anything is left over
                    while input.next_including_whitespace_and_comments().is_ok() {}
                    Ok::<_, ParseError<StyleParseError>>(())
                })?;
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
        let declaration =
            input.parse_until_before(cssparser::Delimiter::Bang, |input| parse_property(name, input))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        // `auto` is not a valid page margin (CSS Paged Media 3, § 7.1). It would resolve to a
        // margin of zero and silently remove the page margin, so the declaration is rejected and
        // the default margin from the user agent stylesheet stays in effect.
        if has_auto_margin(&declaration) {
            return Err(ParseError {
                kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue),
            });
        }
        Ok(PageBodyItem::Declaration(ParsedDeclaration { declaration, important }))
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

/// Whether the text of a declaration is about a property that only SVG elements have.
fn is_svg_property(declaration: &str) -> bool {
    const PROPERTIES: &[&str] = &[
        "alignment-baseline", "baseline-shift", "clip-path", "clip-rule", "color-interpolation",
        "color-interpolation-filters", "dominant-baseline", "fill", "fill-opacity", "fill-rule",
        "filter", "flood-color", "flood-opacity", "lighting-color", "marker", "marker-end",
        "marker-mid", "marker-start", "mask", "mask-type", "mix-blend-mode", "paint-order",
        "shape-rendering", "stop-color", "stop-opacity", "stroke", "stroke-dasharray",
        "stroke-dashoffset", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit",
        "stroke-opacity", "stroke-width", "text-anchor", "text-rendering", "vector-effect",
    ];
    let name = declaration.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    PROPERTIES.contains(&name.as_str())
}

#[derive(Default)]
struct StyleDeclarationParser {
    raw: Vec<RawDeclaration>,
}

impl<'i> cssparser::DeclarationParser<'i> for StyleDeclarationParser {
    type Declaration = ParsedDeclaration;
    type Error = StyleParseError;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        // keep the text of the value, whether Helium knows the property or not
        let state = input.state();
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let text = input.slice_from(start).trim();
        let (text, important) = split_important(text);
        self.raw.push(RawDeclaration { name: name.to_string(), value: text.to_owned(), important });
        input.reset(&state);
        // the value ends at `!important`
        let declaration =
            input.parse_until_before(cssparser::Delimiter::Bang, |input| parse_property(name, input))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        Ok(ParsedDeclaration { declaration, important })
    }
}

/// Splits `!important` off the end of the text of a value.
fn split_important(text: &str) -> (&str, bool) {
    let Some(bang) = text.rfind('!') else {
        return (text, false);
    };
    if text[bang + 1..].trim().eq_ignore_ascii_case("important") {
        (text[..bang].trim_end(), true)
    } else {
        (text, false)
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
    if is_non_negative_property(&name) {
        // a negative length is not valid there: the whole declaration is dropped
        let state = input.state();
        let mut negative = false;
        while let Ok(token) = input.next() {
            use cssparser::Token;
            negative |= matches!(token,
                Token::Dimension { value, .. } | Token::Percentage { unit_value: value, .. } | Token::Number { value, .. } if *value < 0.0);
        }
        input.reset(&state);
        if negative {
            return Err(ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue) });
        }
    }
    PropertyDeclaration::parse_declaration(name, input)
}

/// The properties whose lengths must not be negative.
fn is_non_negative_property(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "padding"
        || name == "border-width"
        || name.starts_with("padding-")
        || (name.starts_with("border-") && name.ends_with("-width"))
        || matches!(
            name.trim_start_matches("min-").trim_start_matches("max-"),
            "width" | "height" | "inline-size" | "block-size"
        )
}

impl<'i> AtRuleParser<'i> for StyleDeclarationParser {
    type Prelude = ();
    type AtRule = ParsedDeclaration;
    type Error = StyleParseError;
}

impl<'i> QualifiedRuleParser<'i> for StyleDeclarationParser {
    type Prelude = ();
    type QualifiedRule = ParsedDeclaration;
    type Error = StyleParseError;
}

impl<'i> RuleBodyItemParser<'i, ParsedDeclaration, StyleParseError>
    for StyleDeclarationParser
{
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}
