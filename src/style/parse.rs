//! This module provides CSS declaration parsing using the `cssparser` and `selectors` crates.

use super::font_face::{FontFaceRule, parse_font_face};
use super::media::MediaEnv;
use crate::layout::text::font::FontContext;
use super::properties::custom::{self, CustomProperties};
use super::properties::specified::{
    CssWideDeclaration, CustomDeclaration, Font, ParseDeclaration, PropertyDeclaration,
    UnparsedDeclaration,
};
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
            // relative URLs in the style sheet are relative to the style sheet
            Ok(css) => {
                let sheet_dir = path.parent().unwrap_or(base_dir);
                crate::style::url_context::with(sheet_dir, base_dir, || self.parse_css_in(&css, sheet_dir))
            }
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
    LowerGreek,
    /// ა ბ გ … (1 to 19999)
    Georgian,
    /// ٠ ١ ٢ ٣ …
    ArabicIndic,
    /// The extended Arabic-Indic digits of Persian and Urdu: ۰ ۱ ۲ ۳ …
    Persian,
    /// The symbols of unordered lists, the same for every value.
    Disc,
    Circle,
    Square,
    /// No text at all.
    None,
}

impl CounterStyle {
    /// The style that a keyword (`decimal`, `lower-roman`, ...) names.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match_ignore_ascii_case! { name,
            "decimal" => CounterStyle::Decimal,
            "decimal-leading-zero" => CounterStyle::DecimalLeadingZero,
            "lower-roman" => CounterStyle::LowerRoman,
            "upper-roman" => CounterStyle::UpperRoman,
            "lower-alpha" | "lower-latin" => CounterStyle::LowerAlpha,
            "upper-alpha" | "upper-latin" => CounterStyle::UpperAlpha,
            "lower-greek" => CounterStyle::LowerGreek,
            "georgian" => CounterStyle::Georgian,
            "arabic-indic" => CounterStyle::ArabicIndic,
            "persian" => CounterStyle::Persian,
            "disc" => CounterStyle::Disc,
            "circle" => CounterStyle::Circle,
            "square" => CounterStyle::Square,
            "none" => CounterStyle::None,
            _ => return None,
        })
    }
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

/// The value of `-helium-page-label`, the label of the page in the PDF (what the page box of a
/// viewer shows). There is no CSS property for this.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageLabelSpec {
    /// The label is the number that the footer or else the header of the page prints.
    #[default]
    Auto,
    /// The label is the number of the page in decimal digits.
    None,
    /// A prefix and the number in a style, written like `content`: `"A-" counter(page, upper-roman)`.
    /// The text after `counter(page)` is not part of the label, and no other counter can come
    /// before it.
    Content(Vec<ContentItem>),
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
    /// `-helium-page-label` of an `@page` rule.
    pub page_label: Option<PageLabelSpec>,
}

impl QualifiedRule {
    pub fn new(selectors: SelectorList<Selectors>, properties: PropertyDeclarationBlock) -> Self {
        QualifiedRule {
            selectors: GeneralizedSelector::Selectors(selectors),
            properties,
            margin_boxes: vec![],
            page_label: None,
        }
    }

    pub fn matches(&self, elem: &ElementRef) -> bool {
        self.matching_specificity(elem).is_some()
    }

    /// The specificity of the most specific selector of this rule that matches `elem`, or `None`
    /// if the rule does not match. Used to order the cascade: rules with higher specificity are
    /// applied later and win over earlier declarations of the same property.
    pub fn matching_specificity(&self, elem: &ElementRef) -> Option<u32> {
        self.matching_specificity_of(elem, &[])
    }

    /// Like [`matching_specificity`](Self::matching_specificity) for the pseudo-element `pseudo`
    /// of `elem`: only the selectors that end in that pseudo-element are tried, and they are
    /// matched against the originating element `elem`.
    pub fn matching_pseudo_specificity(&self, elem: &ElementRef, pseudo: PseudoElement) -> Option<u32> {
        self.matching_specificity_of(elem, &[pseudo])
    }

