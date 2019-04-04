use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};
use std::sync::Arc;

pub use super::specified::{Display, FontFamily};
use super::specified::{self, BorderStyle, ParseDeclaration, Percentage, PropertyDeclaration};

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

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Length {
    Absolute(PixelLength),
    Em(CssFloat),
}

impl Default for Length {
    fn default() -> Self {
        Length::Absolute(PixelLength(0.0))
    }
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

    pub fn absolute(self, font_size: PixelLength) -> PixelLength {
        match self {
            Length::Absolute(abs) => abs,
            Length::Em(em) => font_size * em,
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

impl Default for LengthPercentage {
    fn default() -> Self {
        LengthPercentage::Length(Length::default())
    }
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
            LengthPercentage::Length(len) => len.absolute(font_size),
            LengthPercentage::Percentage(percentage) => container_size * percentage,
            _ => unimplemented!(),
        }
    }
}

impl super::ComputedValue<specified::QuantityOrPercentage> for LengthPercentage {
    fn from_specified(spec: &super::specified::QuantityOrPercentage) -> Self {
        use super::specified::{Quantity, QuantityOrPercentage};
        match spec {
            QuantityOrPercentage::Quantity(Quantity { value, unit }) => {
                LengthPercentage::Length(Length::from_val_unit(*value, *unit))
            }
            QuantityOrPercentage::Percentage(percentage) => {
                LengthPercentage::Percentage(*percentage)
            }
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

impl super::ComputedValue<specified::AutoOrQuantityOrPercentage> for GenericSize {
    fn from_specified(spec: &specified::AutoOrQuantityOrPercentage) -> Self {
        match spec {
            specified::AutoOrQuantityOrPercentage::Auto => GenericSize::Auto,
            specified::AutoOrQuantityOrPercentage::Percentage(percentage) => {
                GenericSize::LengthPercentage(LengthPercentage::Percentage(*percentage))
            }
            specified::AutoOrQuantityOrPercentage::Quantity(specified::Quantity {
                value,
                unit,
            }) => GenericSize::LengthPercentage(LengthPercentage::Length(Length::from_val_unit(
                *value, *unit,
            ))),
        }
    }
}

impl GenericSize {
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
    pub padding_top: LengthPercentage,
    pub padding_bottom: LengthPercentage,
    pub padding_left: LengthPercentage,
    pub padding_right: LengthPercentage,
    pub border_top_width: Length,
    pub border_bottom_width: Length,
    pub border_left_width: Length,
    pub border_right_width: Length,
    pub border_top_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub border_right_style: BorderStyle,
}

impl Position {
    fn fixup(&mut self) {
        if let BorderStyle::None = self.border_top_style {
            self.border_top_width = Length::default();
        }
        if let BorderStyle::None = self.border_bottom_style {
            self.border_bottom_width = Length::default();
        }
        if let BorderStyle::None = self.border_left_style {
            self.border_left_width = Length::default();
        }
        if let BorderStyle::None = self.border_right_style {
            self.border_right_width = Length::default();
        }
    }
}

/// Contains the style structs.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>,
    pub font: Arc<Font>,
    pub text: Arc<Text>,
    pub background: Arc<Background>,
}

impl ComputedValues {
    fn fixup(&mut self) {
        Arc::make_mut(&mut self.position).fixup()
    }
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    inherited_values: ComputedValues,
    new_values: ComputedValues,
}

// TODO: Consider using `Arc::make_mut`.
impl StyleBuilder {
    pub fn new(inherited_values: ComputedValues) -> Self {
        StyleBuilder {
            inherited_values,
            ..Default::default()
        }
    }

    pub fn add_declaration(&mut self, decl: PropertyDeclaration) {
        match decl {
            PropertyDeclaration::Box(decl) => {
                decl.apply_to(Arc::make_mut(&mut self.new_values.box_))
            }
            PropertyDeclaration::Font(decl) => {
                decl.apply_to(Arc::make_mut(&mut self.new_values.font))
            }
            PropertyDeclaration::Position(decl) => {
                decl.apply_to(Arc::make_mut(&mut self.new_values.position))
            }
            PropertyDeclaration::Text(decl) => {
                decl.apply_to(Arc::make_mut(&mut self.new_values.text))
            }
            PropertyDeclaration::Background(decl) => {
                decl.apply_to(Arc::make_mut(&mut self.new_values.background))
            }
        }
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
    pub font_size: PixelLength,
    pub line_height: LineHeight,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            font_family: vec![specified::FontFamily::SansSerif],
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

impl From<specified::LineHeight> for LineHeight {
    fn from(spec: specified::LineHeight) -> Self {
        use specified::PropertyValue;
        match spec {
            specified::LineHeight::Normal => LineHeight::Normal,
            specified::LineHeight::QuantityOrPercentage(qp) => {
                LineHeight::LengthPercentage(qp.computed_value())
            }
        }
    }
}
