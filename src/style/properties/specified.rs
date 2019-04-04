use std::str::FromStr;

use cssparser::{Color, CowRcStr, ParseError, Parser, Token};
use helium_derive::{ParseDeclaration, PropertyValue};

pub trait ParseDeclaration<'i> {
    type Error: 'i;
    type Computed: 'static;
    fn parse_declaration(
        name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, Self::Error>>
    where
        Self: Sized;

    fn apply_to(&self, computed: &mut super::computed::ComputedValues);
}

pub trait PropertyValue<'i>: Sized {
    type Error: 'i;
    fn from_css_property_value(
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, Self::Error>>;

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
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, Self::Error>> {
        Ok(input.expect_ident_or_string()?.to_string())
    }
}

impl<'i, T> PropertyValue<'i> for Vec<T>
where
    T: PropertyValue<'i>,
{
    type Error = T::Error;

    fn from_css_property_value(
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, Self::Error>> {
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
pub enum PropertyDeclaration {
    Position(Position),
    Font(Font),
    Box(Box),
    Text(Text),
    Background(Background),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Position"]
pub enum Position {
    #[property]
    Width(AutoOrQuantityOrPercentage),
    #[property]
    Height(AutoOrQuantityOrPercentage),

    #[property]
    #[shorthand("margin-top", method = "top")]
    #[shorthand("margin-bottom", method = "bottom")]
    #[shorthand("margin-left", method = "left")]
    #[shorthand("margin-right", method = "right")]
    /// https://www.w3.org/TR/CSS22/box.html#propdef-margin
    Margin(List<AutoOrQuantityOrPercentage>),

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
    Padding(List<QuantityOrPercentage>),

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
    BorderWidth(List<BorderWidth>),

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
    BorderStyle(List<BorderStyle>),

    #[property]
    #[shorthand("border-top-width", method = "width")]
    #[shorthand("border-top-style", method = "style")]
    BorderTop(List<Border>),
    #[property]
    #[shorthand("border-bottom-width", method = "width")]
    #[shorthand("border-bottom-style", method = "style")]
    BorderBottom(List<Border>),
    #[property]
    #[shorthand("border-left-width", method = "width")]
    #[shorthand("border-left-style", method = "style")]
    BorderLeft(List<Border>),
    #[property]
    #[shorthand("border-right-width", method = "width")]
    #[shorthand("border-right-style", method = "style")]
    BorderRight(List<Border>),

    #[property]
    #[shorthand("border-top", method = "clone")]
    #[shorthand("border-bottom", method = "clone")]
    #[shorthand("border-left", method = "clone")]
    #[shorthand("border-right", method = "clone")]
    Border(List<Border>),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Font"]
pub enum Font {
    #[property]
    FontFamily(CommaSeparated<FontFamily>),
    #[property]
    FontSize(FontSize),
    #[property]
    LineHeight(LineHeight),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Box"]
pub enum Box {
    #[property]
    Display(Display),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Text"]
pub enum Text {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    WhiteSpace(WhiteSpace),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Background"]
pub enum Background {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    BackgroundColor(Color),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum Display {
    None,
    Block,
    Inline,
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
        match s {
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
pub struct List<T> {
    elements: Vec<T>,
}

impl<'i, T: PropertyValue<'i>> PropertyValue<'i> for List<T> {
    type Error = T::Error;

    fn from_css_property_value(
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, T::Error>> {
        Vec::<T>::from_css_property_value(input).map(|elements| List { elements })
    }
}

impl<'a, T: Copy + PropertyValue<'a>> List<T> {
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
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, T::Error>> {
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

    fn from_css_property_value(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
        let token = input.next()?;
        let (value, unit) = match token {
            Token::Dimension { value, unit, .. } => (*value, unit),
            _ => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token));
            }
        };

        let unit = match unit.parse() {
            Ok(unit) => unit,
            Err(_) => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token));
            }
        };

        Ok(Quantity {
            value: value,
            unit: unit,
        })
    }
}

/// A value between 0 and 1 which is interpreted as a relative size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub super::CssFloat);

impl<'i> PropertyValue<'i> for Percentage {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
        let token = input.next()?;
        match token {
            Token::Percentage { unit_value, .. } => Ok(Percentage(*unit_value)),
            _ => {
                let token = token.clone();
                Err(input.new_unexpected_token_error(token))
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
        use super::computed::PixelLength;

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
}

impl ComputedValue<FontSize> for super::computed::PixelLength {
    fn from_specified(font_size: &FontSize, style: &super::computed::ComputedValues) -> Self {
        use super::computed::PixelLength;

        match *font_size {
            FontSize::Quantity(Quantity { value, unit }) => {
                PixelLength::from_val_unit(value, unit, style.font.font_size)
            }
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
    type Error = ();

    fn from_css_property_value(
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, Self::Error>> {
        Color::parse(input).map_err(Into::into)
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum Border {
    Width(BorderWidth),
    Style(BorderStyle),
}

impl List<Border> {
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
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum LineHeight {
    QuantityOrPercentage(QuantityOrPercentage),
    Normal,
}

#[derive(Debug, Clone, PropertyValue)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Named(String),
}