    /// Like [`matching_pseudo_specificity`](Self::matching_pseudo_specificity) for a pseudo-element
    /// of a pseudo-element: the `::marker` of the `::before` of `elem`, for `outer` `Before` and
    /// `inner` `Marker`.
    pub fn matching_nested_pseudo_specificity(
        &self,
        elem: &ElementRef,
        outer: PseudoElement,
        inner: PseudoElement,
    ) -> Option<u32> {
        // (the pseudo-elements of a selector are listed from the right)
        self.matching_specificity_of(elem, &[inner, outer])
    }

    /// The specificity for the pseudo-elements `pseudos` (the rightmost first), none for the
    /// element itself.
    fn matching_specificity_of(&self, elem: &ElementRef, pseudos: &[PseudoElement]) -> Option<u32> {
        let GeneralizedSelector::Selectors(list) = &self.selectors else {
            return None;
        };
        let mut caches = SelectorCaches::default();
        let mut context = matching::MatchingContext::new(
            if !pseudos.is_empty() {
                matching::MatchingMode::ForStatelessPseudoElement
            } else {
                matching::MatchingMode::Normal
            },
            None,
            &mut caches,
            QuirksMode::NoQuirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        list.slice()
            .iter()
            // (a selector with other pseudo-elements, `::before::marker`, is not for the one)
            .filter(|selector| {
                pseudos.is_empty() || selector.pseudo_elements().iter().copied().eq(pseudos.iter())
            })
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
        let mut body_parser = StyleBodyParser { raw: vec![], nesting: None };
        let mut block = Self::new();
        for item in RuleBodyParser::new(input, &mut body_parser) {
            match item {
                Ok(BodyItem::Declaration(declaration)) => block.declarations.push(declaration),
                // there are no nested rules without `nesting`
                Ok(BodyItem::Rules { .. }) => {}
                Err((error, text, _location)) => {
                    // the properties of SVG elements are for the SVG parser
                    if !is_svg_property(text) {
                        warn!("Error while parsing declaration {:?}: {:?}", text, error)
                    }
                }
            }
        }
        block.raw = body_parser.raw;
        block
    }

    fn is_empty(&self) -> bool {
        self.declarations.is_empty() && self.raw.is_empty()
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
    /// The selectors and their text, for the rules nested in the rule.
    type Prelude = (SelectorList<Selectors>, String);
    type QualifiedRule = Vec<TopLevelRule>;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let start = input.position();
        let selector_parser = SelectorParser::default();
        let selector_list = SelectorList::parse(&selector_parser, input, ParseRelative::No)
            .map_err(|err| err.into::<StyleParseError>())?;
        Ok((selector_list, input.slice_from(start).trim().to_owned()))
    }

    fn parse_block<'t>(
        &mut self,
        (selectors, text): Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        Ok(parse_style_block(selectors, &text, input, self.media, &self.base_dir))
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
            page_label: body.page_label,
        })])
    }
}

#[derive(Default)]
struct PageBody {
    properties: PropertyDeclarationBlock,
    content: Option<Vec<ContentItem>>,
    margin_boxes: Vec<MarginBoxRule>,
    page_label: Option<PageLabelSpec>,
}

enum PageBodyItem {
    Declaration(ParsedDeclaration),
    Content(Vec<ContentItem>),
    PageLabel(PageLabelSpec),
    MarginBox(MarginBoxRule),
}

