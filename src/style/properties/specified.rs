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
    MarginRight(QuantityOrPercentage)
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
    Pt,
    Em,
}

impl FromStr for Unit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "px" => Ok(Unit::Px),
            "cm" => Ok(Unit::Cm),
            "pt" => Ok(Unit::Pt),
            "em" => Ok(Unit::Em),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum QuantityOrPercentage {
    Quantity { value: super::CssFloat, unit: Unit },
    Percentage(super::Percentage),
}

impl QuantityOrPercentage {
    pub fn parse<'a, 'b>(token: &'a Token<'b>) -> Result<Self, Token<'b>> {
        let result = match token {
            Token::Dimension { value, unit, .. } => QuantityOrPercentage::Quantity {
                value: *value,
                unit: unit.parse().map_err(|_| token.clone())?,
            },
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
