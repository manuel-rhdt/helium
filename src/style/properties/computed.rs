use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};
use std::sync::Arc;

use super::specified::PropertyDeclaration;

pub type CssFloat = f32;

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct PixelLength(pub CssFloat);

impl PixelLength {
    pub fn from_pt(pt: f32) -> Self {
        PixelLength(pt / 0.75)
    }

    pub fn max(self, other: PixelLength) -> PixelLength {
        PixelLength(self.0.max(other.0))
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

/// A value between 0 and 1 which is interpreted as a relative size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub CssFloat);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Length {
    Absolute(PixelLength),
    Em(CssFloat),
}

impl Length {
    #[inline]
    pub fn from_val_unit(val: f32, unit: super::specified::Unit) -> Self {
        use super::specified::Unit;

        match unit {
            Unit::Px => Length::Absolute(PixelLength(val)),
            Unit::Cm => Length::from_val_unit(val / 2.54, Unit::In),
            Unit::Pt => Length::Absolute(PixelLength::from_pt(val)),
            Unit::Em => Length::Em(val),
            Unit::Mm => Length::from_val_unit(0.1 * val, Unit::Cm),
            Unit::In => Length::from_val_unit(val * 72.0, Unit::Pt),
        }
    }
}

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

    pub fn absolute(self, container_size: PixelLength, font_size: PixelLength) -> PixelLength {
        match self {
            LengthPercentage::Length(Length::Absolute(abs)) => abs,
            LengthPercentage::Length(Length::Em(em)) => font_size * em,
            LengthPercentage::Percentage(percentage) => container_size * percentage,
            _ => unimplemented!(),
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
                LengthPercentage::Length(Length::from_val_unit(value, unit)),
            ),
            QuantityOrPercentage::Percentage(percentage) => {
                GenericSize::LengthPercentage(LengthPercentage::Percentage(percentage))
            }
        }
    }

    pub fn absolute(
        self,
        if_auto: PixelLength,
        container_size: PixelLength,
        font_size: PixelLength,
    ) -> PixelLength {
        match self {
            GenericSize::Auto => if_auto,
            GenericSize::LengthPercentage(lp) => lp.absolute(container_size, font_size),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Position {
    pub width: GenericSize,
    pub height: GenericSize,
    pub margin_top: GenericSize,
    pub margin_bottom: GenericSize,
    pub margin_left: GenericSize,
    pub margin_right: GenericSize,
    pub padding_top: GenericSize,
    pub padding_bottom: GenericSize,
    pub padding_left: GenericSize,
    pub padding_right: GenericSize,
    pub border_top_width: GenericSize,
    pub border_bottom_width: GenericSize,
    pub border_left_width: GenericSize,
    pub border_right_width: GenericSize,
}

/// Contains the style structs.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>,
    pub font: Arc<Font>,
    pub text: Arc<Text>,
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    inherited_values: ComputedValues,
    position: Option<Position>,
    box_: Option<Box>,
    font: Option<Font>,
    text: Option<Text>,
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

    fn mutate_text(&mut self, f: impl FnOnce(&mut Text)) {
        let mut text = self
            .text
            .take()
            .unwrap_or_else(|| Text::clone(&*self.inherited_values.text));
        f(&mut text);
        self.text = Some(text);
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
            PropertyDeclaration::FontSize(_) => unimplemented!(),
            PropertyDeclaration::MarginTop(val) => {
                self.mutate_position(|mut pos| pos.margin_top = GenericSize::from_specified(val))
            }
            PropertyDeclaration::MarginBottom(val) => {
                self.mutate_position(|mut pos| pos.margin_bottom = GenericSize::from_specified(val))
            }
            PropertyDeclaration::MarginLeft(val) => {
                self.mutate_position(|mut pos| pos.margin_left = GenericSize::from_specified(val))
            }
            PropertyDeclaration::MarginRight(val) => {
                self.mutate_position(|mut pos| pos.margin_right = GenericSize::from_specified(val))
            }
            PropertyDeclaration::PaddingTop(val) => {
                self.mutate_position(|mut pos| pos.padding_top = GenericSize::from_specified(val))
            }
            PropertyDeclaration::PaddingBottom(val) => self
                .mutate_position(|mut pos| pos.padding_bottom = GenericSize::from_specified(val)),
            PropertyDeclaration::PaddingLeft(val) => {
                self.mutate_position(|mut pos| pos.padding_left = GenericSize::from_specified(val))
            }
            PropertyDeclaration::PaddingRight(val) => {
                self.mutate_position(|mut pos| pos.padding_right = GenericSize::from_specified(val))
            }
            PropertyDeclaration::BorderTopWidth(val) => self
                .mutate_position(|mut pos| pos.border_top_width = GenericSize::from_specified(val)),
            PropertyDeclaration::BorderBottomWidth(val) => self.mutate_position(|mut pos| {
                pos.border_bottom_width = GenericSize::from_specified(val)
            }),
            PropertyDeclaration::BorderLeftWidth(val) => self.mutate_position(|mut pos| {
                pos.border_left_width = GenericSize::from_specified(val)
            }),
            PropertyDeclaration::BorderRightWidth(val) => self.mutate_position(|mut pos| {
                pos.border_right_width = GenericSize::from_specified(val)
            }),
            PropertyDeclaration::WhiteSpace(val) => self.mutate_text(|mut text| {
                text.white_space = val
            })
        }
    }

    pub fn build(self) -> ComputedValues {
        ComputedValues {
            position: self.position.map(Arc::new).unwrap_or_default(),
            box_: self
                .box_
                .map(Arc::new)
                .unwrap_or(self.inherited_values.box_),
            font: self
                .font
                .map(Arc::new)
                .unwrap_or(self.inherited_values.font),
            text: self
                .text
                .map(Arc::new)
                .unwrap_or(self.inherited_values.text),
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
    pub line_height: PixelLength,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            font_size: PixelLength::from_pt(12.0),
            line_height: PixelLength::from_pt(12.0) * 1.2,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Text {
    pub white_space: super::specified::WhiteSpace,
}