/// Parses the contents of an `@page` rule or of a margin at-rule.
fn parse_page_body(input: &mut Parser) -> PageBody {
    let mut body = PageBody::default();
    for item in RuleBodyParser::new(input, &mut PageBodyParser) {
        match item {
            Ok(PageBodyItem::Declaration(decl)) => body.properties.declarations.push(decl),
            Ok(PageBodyItem::Content(content)) => body.content = Some(content),
            Ok(PageBodyItem::PageLabel(label)) => body.page_label = Some(label),
            Ok(PageBodyItem::MarginBox(margin_box)) => body.margin_boxes.push(margin_box),
            Err((error, text, _location)) => {
                warn!("Error while parsing declaration {:?}: {:?}", text, error)
            }
        }
    }
    body
}

/// `auto | none | <string>* counter(page [, <counter-style>])`
fn parse_page_label<'i>(input: &mut Parser<'i>) -> Result<PageLabelSpec, ParseError<StyleParseError>> {
    if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        input.expect_exhausted()?;
        return Ok(PageLabelSpec::Auto);
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        input.expect_exhausted()?;
        return Ok(PageLabelSpec::None);
    }
    let items = parse_content(input)?;
    // a prefix of strings, then the number of the page: nothing else makes a label
    let numbered = items.iter().position(|item| matches!(item, ContentItem::Counter(..)));
    let valid = numbered.is_some_and(|at| {
        items[..at].iter().all(|item| matches!(item, ContentItem::String(_)))
            && matches!(items[at], ContentItem::Counter(CounterName::Page, _))
    });
    if !valid {
        return Err(ParseError { kind: cssparser::ParseErrorKind::Custom(StyleParseError::InvalidValue) });
    }
    Ok(PageLabelSpec::Content(items))
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
                    let style = super::properties::generated::counter_style(input)?;
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
        if name.eq_ignore_ascii_case("-helium-page-label") {
            return Ok(PageBodyItem::PageLabel(parse_page_label(input)?));
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

/// The pseudo-elements that generate content (`::before` and `::after`) and the one of the
/// marker of a list item (`::marker`).
/// https://www.w3.org/TR/css-pseudo-4/#generated-content
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Marker,
}

impl ToCss for PseudoElement {
    fn to_css<W>(&self, dest: &mut W) -> std::fmt::Result
    where
        W: std::fmt::Write,
    {
        dest.write_str(match *self {
            PseudoElement::Before => "::before",
            PseudoElement::After => "::after",
            PseudoElement::Marker => "::marker",
        })
    }
}

impl selectors::parser::PseudoElement for PseudoElement {
    /// `::before::marker` and `::after::marker` are valid selectors.
    fn valid_after_before_or_after(&self) -> bool {
        matches!(self, PseudoElement::Marker)
    }

    /// `::before` and `::after` are the ones that `::marker` can follow.
    fn is_before_or_after(&self) -> bool {
        matches!(self, PseudoElement::Before | PseudoElement::After)
    }
}

#[derive(Default)]
struct SelectorParser;

impl<'i> selectors::Parser<'i> for SelectorParser {
    type Impl = Selectors;
    type Error = SelectorParseErrorKind;

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_pseudo_element(
        &self,
        name: CowRcStr<'i>,
    ) -> Result<PseudoElement, ParseError<SelectorParseErrorKind>> {
        // (`:before` and `:after` with a single colon are recognized by the selector parser)
        if name.eq_ignore_ascii_case("before") {
            Ok(PseudoElement::Before)
        } else if name.eq_ignore_ascii_case("after") {
            Ok(PseudoElement::After)
        } else if name.eq_ignore_ascii_case("marker") {
            Ok(PseudoElement::Marker)
        } else {
            Err(ParseError::custom(
                SelectorParseErrorKind::UnsupportedPseudoClassOrElement,
            ))
        }
    }

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

/// What is nested in a style rule: the style sheet of its `@media` and rules.
struct Nesting<'a> {
    /// The selectors of the rule that a nested rule refers to with `&`.
    selectors: &'a SelectorList<Selectors>,
    /// The text of the selectors.
    selector_text: &'a str,
    media: MediaEnv,
    base_dir: &'a Path,
}

