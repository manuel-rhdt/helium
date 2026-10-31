use std::borrow::{Cow, ToOwned};
use std::str::FromStr;

use super::super::parse::StyleParseError;
use super::computed::PixelLength;
use cssparser::{match_ignore_ascii_case, CowRcStr, ParseError, Parser, Token};
use cssparser_color::Color;
use helium_derive::{ParseDeclaration, PropertyValue};

use math_render::unicode_math::Family as MathFamily2;

pub trait ParseDeclaration<'i> {
    type Error: 'i;
    fn parse_declaration(
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>>
    where
        Self: Sized;
}

pub trait ApplyToComputedValues {
    fn apply_to(&self, computed: &mut super::computed::ComputedValues);
}

pub trait PropertyValue<'i>: Sized {
    type Error: 'i;
    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>>
    where
        Self: Sized;

    fn computed_value<T>(&self, computed_values: &super::computed::ComputedValues) -> T
    where
        T: ComputedValue<Self>,
    {
        T::from_specified(self, computed_values)
    }
}

pub trait ComputedValue<T>: Sized {
    fn from_specified(
        specified_value: &T,
        computed_values: &super::computed::ComputedValues,
    ) -> Self;
}

impl<'i> PropertyValue<'i> for String {
    type Error = ();

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        Ok(input.expect_ident_or_string()?.to_string())
    }
}

impl<'i, T> PropertyValue<'i> for Vec<T>
where
    T: PropertyValue<'i>,
{
    type Error = T::Error;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        input.parse_entirely(|input| {
            let mut result = vec![];
            loop {
                if input.is_exhausted() {
                    break Ok(result);
                }
                match T::from_css_property_value(input) {
                    Ok(val) => result.push(val),
                    Err(err) => break Err(err),
                }
            }
        })
    }
}

impl<Specified, Computed> ComputedValue<Specified> for Computed
where
    Computed: From<Specified>,
    Specified: Clone,
{
    fn from_specified(
        specified_value: &Specified,
        _: &super::computed::ComputedValues,
    ) -> Computed {
        specified_value.clone().into()
    }
}

