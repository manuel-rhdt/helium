use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};
use std::sync::Arc;

use super::specified::{
    self, BorderStyle, ComputedValue, ParseDeclaration, PropertyDeclaration,
};
pub use super::specified::{Display, FontFamily, FontStyle, FontWeight, Percentage};

pub use cssparser::RGBA;

pub type CssFloat = f32;

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct PixelLength(pub CssFloat);

impl PixelLength {
    pub fn from_pt(pt: f32) -> Self {
        PixelLength(pt / 0.75)
    }

    pub fn to_pt(self) -> f32 {
        self.0 * 0.75
    }

    pub fn max(self, other: PixelLength) -> PixelLength {
        PixelLength(self.0.max(other.0))
    }

    #[inline]
    pub fn from_val_unit(val: f32, unit: super::specified::Unit, font_size: PixelLength) -> Self {
        use super::specified::Unit;

        match unit {
            Unit::Px => Self(val),
            Unit::Cm => Self::from_val_unit(val / 2.54, Unit::In, font_size),
            Unit::Pt => Self::from_pt(val),
            Unit::Em => val * font_size,
            Unit::Mm => Self::from_val_unit(0.1 * val, Unit::Cm, font_size),
            Unit::In => Self::from_val_unit(val * 72.0, Unit::Pt, font_size),
        }
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

impl Mul<PixelLength> for f32 {
    type Output = PixelLength;
    fn mul(self, other: PixelLength) -> PixelLength {
        PixelLength(self * other.0)
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

/// A length, a percentage or a combination of both using `calc`.
///
/// See
/// [https://drafts.csswg.org/css-values-4/#typedef-length-percentage]
/// (https://drafts.csswg.org/css-values-4/#typedef-length-percentage).
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LengthPercentage {
    PixelLength(PixelLength),
    Percentage(Percentage),
    Both {
        length: PixelLength,
        percentage: Percentage,
    },
}

impl Default for LengthPercentage {
    fn default() -> Self {
        LengthPercentage::PixelLength(PixelLength::default())
    }
}

impl From<PixelLength> for LengthPercentage {
    fn from(px: PixelLength) -> LengthPercentage {
        LengthPercentage::PixelLength(px)
    }
}

impl From<Percentage> for LengthPercentage {
    fn from(percent: Percentage) -> LengthPercentage {
        LengthPercentage::Percentage(percent)
    }
}

impl LengthPercentage {
    /// If the `LengthPercentage` contains an absolute length, return it.
    pub fn length(&self) -> Option<PixelLength> {
        match self {
            LengthPercentage::PixelLength(length) | LengthPercentage::Both { length, .. } => {
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

    pub fn absolute(self, container_size: PixelLength) -> PixelLength {
        match self {
            LengthPercentage::PixelLength(len) => len,
            LengthPercentage::Percentage(percentage) => container_size * percentage,
            _ => unimplemented!(),
        }
    }
}

impl super::ComputedValue<specified::QuantityOrPercentage> for LengthPercentage {
    fn from_specified(
        spec: &super::specified::QuantityOrPercentage,
        style: &ComputedValues,
    ) -> Self {
        use super::specified::{Quantity, QuantityOrPercentage};
        match spec {
            QuantityOrPercentage::Quantity(Quantity { value, unit }) => {
                LengthPercentage::PixelLength(PixelLength::from_val_unit(
                    *value,
                    *unit,
                    style.font.font_size,
                ))
            }
            QuantityOrPercentage::Percentage(percentage) => (*percentage).into(),
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

impl From<PixelLength> for GenericSize {
    fn from(px: PixelLength) -> GenericSize {
        GenericSize::LengthPercentage(px.into())
    }
}

impl From<Percentage> for GenericSize {
    fn from(percent: Percentage) -> GenericSize {
        GenericSize::LengthPercentage(percent.into())
    }
}

impl super::ComputedValue<specified::AutoOrQuantityOrPercentage> for GenericSize {
    fn from_specified(
        spec: &specified::AutoOrQuantityOrPercentage,
        style: &ComputedValues,
    ) -> Self {
        match spec {
            specified::AutoOrQuantityOrPercentage::Auto => GenericSize::Auto,
            specified::AutoOrQuantityOrPercentage::Percentage(percentage) => (*percentage).into(),
            specified::AutoOrQuantityOrPercentage::Quantity(specified::Quantity {
                value,
                unit,
            }) => GenericSize::LengthPercentage(LengthPercentage::PixelLength(
                PixelLength::from_val_unit(*value, *unit, style.font.font_size),
            )),
        }
    }
}

impl GenericSize {
    pub fn absolute(self, if_auto: PixelLength, container_size: PixelLength) -> PixelLength {
        match self {
            GenericSize::Auto => if_auto,
            GenericSize::LengthPercentage(lp) => lp.absolute(container_size),
        }
    }
}

/// The style struct containing properties related to positioning
/// of boxes.
///
/// These properties are _not_ inherited.
#[derive(Debug, Clone, Default)]
pub struct Position {
    pub width: GenericSize,
    pub height: GenericSize,
    pub margin_top: GenericSize,
    pub margin_bottom: GenericSize,
    pub margin_left: GenericSize,
    pub margin_right: GenericSize,
    pub padding_top: LengthPercentage,
    pub padding_bottom: LengthPercentage,
    pub padding_left: LengthPercentage,
    pub padding_right: LengthPercentage,
    pub border_top_width: PixelLength,
    pub border_bottom_width: PixelLength,
    pub border_left_width: PixelLength,
    pub border_right_width: PixelLength,
    pub border_top_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub border_right_style: BorderStyle,
}

impl Position {
    fn fixup(&mut self) {
        if let BorderStyle::None = self.border_top_style {
            self.border_top_width = PixelLength::default();
        }
        if let BorderStyle::None = self.border_bottom_style {
            self.border_bottom_width = PixelLength::default();
        }
        if let BorderStyle::None = self.border_left_style {
            self.border_left_width = PixelLength::default();
        }
        if let BorderStyle::None = self.border_right_style {
            self.border_right_width = PixelLength::default();
        }
    }
}

use std::convert::AsMut;

/// Contains the style structs and encompasses all the style information for
/// an element.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>,
    pub font: Arc<Font>,
    pub text: Arc<Text>,
    pub background: Arc<Background>,
}

impl AsMut<Box> for ComputedValues {
    fn as_mut(&mut self) -> &mut Box {
        Arc::make_mut(&mut self.box_)
    }
}
impl AsMut<Position> for ComputedValues {
    fn as_mut(&mut self) -> &mut Position {
        Arc::make_mut(&mut self.position)
    }
}
impl AsMut<Font> for ComputedValues {
    fn as_mut(&mut self) -> &mut Font {
        Arc::make_mut(&mut self.font)
    }
}
impl AsMut<Text> for ComputedValues {
    fn as_mut(&mut self) -> &mut Text {
        Arc::make_mut(&mut self.text)
    }
}
impl AsMut<Background> for ComputedValues {
    fn as_mut(&mut self) -> &mut Background {
        Arc::make_mut(&mut self.background)
    }
}

impl ComputedValues {
    fn fixup(&mut self) {
        Arc::make_mut(&mut self.position).fixup()
    }
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    new_values: ComputedValues,
}

impl StyleBuilder {
    pub fn new(inherited_values: ComputedValues) -> Self {
        let inherited = ComputedValues {
            font: inherited_values.font.clone(),
            ..Default::default()
        };
        StyleBuilder {
            new_values: inherited,
        }
    }

    pub fn add_declaration(&mut self, decl: PropertyDeclaration) {
        decl.apply_to(&mut self.new_values)
    }

    pub fn build(mut self) -> ComputedValues {
        self.new_values.fixup();
        self.new_values
    }
}

#[derive(Debug, Clone, Default)]
pub struct Box {
    pub display: specified::Display,
}

#[derive(Debug, Clone)]
pub struct Font {
    pub font_family: Vec<specified::FontFamily>,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub font_size: PixelLength,
    pub line_height: LineHeight,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            font_family: vec![specified::FontFamily::SansSerif],
            font_style: FontStyle::default(),
            font_weight: FontWeight::default(),
            font_size: PixelLength::from_pt(12.0),
            line_height: LineHeight::default(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Text {
    pub white_space: specified::WhiteSpace,
}

#[derive(Debug, Clone)]
pub struct Background {
    pub background_color: cssparser::Color,
}

impl Default for Background {
    fn default() -> Self {
        Background {
            background_color: cssparser::Color::RGBA(RGBA::transparent()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LineHeight {
    Normal,
    LengthPercentage(LengthPercentage),
}

impl Default for LineHeight {
    fn default() -> Self {
        LineHeight::Normal
    }
}

impl ComputedValue<specified::LineHeight> for LineHeight {
    fn from_specified(lh: &specified::LineHeight, style: &super::computed::ComputedValues) -> Self {
        match lh {
            specified::LineHeight::Normal => LineHeight::Normal,
            specified::LineHeight::QuantityOrPercentage(qp) => {
                LineHeight::LengthPercentage(LengthPercentage::from_specified(qp, style))
            }
        }
    }
}