/// An item of the body of a style rule.
enum BodyItem {
    Declaration(ParsedDeclaration),
    /// The rules of a nested rule or `@media`, and the declarations that were written (as text)
    /// before it.
    Rules { rules: Vec<TopLevelRule>, raw: Vec<RawDeclaration> },
}

/// Reads the body of a style rule: declarations and, if `nesting` is set, nested rules.
struct StyleBodyParser<'a> {
    raw: Vec<RawDeclaration>,
    nesting: Option<Nesting<'a>>,
}

impl StyleBodyParser<'_> {
    fn invalid<'i>(&self, input: &Parser<'i>) -> ParseError<StyleParseError> {
        ParseError::custom(StyleParseError::InvalidValue)
    }
}

/// Reads the body of a style rule. A rule inside of it is the rule of the selector and its
/// parent, so the declarations after a nested rule are a rule of their own: they come after it
/// in the cascade.
fn parse_style_block(
    selectors: SelectorList<Selectors>,
    selector_text: &str,
    input: &mut Parser,
    media: MediaEnv,
    base_dir: &Path,
) -> Vec<TopLevelRule> {
    let mut body_parser = StyleBodyParser {
        raw: vec![],
        nesting: Some(Nesting { selectors: &selectors, selector_text, media, base_dir }),
    };
    let mut rules = vec![];
    let mut block = PropertyDeclarationBlock::new();
    for item in RuleBodyParser::new(input, &mut body_parser) {
        match item {
            Ok(BodyItem::Declaration(declaration)) => block.declarations.push(declaration),
            Ok(BodyItem::Rules { rules: nested, raw }) => {
                block.raw = raw;
                if !block.is_empty() {
                    let block = std::mem::take(&mut block);
                    rules.push(TopLevelRule::Style(QualifiedRule::new(selectors.clone(), block)));
                }
                rules.extend(nested);
            }
            Err((error, text, _location)) => {
                if !is_svg_property(text) {
                    warn!("Error while parsing declaration {:?}: {:?}", text, error)
                }
            }
        }
    }
    block.raw = body_parser.raw;
    if !block.is_empty() || rules.is_empty() {
        rules.push(TopLevelRule::Style(QualifiedRule::new(selectors, block)));
    }
    rules
}

impl<'i> cssparser::DeclarationParser<'i> for StyleBodyParser<'_> {
    type Declaration = BodyItem;
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
        let mut has_block = false;
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            has_block |= matches!(token, cssparser::Token::CurlyBracketBlock);
        }
        let text = input.slice_from(start).trim();
        let is_custom = custom::is_custom_property(&name);
        // The value of a property cannot hold a block. What looks like a declaration and has one
        // is a nested rule: `a:hover { ... }`.
        if has_block && !is_custom {
            return Err(ParseError::custom(StyleParseError::InvalidValue));
        }
        let (text, important) = split_important(text);
        if is_custom {
            self.raw.push(RawDeclaration { name: name.to_string(), value: text.to_owned(), important });
            let declaration = PropertyDeclaration::Custom(CustomDeclaration {
                name: name.to_string(),
                value: text.to_owned(),
            });
            return Ok(BodyItem::Declaration(ParsedDeclaration { declaration, important }));
        }
        if custom::contains_var(text) {
            // The property must be known. A value that is empty or not valid does tell
            // (it is not an unknown property that fails), the real one is only parsed once the
            // custom properties are.
            if let Err(error) = Parser::new("").parse_entirely(|input| parse_property(name.clone(), input)) {
                if matches!(error.kind, cssparser::ParseErrorKind::Custom(StyleParseError::UnknownProperty { .. })) {
                    return Err(ParseError::custom(StyleParseError::UnknownProperty { name: name.to_string() }));
                }
            }
            self.raw.push(RawDeclaration { name: name.to_string(), value: text.to_owned(), important });
            let declaration = PropertyDeclaration::Unparsed(UnparsedDeclaration {
                name: name.to_string(),
                value: text.to_owned(),
            });
            return Ok(BodyItem::Declaration(ParsedDeclaration { declaration, important }));
        }
        self.raw.push(RawDeclaration { name: name.to_string(), value: text.to_owned(), important });
        input.reset(&state);
        // the value ends at `!important`
        let declaration =
            input.parse_until_before(cssparser::Delimiter::Bang, |input| parse_property(name, input))?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        Ok(BodyItem::Declaration(ParsedDeclaration { declaration, important }))
    }
}

