use std::str::FromStr;

use cssparser::Token;

#[derive(Debug, Copy, Clone)]
pub enum PropertyDeclaration {
    Width(QuantityOrPercentage),
    Height(QuantityOrPercentage),
    Display(Display),
    FontSize
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
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
}

impl FromStr for Unit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "px" => Ok(Unit::Px),
            "cm" => Ok(Unit::Cm),
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
                unit: unit.parse().unwrap(),
            },
            Token::Percentage { unit_value, .. } => {
                QuantityOrPercentage::Percentage(super::Percentage(*unit_value))
            }
            unexpected => return Err(unexpected.clone()),
        };
        Ok(result)
    }
}