/// Holds any property declaration that is supported by helium, grouped
/// by style structs.
#[derive(Debug, Clone, ParseDeclaration)]
pub enum PropertyDeclaration<'a> {
    Position(Position<'a>),
    Font(Font),
    Box(Box),
    Text(Text),
    Background(Background),
    Page(Page),
    Math(Math),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Position"]
pub enum Position<'a> {
    #[property]
    Width(AutoOrQuantityOrPercentage),
    #[property]
    Height(AutoOrQuantityOrPercentage),

    /// Offsets of positioned boxes.
    /// https://www.w3.org/TR/CSS22/visuren.html#position-props
    #[property]
    Top(AutoOrQuantityOrPercentage),
    #[property]
    Right(AutoOrQuantityOrPercentage),
    #[property]
    Bottom(AutoOrQuantityOrPercentage),
    #[property]
    Left(AutoOrQuantityOrPercentage),

    #[property]
    #[shorthand("margin-top", method = "top")]
    #[shorthand("margin-bottom", method = "bottom")]
    #[shorthand("margin-left", method = "left")]
    #[shorthand("margin-right", method = "right")]
    /// https://www.w3.org/TR/CSS22/box.html#propdef-margin
    Margin(List<'a, AutoOrQuantityOrPercentage>),

    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-top
    #[property]
    MarginTop(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-bottom
    #[property]
    MarginBottom(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-left
    #[property]
    MarginLeft(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-right
    #[property]
    MarginRight(AutoOrQuantityOrPercentage),

    #[property]
    #[shorthand("padding-top", method = "top")]
    #[shorthand("padding-bottom", method = "bottom")]
    #[shorthand("padding-left", method = "left")]
    #[shorthand("padding-right", method = "right")]
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding
    Padding(List<'a, QuantityOrPercentage>),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-top
    #[property]
    PaddingTop(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-bottom
    #[property]
    PaddingBottom(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-left
    #[property]
    PaddingLeft(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-right
    #[property]
    PaddingRight(QuantityOrPercentage),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-top-width
    #[property]
    BorderTopWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-bottom-width
    #[property]
    BorderBottomWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-left-width
    #[property]
    BorderLeftWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-right-width
    #[property]
    BorderRightWidth(BorderWidth),
    #[property]
    #[shorthand("border-top-width", method = "top")]
    #[shorthand("border-bottom-width", method = "bottom")]
    #[shorthand("border-left-width", method = "left")]
    #[shorthand("border-right-width", method = "right")]
    BorderWidth(List<'a, BorderWidth>),

    #[property]
    BorderTopStyle(BorderStyle),
    #[property]
    BorderBottomStyle(BorderStyle),
    #[property]
    BorderLeftStyle(BorderStyle),
    #[property]
    BorderRightStyle(BorderStyle),
    #[property]
    #[shorthand("border-top-style", method = "top")]
    #[shorthand("border-bottom-style", method = "bottom")]
    #[shorthand("border-left-style", method = "left")]
    #[shorthand("border-right-style", method = "right")]
    BorderStyle(List<'a, BorderStyle>),

    #[property]
    BorderTopColor(Color),
    #[property]
    BorderBottomColor(Color),
    #[property]
    BorderLeftColor(Color),
    #[property]
    BorderRightColor(Color),
    #[property]
    #[shorthand("border-top-color", method = "top")]
    #[shorthand("border-bottom-color", method = "bottom")]
    #[shorthand("border-left-color", method = "left")]
    #[shorthand("border-right-color", method = "right")]
    BorderColor(List<'a, Color>),

    #[property]
    #[shorthand("border-top-width", method = "width")]
    #[shorthand("border-top-style", method = "style")]
    #[shorthand("border-top-color", method = "color")]
    BorderTop(List<'a, Border>),
    #[property]
    #[shorthand("border-bottom-width", method = "width")]
    #[shorthand("border-bottom-style", method = "style")]
    #[shorthand("border-bottom-color", method = "color")]
    BorderBottom(List<'a, Border>),
    #[property]
    #[shorthand("border-left-width", method = "width")]
    #[shorthand("border-left-style", method = "style")]
    #[shorthand("border-left-color", method = "color")]
    BorderLeft(List<'a, Border>),
    #[property]
    #[shorthand("border-right-width", method = "width")]
    #[shorthand("border-right-style", method = "style")]
    #[shorthand("border-right-color", method = "color")]
    BorderRight(List<'a, Border>),

    #[property]
    #[shorthand("border-top", method = "make_ref")]
    #[shorthand("border-bottom", method = "make_ref")]
    #[shorthand("border-left", method = "make_ref")]
    #[shorthand("border-right", method = "make_ref")]
    Border(List<'a, Border>),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Font"]
pub enum Font {
    #[property]
    FontFamily(CommaSeparated<FontFamily>),
    #[property]
    FontStyle(FontStyle),
    #[property]
    FontWeight(FontWeight),
    #[property]
    FontSize(FontSize),
    #[property]
    LineHeight(LineHeight),
    /// Inherited like the font properties, so it lives in the same style struct.
    #[property]
    Color(Color),
    /// Inherited, so it lives here as well.
    #[property]
    ListStyleType(ListStyleType),
    /// Inherited instead of propagated to the inline boxes of the descendants, which has the same
    /// effect for everything that is supported.
    #[property]
    TextDecoration(TextDecoration),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Box"]
pub enum Box {
    #[property]
    Display(Display),
    #[property]
    Position(PositionSpec),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Text"]
pub enum Text {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    WhiteSpace(WhiteSpace),
    #[property]
    TextIndent(QuantityOrPercentage),
    #[property]
    TextAlign(TextAlign),
    #[ua_property]
    LineBreak(LineBreak)
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Background"]
pub enum Background {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    BackgroundColor(Color),
    /// Only the `<color>` form of the shorthand is supported.
    #[property]
    #[shorthand("background-color", method = "clone")]
    Background(Color),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Page"]
pub enum Page {
    #[property]
    Orphans(usize),
    #[property]
    Widows(usize),
    #[property]
    PageBreakAfter(PageBreak),
    #[property]
    PageBreakBefore(PageBreak),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Math"]
pub enum Math {
    #[ua_property]
    MathVariant(MathFamily),
    #[ua_property]
    Form(MathOperatorForm),
    #[ua_property]
    OperatorIsStretchy(AutoBool),
    #[ua_property]
    MathStyle(MathStyle),
}


#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum Display {
    None,
    Block,
    Inline,
    InlineBlock,
}

impl Default for Display {
    fn default() -> Self {
        Display::Inline
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Unit {
    Px,
    Cm,
    Mm,
    Pt,
    Em,
    In,
}

impl Default for Unit {
    fn default() -> Self {
        Unit::Px
    }
}

impl FromStr for Unit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match_ignore_ascii_case! { s,
            "px" => Ok(Unit::Px),
            "cm" => Ok(Unit::Cm),
            "pt" => Ok(Unit::Pt),
            "em" => Ok(Unit::Em),
            "mm" => Ok(Unit::Mm),
            "in" => Ok(Unit::In),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct List<'a, T: 'a>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    elements: Cow<'a, [T]>,
}

impl<'a, 'i, T: PropertyValue<'i>> PropertyValue<'i> for List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    type Error = T::Error;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<T::Error>> {
        // TODO: Error if zero elements.
        Vec::<T>::from_css_property_value(input).map(|elements| List {
            elements: Cow::Owned(elements),
        })
    }
}

impl<'a, T> List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    fn make_ref(&'a self) -> Self {
        List {
            elements: Cow::Borrowed(&self.elements),
        }
    }
}

// TODO: Don't panic!
impl<'a, T: Copy + PropertyValue<'a>> List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    fn top(&self) -> T {
        match self.elements.len() {
            1 | 2 | 3 | 4 => self.elements[0],
            _ => panic!(),
        }
    }

    fn bottom(&self) -> T {
        match self.elements.len() {
            1 | 2 => self.elements[0],
            3 | 4 => self.elements[2],
            _ => panic!(),
        }
    }

    fn left(&self) -> T {
        match self.elements.len() {
            1 => self.elements[0],
            2 | 3 => self.elements[1],
            4 => self.elements[3],
            _ => panic!(),
        }
    }

    fn right(&self) -> T {
        match self.elements.len() {
            1 => self.elements[0],
            2 | 3 | 4 => self.elements[1],
            _ => panic!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommaSeparated<T> {
    elements: Vec<T>,
}

impl<'i, T: PropertyValue<'i>> PropertyValue<'i> for CommaSeparated<T> {
    type Error = T::Error;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<T::Error>> {
        input
            .parse_comma_separated(|input| T::from_css_property_value(input))
            .map(|elements| CommaSeparated { elements })
    }
}

impl<T: Clone> ComputedValue<CommaSeparated<T>> for Vec<T> {
    fn from_specified(
        comma_separated: &CommaSeparated<T>,
        _: &super::computed::ComputedValues,
    ) -> Self {
        comma_separated.elements.clone()
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Quantity {
    pub value: super::CssFloat,
    pub unit: Unit,
}

impl<'i> PropertyValue<'i> for Quantity {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<()>> {
        let token = input.next()?;
        let (value, unit) = match token {
            Token::Dimension { value, unit, .. } => (*value, unit.parse()),
            Token::Number {
                int_value: Some(val),
                ..
            } if *val == 0 => (0.0, Ok(Unit::Px)),
            _ => {
                let _token = token.clone();
                return Err(ParseError::unexpected_token());
            }
        };

        let unit = match unit {
            Err(_) => {
                let _token = token.clone();
                return Err(ParseError::unexpected_token());
            }
            Ok(unit) => unit,
        };

        Ok(Quantity { value, unit })
    }
}

/// A value between 0 and 1 which is interpreted as a relative size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub super::CssFloat);

impl<'i> PropertyValue<'i> for Percentage {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<()>> {
        let token = input.next()?;
        match token {
            Token::Percentage { unit_value, .. } => Ok(Percentage(*unit_value)),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum QuantityOrPercentage {
    Quantity(Quantity),
    Percentage(Percentage),
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum AutoOrQuantityOrPercentage {
    Auto,
    Quantity(Quantity),
    Percentage(Percentage),
}

impl Default for AutoOrQuantityOrPercentage {
    fn default() -> Self {
        AutoOrQuantityOrPercentage::Auto
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum BorderWidth {
    Thin,
    Medium,
    Thick,
    Quantity(Quantity),
}

impl Default for BorderWidth {
    fn default() -> BorderWidth {
        BorderWidth::Medium
    }
}

impl ComputedValue<BorderWidth> for super::computed::PixelLength {
    fn from_specified(bw: &BorderWidth, style: &super::computed::ComputedValues) -> Self {
        match bw {
            BorderWidth::Thin => PixelLength(1.0),
            BorderWidth::Medium => PixelLength(3.0),
            BorderWidth::Thick => PixelLength(5.0),
            &BorderWidth::Quantity(Quantity { value, unit }) => {
                PixelLength::from_val_unit(value, unit, style.font.font_size)
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum FontSize {
    Quantity(Quantity),
    Percentage(Percentage),
    Smaller,
    Larger,
}

impl ComputedValue<FontSize> for super::computed::PixelLength {
    fn from_specified(font_size: &FontSize, style: &super::computed::ComputedValues) -> Self {
        match *font_size {
            FontSize::Quantity(Quantity { value, unit }) => {
                PixelLength::from_val_unit(value, unit, style.font.font_size)
            }
            FontSize::Percentage(percentage) => percentage.0 * style.font.font_size,
            FontSize::Smaller => 0.8 * style.font.font_size,
            FontSize::Larger => 1.2 * style.font.font_size,
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum WhiteSpace {
    Normal,
}

impl Default for WhiteSpace {
    fn default() -> Self {
        WhiteSpace::Normal
    }
}

/// The lines of `text-decoration-line`. Styles and colors of the lines are not supported and
/// ignored.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct TextDecoration {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

impl TextDecoration {
    pub fn is_none(self) -> bool {
        self == TextDecoration::default()
    }
}

impl<'i> PropertyValue<'i> for TextDecoration {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut result = TextDecoration::default();
        // at least one component is required
        let mut token = input.next()?.clone();
        loop {
            if let Token::Ident(ident) = &token {
                match &*ident.to_ascii_lowercase() {
                    "underline" => result.underline = true,
                    "overline" => result.overline = true,
                    "line-through" => result.line_through = true,
                    // `none` and the unsupported style and color components
                    _ => {}
                }
            }
            if input.is_exhausted() {
                return Ok(result);
            }
            token = input.next()?.clone();
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum ListStyleType {
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    None,
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Disc
    }
}

impl ListStyleType {
    /// The text of the marker of the `ordinal`th (1-based) list item, without any spacing.
    pub fn marker_text(self, ordinal: usize) -> Option<String> {
        fn alpha(mut n: usize, first: u8) -> String {
            let mut letters = vec![];
            while n > 0 {
                n -= 1;
                letters.push(first + (n % 26) as u8);
                n /= 26;
            }
            letters.reverse();
            String::from_utf8(letters).unwrap()
        }
        fn roman(mut n: usize) -> String {
            const NUMERALS: [(usize, &str); 13] = [
                (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
                (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
            ];
            let mut out = String::new();
            for &(value, numeral) in &NUMERALS {
                while n >= value {
                    out.push_str(numeral);
                    n -= value;
                }
            }
            out
        }
        Some(match self {
            ListStyleType::Disc => "\u{2022}".to_string(),
            ListStyleType::Circle => "\u{25E6}".to_string(),
            ListStyleType::Square => "\u{25AA}".to_string(),
            ListStyleType::None => return None,
            // roman numerals and letters are only defined for positive numbers
            ListStyleType::Decimal => format!("{}.", ordinal),
            _ if ordinal == 0 => format!("{}.", ordinal),
            ListStyleType::LowerAlpha => format!("{}.", alpha(ordinal, b'a')),
            ListStyleType::UpperAlpha => format!("{}.", alpha(ordinal, b'A')),
            ListStyleType::LowerRoman => format!("{}.", roman(ordinal).to_lowercase()),
            ListStyleType::UpperRoman => format!("{}.", roman(ordinal)),
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum TextAlign {
    Left,
    Justify,
    Center,
    Right,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Left
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum BorderStyle {
    None,
    Solid,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::None
    }
}

impl<'i> PropertyValue<'i> for Color {
    type Error = StyleParseError;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        Color::parse(input).map_err(|error| match error.kind {
            cssparser::ParseErrorKind::Basic(kind) => ParseError::from_basic_kind(kind),
            cssparser::ParseErrorKind::Custom(()) => ParseError::unexpected_token(),
        })
    }
}

impl<'i> PropertyValue<'i> for f32 {
    type Error = StyleParseError;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        let token = input.next()?;
        match token {
            Token::Number { value, .. } => Ok(*value),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

impl<'i> PropertyValue<'i> for usize {
    type Error = StyleParseError;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        let token = input.next()?;
        match token {
            Token::Number {
                int_value: Some(val),
                ..
            } => Ok(*val as usize),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

impl<'i> PropertyValue<'i> for bool {
    type Error = StyleParseError;

    fn from_css_property_value(
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        let val = input.expect_ident_or_string()?;
        if val.eq_ignore_ascii_case("true") {
            Ok(true)
        } else if val.eq_ignore_ascii_case("false") {
            Ok(false)
        } else {
            let _val = val.clone();
            Err(ParseError::unexpected_token())
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum AutoBool {
    True,
    False,
    Auto,
}

impl Default for AutoBool {
    fn default() -> Self {
        AutoBool::Auto
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum Border {
    Width(BorderWidth),
    Style(BorderStyle),
    Color(Color),
}

impl<'a> List<'a, Border> {
    fn width(&self) -> BorderWidth {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Width(width) => Some(*width),
                _ => None,
            })
            .next()
            .unwrap_or_default()
    }

    fn style(&self) -> BorderStyle {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Style(style) => Some(*style),
                _ => None,
            })
            .next()
            .unwrap_or_default()
    }

    fn color(&self) -> Color {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Color(color) => Some(*color),
                _ => None,
            })
            .next()
            .unwrap_or(Color::CurrentColor)
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum LineHeight {
    QuantityOrPercentage(QuantityOrPercentage),
    Number(f32),
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl Default for FontStyle {
    fn default() -> FontStyle {
        FontStyle::Normal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontWeight {
    Normal,
    Bold,
}

impl Default for FontWeight {
    fn default() -> FontWeight {
        FontWeight::Normal
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum PageBreak {
    Auto,
    Always,
}

impl Default for PageBreak {
    fn default() -> Self {
        PageBreak::Auto
    }
}


#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathFamily {
    Normal,
    Italic,
    Bold,
    BoldItalic,
    Script,
    BoldScript,
    Fraktur,
    DoubleStruck,
    BoldFraktur,
    SansSerif,
    BoldSansSerif,
    SansSerifItalic,
    SansSerifBoldItalic,
    Monospace,
}

impl Default for MathFamily {
    fn default() -> Self {
        MathFamily::Normal
    }
}

impl From<MathFamily> for MathFamily2 {
    fn from(fam: MathFamily) -> MathFamily2 {
        match fam {
            MathFamily::Normal => MathFamily2::Normal,
            MathFamily::Italic => MathFamily2::Italics,
            MathFamily::Bold => MathFamily2::Bold,
            MathFamily::BoldItalic => MathFamily2::BoldItalics,
            MathFamily::Script => MathFamily2::Script,
            MathFamily::BoldScript => MathFamily2::BoldScript,
            MathFamily::Fraktur => MathFamily2::Fraktur,
            MathFamily::DoubleStruck => MathFamily2::DoubleStruck,
            MathFamily::BoldFraktur => MathFamily2::BoldFraktur,
            MathFamily::SansSerif => MathFamily2::SansSerif,
            MathFamily::BoldSansSerif => MathFamily2::SansSerifBold,
            MathFamily::SansSerifItalic => MathFamily2::SansSerifItalics,
            MathFamily::SansSerifBoldItalic => MathFamily2::SansSerifBoldItalics,
            MathFamily::Monospace => MathFamily2::Monospace,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum PositionSpec {
    Static,
    Absolute,
}

impl Default for PositionSpec {
    fn default() -> Self {
        PositionSpec::Static
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathOperatorForm {
    Auto,
    Prefix,
    Infix,
    Postfix,
}

impl Default for MathOperatorForm {
    fn default() -> MathOperatorForm {
        MathOperatorForm::Auto
    }
}


impl From<MathOperatorForm> for Option<math_render::mathmlparser::Form> {
    fn from(other: MathOperatorForm) -> Self {
        match other {
            MathOperatorForm::Auto => None,
            MathOperatorForm::Prefix => Some(math_render::mathmlparser::Form::Prefix),
            MathOperatorForm::Infix => Some(math_render::mathmlparser::Form::Infix),
            MathOperatorForm::Postfix => Some(math_render::mathmlparser::Form::Postfix),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathStyle {
    Inline,
    Display
}

impl Default for MathStyle {
    fn default() -> MathStyle {
        MathStyle::Inline
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum LineBreak {
    Normal,
    Force
}

impl Default for LineBreak {
    fn default() -> LineBreak {
        LineBreak::Normal
    }
}
