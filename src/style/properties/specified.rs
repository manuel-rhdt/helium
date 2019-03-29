use std::str::FromStr;

use cssparser::{CowRcStr, ParseError, Parser, Token};
use from_css_decl_derive::{FromCssDecl, PropertyValue};

pub trait FromCssDecl<'i>: Sized {
    type Error: 'i;
    type Computed: 'static;
    fn from_css_decl(name: CowRcStr<'i>, input: &mut Parser<'i, '_>) -> Result<Self, Self::Error>;

    fn apply_to(&self, computed: &mut Self::Computed);
}

pub trait PropertyValue<'i>: Sized {
    type Error: 'i;
    fn from_css_property_value(input: &mut Parser<'i, '_>) -> Result<Self, Self::Error>;

    fn computed_value<T>(&self) -> T
    where
        T: ComputedValue<Self>,
    {
        T::from_specified(self)
    }
}

pub trait ComputedValue<T>: Sized {
    fn from_specified(specified_value: &T) -> Self;
}

impl<Specified, Computed> ComputedValue<Specified> for Computed
where
    Computed: From<Specified>,
    Specified: Clone,
{
    fn from_specified(specified_value: &Specified) -> Computed {
        specified_value.clone().into()
    }
}

#[derive(Debug, Clone, FromCssDecl)]
pub enum PropertyDeclaration {
    Position(Position),
    Font(Font),
    Box(Box),
    Text(Text),
}

#[derive(Debug, Clone, FromCssDecl)]
#[computed = "super::computed::Position"]
pub enum Position {
    #[property]
    Width(AutoOrQuantityOrPercentage),
    #[property]
    Height(AutoOrQuantityOrPercentage),

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
}

#[derive(Debug, Clone, FromCssDecl)]
#[computed = "super::computed::Font"]
pub enum Font {
    #[property]
    FontSize(FontSize),
}

#[derive(Debug, Clone, FromCssDecl)]
#[computed = "super::computed::Box"]
pub enum Box {
    #[property]
    Display(Display),
}

#[derive(Debug, Clone, FromCssDecl)]
#[computed = "super::computed::Text"]
pub enum Text {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    WhiteSpace(WhiteSpace),
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

#[derive(Debug, Copy, Clone)]
pub struct Quantity {
    pub value: super::CssFloat,
    pub unit: Unit,
}

impl<'i> PropertyValue<'i> for Quantity {
    type Error = ParseError<'i, ()>;

    fn from_css_property_value(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
        let token = input.next()?;
        let (value, unit) = match token {
            Token::Dimension { value, unit, .. } => (*value, unit),
            _ => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token))
            },
        };

        let unit = match unit.parse() {
            Ok(unit) => unit,
            Err(_) => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token))
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
    type Error = ParseError<'i, ()>;

    fn from_css_property_value(input: &mut Parser<'i, '_>) -> Result<Self, ParseError<'i, ()>> {
        let token = input.next()?;
        match token {
            Token::Percentage { unit_value, .. } => Ok(Percentage(*unit_value)),
            _ => {
                let token = token.clone();
                Err(input.new_unexpected_token_error(token))
            },
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

impl From<BorderWidth> for super::computed::Length {
    fn from(bw: BorderWidth) -> Self {
        use super::computed::Length;

        match bw {
            BorderWidth::Thin => Length::from_val_unit(1.0, Unit::Pt),
            BorderWidth::Medium => Length::from_val_unit(2.0, Unit::Pt),
            BorderWidth::Thick => Length::from_val_unit(5.0, Unit::Px),
            BorderWidth::Quantity(Quantity { value, unit }) => Length::from_val_unit(value, unit),
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum FontSize {
    Quantity(Quantity),
}

impl ComputedValue<FontSize> for super::computed::PixelLength {
    fn from_specified(specified_value: &FontSize) -> Self {
        use super::computed::Length;

        match *specified_value {
            FontSize::Quantity(Quantity { value, unit }) => {
                match Length::from_val_unit(value, unit) {
                    Length::Absolute(abs) => abs,
                    _ => unimplemented!(),
                }
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