/// The declaration of a property that uses `var()`, with the custom properties of the element
/// substituted. If the value is not valid, the property is `unset`, as CSS prescribes (`None`
/// if that is not possible either).
pub fn resolve_unparsed(
    unparsed: &UnparsedDeclaration,
    custom: &CustomProperties,
) -> Option<PropertyDeclaration<'static>> {
    let parsed = custom::substitute(&unparsed.value, &mut |name| custom.get(name).map(str::to_owned))
        .ok_or("a custom property is missing")
        .and_then(|text| {
            Parser::new(&text)
                .parse_entirely(|input| parse_property(CowRcStr::from(unparsed.name.as_str()), input))
                .map_err(|_| "not a valid value")
        });
    match parsed {
        Ok(declaration) => Some(declaration),
        Err(reason) => {
            warn!("Invalid `{}: {}` ({})", unparsed.name, unparsed.value, reason);
            Parser::new("unset")
                .parse_entirely(|input| Ok::<_, ParseError<()>>(CssWideDeclaration::parse(&unparsed.name, input).ok_or(())))
                .ok()
                .and_then(Result::ok)
                .map(PropertyDeclaration::CssWide)
        }
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
pub(crate) fn parse_property<'i>(
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
    let range = calc_range(&name);
    super::properties::calc::with_range(range, || PropertyDeclaration::parse_declaration(name, input))
}

/// The range that a math function in the value of a property is clamped to: the values of
/// the properties that do not allow negative lengths or numbers (as literals, these are not
/// valid; a math function that is out of range is clamped instead). The shorthands are
/// included if all their longhands are such properties (`font` is `font-size` and
/// `line-height` for the math functions).
fn calc_range(name: &str) -> super::properties::calc::Range {
    use super::properties::calc::Range;
    let name = name.to_ascii_lowercase();
    let non_negative = is_non_negative_property(&name)
        || (name.starts_with("border-") && name.ends_with("radius"))
        || matches!(
            name.as_str(),
            "font" | "font-size" | "line-height" | "gap" | "row-gap" | "column-gap" | "grid-gap"
                | "grid-row-gap" | "grid-column-gap" | "columns" | "column-width" | "column-rule-width"
                | "border-spacing" | "background-size" | "flex" | "flex-basis" | "flex-grow"
                | "flex-shrink" | "outline-width" | "aspect-ratio"
        );
    if non_negative { Range::NonNegative } else { Range::All }
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

impl<'i> AtRuleParser<'i> for StyleBodyParser<'_> {
    /// Whether the media queries match: only `@media` can be nested.
    type Prelude = bool;
    type AtRule = BodyItem;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        match &self.nesting {
            Some(nesting) if name.eq_ignore_ascii_case("media") => {
                let matches = super::media::matches(input, &nesting.media);
                // what is left of an invalid query is skipped with the rule
                while input.next().is_ok() {}
                Ok(matches)
            }
            _ => Err(ParseError::from_basic_kind(BasicParseErrorKind::AtRuleInvalid)),
        }
    }

    fn parse_block<'t>(
        &mut self,
        matches: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        let Some(nesting) = &self.nesting else {
            return Err(ParseError::custom(StyleParseError::InvalidValue));
        };
        // the declarations and rules of the block are the ones of the parent
        let rules = parse_style_block(
            nesting.selectors.clone(),
            nesting.selector_text,
            input,
            nesting.media,
            nesting.base_dir,
        );
        let rules = if matches { rules } else { vec![] };
        Ok(BodyItem::Rules { rules, raw: std::mem::take(&mut self.raw) })
    }
}

