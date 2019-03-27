use std::str::FromStr;

use cssparser::{CowRcStr, ParseError, Parser, Token};
use from_css_decl_derive::FromCssDecl;

pub trait FromCssDecl<'i>: Sized {
    type Error: 'i;
    fn from_css_decl(name: CowRcStr<'i>, input: &mut Parser<'i, '_>) -> Result<Self, Self::Error>;
}

#[derive(Debug, Copy, Clone, FromCssDecl)]
pub enum PropertyDeclaration {
    Width(QuantityOrPercentage),
    Height(QuantityOrPercentage),
    Display(Display),
    FontSize(FontSize),

    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-top
    MarginTop(QuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-bottom
    MarginBottom(QuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-left
    MarginLeft(QuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-right
    MarginRight(QuantityOrPercentage),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-top-width
    BorderTopWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-bottom-width
    BorderBottomWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-left-width
    BorderLeftWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-right-width
    BorderRightWidth(BorderWidth),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-top
    PaddingTop(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-bottom
    PaddingBottom(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-left
    PaddingLeft(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-right
    PaddingRight(QuantityOrPercentage),

    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    WhiteSpace(WhiteSpace),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
}

impl<'i> FromCssDecl<'i> for Display {
    type Error = ParseError<'i, ()>;
    fn from_css_decl(
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        match input.next()? {
            Token::Ident(ident) => match ident.parse() {
                Ok(display) => Ok(display),
                Err(_) => {
                    let ident = ident.clone();
                    Err(input.new_unexpected_token_error(Token::Ident(ident)))
                }
            },
            unexpected => {
                let unexpected = unexpected.clone();
                Err(input.new_basic_unexpected_token_error(unexpected).into())
            }
        }
    }
}

impl FromStr for Display {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Display::None),
            "block" => Ok(Display::Block),
            "inline" => Ok(Display::Inline),
            _ => Err(()),
        }
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

#[derive(Debug, Copy, Clone)]
pub enum QuantityOrPercentage {
    Quantity(Quantity),
    Percentage(super::Percentage),
}

impl QuantityOrPercentage {
    pub fn parse<'a, 'b>(token: &'a Token<'b>) -> Result<Self, Token<'b>> {
        let result = match token {
            Token::Dimension { value, unit, .. } => QuantityOrPercentage::Quantity(Quantity {
                value: *value,
                unit: unit.parse().map_err(|_| token.clone())?,
            }),
            Token::Percentage { unit_value, .. } => {
                QuantityOrPercentage::Percentage(super::Percentage(*unit_value))
            }
            unexpected => return Err(unexpected.clone()),
        };
        Ok(result)
    }
}

impl<'i> FromCssDecl<'i> for QuantityOrPercentage {
    type Error = ParseError<'i, ()>;

    fn from_css_decl(
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        QuantityOrPercentage::parse(input.next()?).map_err(|unexpected_token| {
            input
                .new_basic_unexpected_token_error(unexpected_token)
                .into()
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub enum AutoOrQuantityOrPercentage {
    Auto,
    Quantity(Quantity),
    Percentage(super::Percentage),
}

impl Default for AutoOrQuantityOrPercentage {
    fn default() -> Self {
        AutoOrQuantityOrPercentage::Auto
    }
}

#[derive(Debug, Copy, Clone)]
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
            BorderWidth::Thick => Length::from_val_unit(4.0, Unit::Pt),
            BorderWidth::Quantity(Quantity { value, unit }) => Length::from_val_unit(value, unit),
        }
    }
}

impl<'i> FromCssDecl<'i> for BorderWidth {
    type Error = ParseError<'i, ()>;

    fn from_css_decl(
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        let token = input.next()?;
        match &token {
            Token::Ident(ident) => match ident.as_ref() {
                "thin" => return Ok(BorderWidth::Thin),
                "medium" => return Ok(BorderWidth::Medium),
                "thick" => return Ok(BorderWidth::Thick),
                _ => {}
            },
            Token::Dimension { value, unit, .. } => {
                return Ok(BorderWidth::Quantity(Quantity {
                    value: *value,
                    unit: unit.parse().unwrap_or_default(),
                }));
            }
            _ => {}
        }
        let token = token.clone();
        Err(input.new_unexpected_token_error(token))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum FontSize {}

impl<'i> FromCssDecl<'i> for FontSize {
    type Error = ParseError<'i, ()>;

    fn from_css_decl(
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        Err(input.new_custom_error(()))
    }
}

#[derive(Debug, Copy, Clone)]
pub enum WhiteSpace {
    Normal,
}

impl Default for WhiteSpace {
    fn default() -> Self {
        WhiteSpace::Normal
    }
}

impl<'i> FromCssDecl<'i> for WhiteSpace {
    type Error = ParseError<'i, ()>;

    fn from_css_decl(
        _name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        match input.next()?.clone() {
            Token::Ident(ident) => match ident.as_ref() {
                "normal" => return Ok(WhiteSpace::Normal),
                _ => Err(input.new_unexpected_token_error(Token::Ident(ident))),
            },
            unexpected => Err(input.new_unexpected_token_error(unexpected)),
        }
    }
}
