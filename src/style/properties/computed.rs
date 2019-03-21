use std::sync::Arc;
use std::ops::{Add, AddAssign, Mul, Div, Sub, Neg};

use super::specified::PropertyDeclaration;

pub type CssFloat = f32;

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct PixelLength(pub CssFloat);

impl PixelLength {
    pub fn from_pt(pt: f32) -> Self {
        PixelLength(pt / 0.75)
    }
}

impl Add for PixelLength {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl AddAssign for PixelLength {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0
    }
}

impl Sub for PixelLength {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl Neg for PixelLength {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl Mul<Percentage> for PixelLength {
    type Output = Self;
    fn mul(self, other: Percentage) -> Self {
        Self(self.0 * other.0)
    }
}

impl Mul<f32> for PixelLength {
    type Output = Self;
    fn mul(self, other: f32) -> Self {
        Self(self.0 * other)
    }
}

impl Mul for PixelLength {
    type Output = Self;
    fn mul(self, other: PixelLength) -> Self {
        Self(self.0 * other.0)
    }
}

impl Div<f32> for PixelLength {
    type Output = Self;
    fn div(self, other: f32) -> Self {
        Self(self.0 / other)
    }
}

impl PixelLength {
    pub fn from_val_unit(val: f32, unit: super::specified::Unit) -> Self {
        match unit {
            super::specified::Unit::Px => PixelLength(val),
            super::specified::Unit::Cm => PixelLength::from_pt(val / 2.54 * 72.0),
            super::specified::Unit::Pt => PixelLength::from_pt(val)
        }
    }
}

/// A value between 0 and 1 which is interpreted as a relative size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub CssFloat);

type Length = PixelLength;

/// A length, a percentage or a combination of both using `calc`.
///
/// See
/// [https://drafts.csswg.org/css-values-4/#typedef-length-percentage]
/// (https://drafts.csswg.org/css-values-4/#typedef-length-percentage).
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

impl GenericSize {
    pub fn from_specified(spec: super::specified::QuantityOrPercentage) -> Self {
        use super::specified::QuantityOrPercentage;
        match spec {
            QuantityOrPercentage::Quantity { value, unit } => GenericSize::LengthPercentage(
                LengthPercentage::Length(PixelLength::from_val_unit(value, unit)),
            ),
            QuantityOrPercentage::Percentage(percentage) => {
                GenericSize::LengthPercentage(LengthPercentage::Percentage(percentage))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Position {
    pub width: GenericSize,
    pub height: GenericSize,
}

/// Contains the style structs.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>,
    pub font: Arc<Font>,
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    inherited_values: ComputedValues,
    position: Option<Position>,
    box_: Option<Box>,
    font: Option<Font>,
}

impl StyleBuilder {
    pub fn new(inherited_values: ComputedValues) -> Self {
        StyleBuilder {
            inherited_values,
            ..Default::default()
        }
    }

    fn mutate_position(&mut self, f: impl FnOnce(&mut Position)) {
        let mut position = self
            .position
            .take()
            .unwrap_or_else(|| Position::clone(&*self.inherited_values.position));
        f(&mut position);
        self.position = Some(position);
    }

    fn mutate_box(&mut self, f: impl FnOnce(&mut Box)) {
        let mut box_ = self
            .box_
            .take()
            .unwrap_or_else(|| Box::clone(&*self.inherited_values.box_));
        f(&mut box_);
        self.box_ = Some(box_);
    }

    pub fn add_declaration(&mut self, decl: PropertyDeclaration) {
        match decl {
            PropertyDeclaration::Width(specified) => {
                self.mutate_position(|mut pos| pos.width = GenericSize::from_specified(specified))
            }
            PropertyDeclaration::Height(specified) => {
                self.mutate_position(|mut pos| pos.height = GenericSize::from_specified(specified))
            }
            PropertyDeclaration::Display(val) => {
                self.mutate_box(|mut box_| box_.display = val.into())
            }
            PropertyDeclaration::FontSize => unimplemented!()
        }
    }

    pub fn build(self) -> ComputedValues {
        ComputedValues {
            position: self
                .position
                .map(Arc::new)
                .unwrap_or(self.inherited_values.position),
            box_: self
                .box_
                .map(Arc::new)
                .unwrap_or(self.inherited_values.box_),
            font: self
                .font
                .map(Arc::new)
                .unwrap_or(self.inherited_values.font),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Box {
    pub display: Display,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
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
            super::specified::Display::None => Display::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Font {
    pub font_size: PixelLength,
    pub line_height: PixelLength
}

impl Default for Font {
    fn default() -> Self {
        Self {
            font_size: PixelLength::from_pt(12.0),
            line_height: PixelLength::from_pt(12.0) * 1.2
        }
    }
}