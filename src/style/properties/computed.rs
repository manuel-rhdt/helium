use std::sync::Arc;

use super::super::parse::PropertyDeclaration;

pub type CssFloat = f32;

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct PixelLength(pub CssFloat);

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub CssFloat);

type Length = PixelLength;

/// A length, a percentage or a combination of both using `calc`.
///
/// See [https://drafts.csswg.org/css-values-4/#typedef-length-percentage](https://drafts.csswg.org/css-values-4/#typedef-length-percentage).
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LengthPercentage {
    Length(Length),
    Percentage(Percentage),
    Both {
        length: Length,
        percentage: Percentage,
    },
}

impl LengthPercentage {
    /// If the `LengthPercentage` contains an absolute length, return it.
    pub fn length(&self) -> Option<Length> {
        match self {
            LengthPercentage::Length(length) | LengthPercentage::Both { length, .. } => {
                Some(*length)
            }
            _ => None,
        }
    }

    /// If the `LengthPercentage` contains a percentage, return it.
    pub fn percentage(&self) -> Option<Percentage> {
        match self {
            LengthPercentage::Percentage(percentage)
            | LengthPercentage::Both { percentage, .. } => Some(*percentage),
            _ => None,
        }
    }
}

/// Used as the computed value for `width`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum GenericSize {
    LengthPercentage(LengthPercentage),
    Auto,
}

impl Default for GenericSize {
    fn default() -> Self {
        GenericSize::Auto
    }
}

#[derive(Debug, Clone, Default)]
pub struct Position {
    pub width: GenericSize
}

/// Contains the style structs.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    inherited_values: ComputedValues,
    position: Option<Position>,
    box_: Option<Box>
}

impl StyleBuilder {
    pub fn new(inherited_values: ComputedValues) -> Self {
        StyleBuilder {
            inherited_values,
            ..Default::default()
        }
    }

    fn mutate_position(&mut self, f: impl FnOnce(&mut Position)) {
        let mut position = self.position.take().unwrap_or_else(|| Position::clone(&*self.inherited_values.position));
        f(&mut position);
        self.position = Some(position);
    }

    fn mutate_box(&mut self, f: impl FnOnce(&mut Box)) {
        let mut box_ = self.box_.take().unwrap_or_else(|| Box::clone(&*self.inherited_values.box_));
        f(&mut box_);
        self.box_ = Some(box_);
    }

    pub fn add_declaration(&mut self, decl: PropertyDeclaration) {
        match decl {
            PropertyDeclaration::Width(val) => self.mutate_position(|mut pos| pos.width = GenericSize::LengthPercentage(LengthPercentage::Length(PixelLength(val)))),
            PropertyDeclaration::Display(val) => self.mutate_box(|mut box_| box_.display = val.into()),
            _ => {}
        }
    }

    pub fn build(self) -> ComputedValues {
        ComputedValues {
            position: match self.position {
                None => self.inherited_values.position,
                Some(pos) => Arc::new(pos)
            },
            box_: match self.box_ {
                None => self.inherited_values.box_,
                Some(box_) => Arc::new(box_)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Box {
    pub display: Display
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline
}

impl Default for Display {
    fn default() -> Display {
        Display::Inline
    }
}

impl From<super::specified::Display> for Display {
    fn from(specified: super::specified::Display) -> Display {
        match specified {
            super::specified::Display::Block => Display::Block,
            super::specified::Display::Inline => Display::Inline,
            _ => Display::default()
        }
    }
}