impl<'i> QualifiedRuleParser<'i> for StyleBodyParser<'_> {
    type Prelude = (SelectorList<Selectors>, String);
    type QualifiedRule = BodyItem;
    type Error = StyleParseError;

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let Some(nesting) = &self.nesting else {
            return Err(ParseError::custom(StyleParseError::InvalidValue));
        };
        let text = resolve_nesting(input, nesting.selector_text).ok_or_else(|| ParseError::custom(StyleParseError::InvalidValue))?;
        let selector_parser = SelectorParser::default();
        let selectors = SelectorList::parse(&selector_parser, &mut Parser::new(&text), ParseRelative::No)
            .map_err(|error| match error.kind {
                cssparser::ParseErrorKind::Custom(kind) => ParseError::custom(StyleParseError::from(kind)),
                cssparser::ParseErrorKind::Basic(_) => ParseError::custom(StyleParseError::InvalidValue),
            })?;
        Ok((selectors, text))
    }

    fn parse_block<'t>(
        &mut self,
        (selectors, text): Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        let Some(nesting) = &self.nesting else {
            return Err(ParseError::custom(StyleParseError::InvalidValue));
        };
        let rules = parse_style_block(selectors, &text, input, nesting.media, nesting.base_dir);
        Ok(BodyItem::Rules { rules, raw: std::mem::take(&mut self.raw) })
    }
}

impl<'i> RuleBodyItemParser<'i, BodyItem, StyleParseError> for StyleBodyParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        self.nesting.is_some()
    }
}

/// The selectors of a nested rule as the selectors of a rule of its own, in the text: `&` stands
/// for the selectors of the parent as `:is()` (which also gives the specificity that CSS
/// prescribes), and a selector without `&` is a descendant of it, or related to it by its first
/// combinator. https://www.w3.org/TR/css-nesting-1/#nest-selector
fn resolve_nesting(input: &mut Parser, parent: &str) -> Option<String> {
    fn copy(
        input: &mut Parser,
        parent: &str,
        out: &mut String,
        has_parent: &mut bool,
    ) -> Option<()> {
        use cssparser::Token;
        while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
            let (open, close) = match &token {
                Token::Delim('&') => {
                    *has_parent = true;
                    out.push_str(":is(");
                    out.push_str(parent);
                    out.push(')');
                    continue;
                }
                Token::Function(name) => {
                    cssparser::serialize_identifier(name, out).ok()?;
                    ("(", ")")
                }
                Token::ParenthesisBlock => ("(", ")"),
                Token::SquareBracketBlock => ("[", "]"),
                Token::CurlyBracketBlock => return None,
                token => {
                    ToCss::to_css(token, out).ok()?;
                    continue;
                }
            };
            out.push_str(open);
            input
                .parse_nested_block(|input| {
                    copy(input, parent, out, has_parent).ok_or_else(|| ParseError::<()>::custom(()))
                })
                .ok()?;
            out.push_str(close);
        }
        Some(())
    }

    let mut result = String::new();
    loop {
        let mut selector = String::new();
        let mut has_parent = false;
        // the selector ends at a comma of the list
        input
            .parse_until_before(cssparser::Delimiter::Comma, |input| {
                copy(input, parent, &mut selector, &mut has_parent)
                    .ok_or_else(|| ParseError::<()>::custom(()))
            })
            .ok()?;
        let selector = selector.trim();
        if selector.is_empty() {
            return None;
        }
        if !result.is_empty() {
            result.push_str(", ");
        }
        if !has_parent {
            result.push_str(":is(");
            result.push_str(parent);
            result.push_str(") ");
        }
        result.push_str(selector);
        if input.next().is_err() {
            return Some(result);
        }
    }
}
