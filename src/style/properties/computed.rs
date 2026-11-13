use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};
use std::sync::Arc;

use super::specified::{
    self, ApplyToComputedValues, AutoBool, BorderStyle, ComputedValue, MathFamily, MathOperatorForm,
};
pub use super::specified::{
    BoxSizing, ContentAlignment, Display, FlexDirection, FlexWrap, FontFamily, FontStyle,
    GridAutoFlow, GridPlacement, ItemAlignment, MathDepth, MathShift, MathStyle, Break, BreakInside,
    Percentage, PositionSpec, SelfAlignment, TextAlign,
};

pub use cssparser_color::Color;

/// The sRGB colour a CSS colour is painted with, see [`super::color`].
pub use super::color::color_to_rgba;

/// An sRGB color with 8 bits per channel.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub struct RGBA {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl RGBA {
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        RGBA {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub const fn transparent() -> Self {
        RGBA::new(0, 0, 0, 0)
    }
}

/// An opaque sRGB color.
pub fn opaque(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgba(cssparser_color::RgbaLegacy::new(red, green, blue, 1.0))
}

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
    pub fn min(self, other: PixelLength) -> PixelLength {
        PixelLength(self.0.min(other.0))
    }
    pub fn abs(self) -> PixelLength {
        PixelLength(self.0.abs())
    }

    #[inline]
    pub fn from_val_unit(val: f32, unit: super::specified::Unit, font_size: PixelLength) -> Self {
        use super::specified::Unit;

        match unit {
            Unit::Px => Self(val),
            Unit::Cm => Self::from_val_unit(val / 2.54, Unit::In, font_size),
            Unit::Pt => Self::from_pt(val),
            Unit::Pc => Self::from_val_unit(val * 12.0, Unit::Pt, font_size),
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
            LengthPercentage::Both { length, percentage } => length + container_size * percentage,
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

impl GenericSize {
    pub fn zero() -> Self {
        GenericSize::LengthPercentage(LengthPercentage::from(PixelLength(0.0)))
    }
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

/// `max-width` and `max-height` reuse [`GenericSize`], where `Auto` stands for the keyword `none`
/// (no limit).
impl super::ComputedValue<specified::NoneOrQuantityOrPercentage> for GenericSize {
    fn from_specified(
        spec: &specified::NoneOrQuantityOrPercentage,
        style: &ComputedValues,
    ) -> Self {
        match spec {
            specified::NoneOrQuantityOrPercentage::None => GenericSize::Auto,
            specified::NoneOrQuantityOrPercentage::Percentage(percentage) => (*percentage).into(),
            specified::NoneOrQuantityOrPercentage::Quantity(specified::Quantity {
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
#[derive(Debug, Clone)]
pub struct Position {
    pub width: GenericSize,
    pub height: GenericSize,
    pub min_width: GenericSize,
    /// `Auto` means `none`.
    pub max_width: GenericSize,
    pub min_height: GenericSize,
    /// `Auto` means `none`.
    pub max_height: GenericSize,
    pub top: GenericSize,
    pub right: GenericSize,
    pub bottom: GenericSize,
    pub left: GenericSize,
    pub margin_top: GenericSize,
    pub margin_bottom: GenericSize,
    pub margin_left: GenericSize,
    pub margin_right: GenericSize,
    /// The radius of the corners of the border edge. Percentages refer to the border box.
    pub border_top_left_radius: LengthPercentage,
    pub border_top_right_radius: LengthPercentage,
    pub border_bottom_right_radius: LengthPercentage,
    pub border_bottom_left_radius: LengthPercentage,
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
    pub border_top_color: Color,
    pub border_bottom_color: Color,
    pub border_left_color: Color,
    pub border_right_color: Color,
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

impl Default for Position {
    fn default() -> Position {
        Position {
            width: GenericSize::Auto,
            height: GenericSize::Auto,
            min_width: GenericSize::Auto,
            max_width: GenericSize::Auto,
            min_height: GenericSize::Auto,
            max_height: GenericSize::Auto,
            top: GenericSize::Auto,
            right: GenericSize::Auto,
            bottom: GenericSize::Auto,
            left: GenericSize::Auto,
            margin_top: GenericSize::zero(),
            margin_bottom: GenericSize::zero(),
            margin_left: GenericSize::zero(),
            margin_right: GenericSize::zero(),
            border_top_left_radius: LengthPercentage::default(),
            border_top_right_radius: LengthPercentage::default(),
            border_bottom_right_radius: LengthPercentage::default(),
            border_bottom_left_radius: LengthPercentage::default(),
            padding_top: LengthPercentage::default(),
            padding_bottom: LengthPercentage::default(),
            padding_left: LengthPercentage::default(),
            padding_right: LengthPercentage::default(),
            border_top_width: PixelLength::default(),
            border_bottom_width: PixelLength::default(),
            border_left_width: PixelLength::default(),
            border_right_width: PixelLength::default(),
            border_top_style: BorderStyle::default(),
            border_bottom_style: BorderStyle::default(),
            border_left_style: BorderStyle::default(),
            border_right_style: BorderStyle::default(),
            border_top_color: Color::CurrentColor,
            border_bottom_color: Color::CurrentColor,
            border_left_color: Color::CurrentColor,
            border_right_color: Color::CurrentColor,
        }
    }
}

use std::convert::{AsMut, AsRef};

/// Contains the style structs and encompasses all the style information for
/// an element.
#[derive(Debug, Clone, Default)]
pub struct ComputedValues {
    pub box_: Arc<Box>,
    pub position: Arc<Position>,
    pub font: Arc<Font>,
    pub text: Arc<Text>,
    pub background: Arc<Background>,
    pub page: Arc<Page>,
    pub math: Arc<Math>,
}

impl AsRef<Box> for ComputedValues {
    fn as_ref(&self) -> &Box {
        &self.box_
    }
}
impl AsMut<Box> for ComputedValues {
    fn as_mut(&mut self) -> &mut Box {
        Arc::make_mut(&mut self.box_)
    }
}
impl AsRef<Position> for ComputedValues {
    fn as_ref(&self) -> &Position {
        &self.position
    }
}
impl AsMut<Position> for ComputedValues {
    fn as_mut(&mut self) -> &mut Position {
        Arc::make_mut(&mut self.position)
    }
}
impl AsRef<Font> for ComputedValues {
    fn as_ref(&self) -> &Font {
        &self.font
    }
}
impl AsMut<Font> for ComputedValues {
    fn as_mut(&mut self) -> &mut Font {
        Arc::make_mut(&mut self.font)
    }
}
impl AsRef<Text> for ComputedValues {
    fn as_ref(&self) -> &Text {
        &self.text
    }
}
impl AsMut<Text> for ComputedValues {
    fn as_mut(&mut self) -> &mut Text {
        Arc::make_mut(&mut self.text)
    }
}
impl AsRef<Background> for ComputedValues {
    fn as_ref(&self) -> &Background {
        &self.background
    }
}
impl AsMut<Background> for ComputedValues {
    fn as_mut(&mut self) -> &mut Background {
        Arc::make_mut(&mut self.background)
    }
}
impl AsRef<Page> for ComputedValues {
    fn as_ref(&self) -> &Page {
        &self.page
    }
}
impl AsMut<Page> for ComputedValues {
    fn as_mut(&mut self) -> &mut Page {
        Arc::make_mut(&mut self.page)
    }
}
impl AsRef<Math> for ComputedValues {
    fn as_ref(&self) -> &Math {
        &self.math
    }
}
impl AsMut<Math> for ComputedValues {
    fn as_mut(&mut self) -> &mut Math {
        Arc::make_mut(&mut self.math)
    }
}

impl ComputedValues {
    /// The style of an anonymous block box in a box with this style: it has the font and text
    /// properties of the box, but none of its box model, background or page break properties.
    /// https://www.w3.org/TR/CSS22/visuren.html#anonymous-block-level
    pub fn anonymous_block(&self) -> ComputedValues {
        ComputedValues {
            box_: Arc::new(Box {
                display: Display::Block,
                ..Default::default()
            }),
            position: Default::default(),
            background: Default::default(),
            page: Arc::new(Page {
                widows: self.page.widows,
                orphans: self.page.orphans,
                ..Default::default()
            }),
            ..self.clone()
        }
    }

    /// Blockifies the outer display type of the box: the children of flex and grid containers
    /// are block-level.
    /// https://www.w3.org/TR/css-display-3/#blockify
    pub fn blockify(&mut self) {
        match self.box_.display {
            Display::Inline | Display::InlineBlock => {
                Arc::make_mut(&mut self.box_).display = Display::Block
            }
            Display::InlineTable => Arc::make_mut(&mut self.box_).display = Display::Table,
            _ => {}
        }
    }

    fn fixup(&mut self) {
        Arc::make_mut(&mut self.position).fixup();
        // https://www.w3.org/TR/CSS22/visuren.html#dis-pos-flo
        // An absolutely positioned box is always blockified.
        if self.box_.position == PositionSpec::Absolute {
            self.blockify();
        }
    }
}

/// `script-percent-scale-down` / `script-script-percent-scale-down` defaults
/// (CSS Fonts 4 §`math-depth`). These are the values used when the font's
/// OpenType MATH table does not provide its own.
const SCRIPT_PERCENT_SCALE_DOWN: f32 = 0.71;
const SCRIPT_SCRIPT_PERCENT_SCALE_DOWN: f32 = 0.5;

/// Cumulative font scale factor at a given `math-depth`.
fn math_depth_scale(depth: i32) -> f32 {
    match depth {
        i32::MIN..=0 => 1.0,
        1 => SCRIPT_PERCENT_SCALE_DOWN,
        _ => SCRIPT_SCRIPT_PERCENT_SCALE_DOWN,
    }
}

#[derive(Debug, Default)]
pub struct StyleBuilder {
    new_values: ComputedValues,
    /// The values of the parent element, for `inherit`.
    parent: ComputedValues,
    is_display_none: bool,
    parent_math_depth: i32,
    parent_math_style: MathStyle,
    parent_font_size: PixelLength,
    /// Set when the winning `font-size` declaration is `math`.
    font_size_is_math: bool,
}

impl StyleBuilder {
    pub fn new(inherited_values: ComputedValues) -> Self {
        // Here we define which CSS property values are inherited

        let parent_font_size = inherited_values.font.font_size;
        let parent = inherited_values.clone();
        let inherited = ComputedValues {
            font: inherited_values.font,
            text: Arc::new(Text {
                white_space: inherited_values.text.white_space,
                text_indent: inherited_values.text.text_indent,
                text_align: inherited_values.text.text_align,
                hyphens: inherited_values.text.hyphens,
                letter_spacing: inherited_values.text.letter_spacing,
                text_transform: inherited_values.text.text_transform,
                direction: inherited_values.text.direction,
                border_spacing: inherited_values.text.border_spacing,
                border_collapse: inherited_values.text.border_collapse,
                ..Default::default()
            }),
            // widows and orphans are inherited, the page break properties are not
            page: Arc::new(Page {
                widows: inherited_values.page.widows,
                orphans: inherited_values.page.orphans,
                ..Default::default()
            }),
            math: Arc::new(Math {
                math_style: inherited_values.math.math_style,
                math_shift: inherited_values.math.math_shift,
                // `math-depth` is inherited. It stays unresolved until `build()`
                // so that `auto-add`/`add()` are relative to the parent's depth.
                math_depth: MathDepth::Absolute(inherited_values.math.depth()),
                ..Default::default()
            }),
            ..Default::default()
        };
        StyleBuilder {
            parent_math_depth: inherited_values.math.depth(),
            parent_math_style: inherited_values.math.math_style,
            parent_font_size,
            font_size_is_math: false,
            new_values: inherited,
            parent,
            is_display_none: inherited_values.box_.display == Display::None,
        }
    }

    /// Resolves `math-depth` relative to the parent element.
    ///
    /// `auto-add` adds 1 if the *inherited* `math-style` is `compact` (inline),
    /// and 0 otherwise.
    fn resolve_math_depth(&mut self) {
        let resolved = match self.new_values.math.math_depth {
            MathDepth::AutoAdd => {
                self.parent_math_depth + i32::from(self.parent_math_style == MathStyle::Inline)
            }
            MathDepth::Add(n) => self.parent_math_depth.saturating_add(n),
            MathDepth::Absolute(n) => n,
        };
        AsMut::<Math>::as_mut(&mut self.new_values).math_depth = MathDepth::Absolute(resolved);
    }

    pub fn add_declaration(&mut self, decl: &specified::PropertyDeclaration<'_>) {
        if let specified::PropertyDeclaration::Font(specified::Font::FontSize(size)) = decl {
            // The last `font-size` declaration wins.
            self.font_size_is_math = matches!(size, specified::FontSize::Math);
        }
        if let specified::PropertyDeclaration::CssWide(css_wide) = decl {
            if css_wide.resets_property("font-size") {
                self.font_size_is_math = false;
            }
            css_wide.apply(&mut self.new_values, &self.parent);
            return;
        }
        decl.apply_to(&mut self.new_values)
    }

    /// `font-size: math`: scale the parent's font size by the change in
    /// script level between the parent and this element.
    fn resolve_font_size_math(&mut self) {
        if !self.font_size_is_math {
            return;
        }
        let depth = self.new_values.math.depth();
        let ratio = math_depth_scale(depth) / math_depth_scale(self.parent_math_depth);
        Arc::make_mut(&mut self.new_values.font).font_size = ratio * self.parent_font_size;
    }

    pub fn build(mut self) -> ComputedValues {
        self.resolve_math_depth();
        self.resolve_font_size_math();
        self.new_values.fixup();
        if self.is_display_none {
            AsMut::<Box>::as_mut(&mut self.new_values).display = Display::None;
        }
        self.new_values
    }
}

#[derive(Debug, Clone)]
pub struct Box {
    pub display: specified::Display,
    pub unicode_bidi: specified::UnicodeBidi,
    pub position: PositionSpec,
    pub box_sizing: BoxSizing,

    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: GenericSize,

    pub justify_content: ContentAlignment,
    pub align_content: ContentAlignment,
    pub justify_items: ItemAlignment,
    pub align_items: ItemAlignment,
    pub justify_self: SelfAlignment,
    pub align_self: SelfAlignment,
    pub row_gap: LengthPercentage,
    pub column_gap: LengthPercentage,

    pub grid_template_columns: TrackList,
    pub grid_template_rows: TrackList,
    pub grid_auto_columns: TrackList,
    pub grid_auto_rows: TrackList,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_column_start: GridPlacement,
    pub grid_column_end: GridPlacement,
    pub grid_row_start: GridPlacement,
    pub grid_row_end: GridPlacement,
}

impl Default for Box {
    fn default() -> Self {
        Box {
            display: Default::default(),
            unicode_bidi: Default::default(),
            position: Default::default(),
            box_sizing: Default::default(),
            flex_direction: Default::default(),
            flex_wrap: Default::default(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: GenericSize::Auto,
            justify_content: Default::default(),
            align_content: Default::default(),
            justify_items: Default::default(),
            align_items: Default::default(),
            justify_self: Default::default(),
            align_self: Default::default(),
            row_gap: LengthPercentage::PixelLength(PixelLength(0.0)),
            column_gap: LengthPercentage::PixelLength(PixelLength(0.0)),
            grid_template_columns: Default::default(),
            grid_template_rows: Default::default(),
            grid_auto_columns: Default::default(),
            grid_auto_rows: Default::default(),
            grid_auto_flow: Default::default(),
            grid_column_start: Default::default(),
            grid_column_end: Default::default(),
            grid_row_start: Default::default(),
            grid_row_end: Default::default(),
        }
    }
}

/// A computed track size of a grid.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum TrackSize {
    Auto,
    MinContent,
    MaxContent,
    Fraction(f32),
    Length(LengthPercentage),
}

/// A computed list of track sizes of a grid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackList(pub Vec<TrackSize>);

impl super::ComputedValue<specified::TrackList> for TrackList {
    fn from_specified(spec: &specified::TrackList, style: &ComputedValues) -> Self {
        TrackList(
            spec.0
                .iter()
                .map(|size| match size {
                    specified::TrackSize::Auto => TrackSize::Auto,
                    specified::TrackSize::MinContent => TrackSize::MinContent,
                    specified::TrackSize::MaxContent => TrackSize::MaxContent,
                    specified::TrackSize::Fraction(fr) => TrackSize::Fraction(*fr),
                    specified::TrackSize::Length(length) => {
                        TrackSize::Length(LengthPercentage::from_specified(length, style))
                    }
                })
                .collect(),
        )
    }
}

/// The computed value of `font-weight`: a number from 1 to 1000.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct FontWeight(pub f32);

impl FontWeight {
    pub const NORMAL: FontWeight = FontWeight(400.0);
    pub const BOLD: FontWeight = FontWeight(700.0);
}

impl Default for FontWeight {
    fn default() -> Self {
        FontWeight::NORMAL
    }
}

// the weight is never NaN
impl Eq for FontWeight {}

impl std::hash::Hash for FontWeight {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state)
    }
}

#[derive(Debug, Clone)]
pub struct Font {
    pub font_family: Vec<specified::FontFamily>,
    pub font_style: FontStyle,
    pub font_weight: FontWeight,
    pub font_size: PixelLength,
    pub line_height: LineHeight,
    pub color: Color,
    pub list_style_type: specified::ListStyleType,
    pub text_decoration: specified::TextDecoration,
    /// The language of the element, from the `lang` attribute. It is not a CSS property but
    /// inherited like one, so it lives in the inherited font struct.
    pub lang: Language,
}

/// The language of the content of an element (a BCP 47 language tag like `zh-Hant-TW`), or
/// unknown. It chooses glyph variants in shaping, fallback fonts and the hyphenation dictionary.
/// https://html.spec.whatwg.org/multipage/dom.html#the-lang-and-xml:lang-attributes
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Language(Option<Arc<str>>);

impl Language {
    /// The language of a `lang` attribute value; the empty value means that it is unknown.
    pub fn from_tag(tag: &str) -> Self {
        let tag = tag.trim().replace('_', "-").to_ascii_lowercase();
        Language((!tag.is_empty()).then(|| Arc::from(tag)))
    }

    /// The whole tag in lower case, if the language is known.
    pub fn tag(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// The primary language subtag (`zh` of `zh-Hant-TW`).
    pub fn primary(&self) -> Option<&str> {
        self.tag().and_then(|tag| tag.split('-').next())
    }

    /// The script subtag (`hant` of `zh-Hant-TW`): the 4-letter subtag after the language.
    pub fn script(&self) -> Option<&str> {
        self.subtags()
            .find(|subtag| subtag.len() == 4 && subtag.chars().all(|c| c.is_ascii_alphabetic()))
    }

    /// The region subtag (`tw` of `zh-Hant-TW`): a 2-letter or 3-digit subtag after the
    /// language.
    pub fn region(&self) -> Option<&str> {
        self.subtags().find(|subtag| {
            (subtag.len() == 2 && subtag.chars().all(|c| c.is_ascii_alphabetic()))
                || (subtag.len() == 3 && subtag.chars().all(|c| c.is_ascii_digit()))
        })
    }

    /// The subtags after the primary language subtag, up to the first singleton (extensions
    /// and private use).
    fn subtags(&self) -> impl Iterator<Item = &str> {
        self.tag()
            .into_iter()
            .flat_map(|tag| tag.split('-').skip(1))
            .take_while(|subtag| subtag.len() > 1)
    }
}

impl Default for Font {
    fn default() -> Self {
        Self {
            font_family: vec![specified::FontFamily::SansSerif],
            font_style: FontStyle::default(),
            font_weight: FontWeight::default(),
            font_size: PixelLength::from_pt(12.0),
            line_height: LineHeight::default(),
            color: opaque(0, 0, 0),
            list_style_type: Default::default(),
            text_decoration: Default::default(),
            lang: Language::default(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Text {
    pub white_space: specified::WhiteSpace,
    pub text_indent: LengthPercentage,
    pub text_align: specified::TextAlign,
    pub hyphens: specified::Hyphens,
    /// The extra space after every character; `normal` is 0.
    pub letter_spacing: PixelLength,
    pub text_transform: specified::TextTransform,
    pub direction: specified::Direction,
    pub line_break: specified::LineBreak,
    pub border_spacing: BorderSpacing,
    pub border_collapse: specified::BorderCollapse,
}

impl ComputedValue<specified::NormalOrQuantity> for PixelLength {
    fn from_specified(spec: &specified::NormalOrQuantity, style: &ComputedValues) -> Self {
        match spec {
            specified::NormalOrQuantity::Normal => PixelLength(0.0),
            specified::NormalOrQuantity::Quantity(q) => {
                PixelLength::from_val_unit(q.value, q.unit, style.font.font_size)
            }
        }
    }
}

/// The computed value of `border-spacing`.
#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct BorderSpacing {
    pub horizontal: PixelLength,
    pub vertical: PixelLength,
}

impl ComputedValue<specified::BorderSpacing> for BorderSpacing {
    fn from_specified(spec: &specified::BorderSpacing, style: &ComputedValues) -> Self {
        let length = |q: specified::Quantity| {
            PixelLength::from_val_unit(q.value, q.unit, style.font.font_size)
        };
        BorderSpacing {
            horizontal: length(spec.horizontal),
            vertical: length(spec.vertical),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Background {
    pub background_color: Color,
    /// The images from top to bottom, painted above `background_color`.
    pub background_image: Vec<super::gradient::ComputedImage>,
    /// The blend mode of each image, repeated if there are fewer than images.
    pub background_blend_mode: Vec<super::specified::BlendMode>,
    /// Like `background_blend_mode`, the lists of the following properties are repeated if
    /// there are fewer values than images.
    pub background_size: Vec<super::background::ComputedSize>,
    pub background_position: Vec<super::gradient::Position<LengthPercentage>>,
    pub background_repeat: Vec<super::background::Repeat>,
}

impl Default for Background {
    fn default() -> Self {
        Background {
            background_color: Color::Rgba(cssparser_color::RgbaLegacy::new(0, 0, 0, 0.0)),
            background_image: Vec::new(),
            background_blend_mode: vec![super::specified::BlendMode::Normal],
            background_size: vec![Default::default()],
            background_position: vec![super::gradient::Position {
                x: LengthPercentage::Percentage(Percentage(0.0)),
                y: LengthPercentage::Percentage(Percentage(0.0)),
            }],
            background_repeat: vec![Default::default()],
        }
    }
}

/// The computed value of `size` in `@page`.
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum PageSize {
    #[default]
    Auto,
    Dimensions {
        width: PixelLength,
        height: PixelLength,
    },
    /// Turn the default page size to the orientation.
    Orientation { landscape: bool },
}

impl ComputedValue<specified::PageSize> for PageSize {
    fn from_specified(spec: &specified::PageSize, style: &ComputedValues) -> Self {
        let px = |q: specified::Quantity| {
            PixelLength::from_val_unit(q.value, q.unit, style.font.font_size)
        };
        match *spec {
            specified::PageSize::Auto => PageSize::Auto,
            specified::PageSize::Lengths(width, height) => PageSize::Dimensions {
                width: px(width),
                height: height.map_or(px(width), px),
            },
            specified::PageSize::Keywords { name: None, landscape } => PageSize::Orientation {
                landscape: landscape.unwrap_or(false),
            },
            specified::PageSize::Keywords { name: Some(name), landscape } => {
                let (width, height, unit) = name.dimensions();
                let (width, height) = (
                    PixelLength::from_val_unit(width, unit, style.font.font_size),
                    PixelLength::from_val_unit(height, unit, style.font.font_size),
                );
                if landscape == Some(true) {
                    PageSize::Dimensions { width: height, height: width }
                } else {
                    PageSize::Dimensions { width, height }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Page {
    pub size: PageSize,
    pub widows: usize,
    pub orphans: usize,
    pub break_after: Break,
    pub break_before: Break,
    pub break_inside: BreakInside,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            size: PageSize::Auto,
            widows: 2,
            orphans: 2,
            break_after: Break::Auto,
            break_before: Break::Auto,
            break_inside: BreakInside::Auto,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Math {
    pub math_variant: MathFamily,
    pub form: MathOperatorForm,
    pub operator_is_stretchy: AutoBool,
    pub math_style: MathStyle,
    /// Always `MathDepth::Absolute` once the style has been built.
    pub math_depth: MathDepth,
    pub math_shift: MathShift,
}

impl Math {
    /// The resolved `math-depth`.
    pub fn depth(&self) -> i32 {
        match self.math_depth {
            MathDepth::Absolute(n) => n,
            // Only observable while a style is still being built.
            MathDepth::AutoAdd | MathDepth::Add(_) => 0,
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
            specified::LineHeight::Number(num) => {
                LineHeight::LengthPercentage(LengthPercentage::Percentage(Percentage(*num)))
            }
        }
    }
}

#[cfg(test)]
mod math_depth_tests {
    use super::*;
    use crate::style::parse::PropertyDeclarationBlock;

    fn child(parent: &ComputedValues, css: &str) -> ComputedValues {
        let mut builder = StyleBuilder::new(parent.clone());
        for decl in PropertyDeclarationBlock::parse_inline(css).declarations() {
            builder.add_declaration(decl);
        }
        builder.build()
    }

    fn root(css: &str) -> ComputedValues {
        child(&ComputedValues::default(), css)
    }

    #[test]
    fn depth_is_inherited_and_relative() {
        let a = root("math-depth: 2");
        assert_eq!(a.math.depth(), 2);
        let b = child(&a, "");
        assert_eq!(b.math.depth(), 2);
        assert_eq!(child(&b, "math-depth: add(1)").math.depth(), 3);
        assert_eq!(child(&b, "math-depth: add(-1)").math.depth(), 1);
        assert_eq!(child(&b, "math-depth: 0").math.depth(), 0);
    }

    #[test]
    fn repeated_declarations_do_not_accumulate() {
        let p = root("math-depth: 1");
        assert_eq!(
            child(&p, "math-depth: add(1); math-depth: add(1)")
                .math
                .depth(),
            2
        );
    }

    #[test]
    fn auto_add_uses_inherited_math_style() {
        // inherited math-style is `display` (normal): no change
        let display = root("-helium-math-style: display");
        assert_eq!(child(&display, "math-depth: auto-add").math.depth(), 0);
        // inherited math-style is `inline` (compact): +1
        let compact = root("-helium-math-style: inline");
        assert_eq!(child(&compact, "math-depth: auto-add").math.depth(), 1);
        // the element's own math-style must not influence its own auto-add,
        // regardless of declaration order
        assert_eq!(
            child(&display, "-helium-math-style: inline; math-depth: auto-add")
                .math
                .depth(),
            0
        );
        assert_eq!(
            child(
                &compact,
                "math-depth: auto-add; -helium-math-style: display"
            )
            .math
            .depth(),
            1
        );
    }

    #[test]
    fn text_properties_are_inherited() {
        let parent = root(
            "white-space: pre; text-indent: 12px; text-align: center; hyphens: auto; border-spacing: 3px 4px",
        );
        let child = child(&parent, "");

        assert_eq!(child.text.white_space, parent.text.white_space);
        assert_eq!(child.text.text_indent, parent.text.text_indent);
        assert_eq!(child.text.text_align, parent.text.text_align);
        assert_eq!(child.text.hyphens, parent.text.hyphens);
        assert_eq!(child.text.border_spacing, parent.text.border_spacing);
    }

    #[test]
    fn letter_spacing_is_inherited_as_a_length() {
        let parent = root("font-size: 20px; letter-spacing: 0.5em");
        assert_eq!(parent.text.letter_spacing, PixelLength(10.0));
        // the computed length is inherited, not the em
        let grandchild = child(&parent, "font-size: 40px");
        assert_eq!(grandchild.text.letter_spacing, PixelLength(10.0));
        let reset = child(&parent, "letter-spacing: normal");
        assert_eq!(reset.text.letter_spacing, PixelLength(0.0));
    }

    #[test]
    fn font_size_math_scales_with_depth() {
        let base = root("font-size: 100px");
        let size = |s: &ComputedValues| s.font.font_size.0;
        let d1 = child(&base, "math-depth: add(1); font-size: math");
        assert!((size(&d1) - 71.0).abs() < 1e-3);
        // 1 -> 2 goes from 0.71 to 0.5 of the base
        let d2 = child(&d1, "math-depth: add(1); font-size: math");
        assert!((size(&d2) - 71.0 * 0.5 / 0.71).abs() < 1e-3);
        // same depth: unchanged; declaration order does not matter
        let same = child(&d2, "font-size: math");
        assert!((size(&same) - size(&d2)).abs() < 1e-3);
        let reordered = child(&base, "font-size: math; math-depth: add(1)");
        assert!((size(&reordered) - 71.0).abs() < 1e-3);
        // going back up restores the size
        let back = child(&d1, "math-depth: 0; font-size: math");
        assert!((size(&back) - 100.0).abs() < 1e-3);
        // an explicit font-size after `math` wins
        let explicit = child(&base, "font-size: math; font-size: 10px; math-depth: 2");
        assert!((size(&explicit) - 10.0).abs() < 1e-3);
    }
}

#[cfg(test)]
mod language_tests {
    use super::Language;

    #[test]
    fn tags_are_split_into_their_subtags() {
        let lang = Language::from_tag(" zh-Hant-TW ");
        assert_eq!(lang.tag(), Some("zh-hant-tw"));
        assert_eq!(lang.primary(), Some("zh"));
        assert_eq!(lang.script(), Some("hant"));
        assert_eq!(lang.region(), Some("tw"));

        let lang = Language::from_tag("es-419");
        assert_eq!(lang.script(), None);
        assert_eq!(lang.region(), Some("419"));

        let lang = Language::from_tag("en_GB");
        assert_eq!(lang.region(), Some("gb"));

        // subtags after an extension are not the script or region
        let lang = Language::from_tag("de-x-test");
        assert_eq!(lang.primary(), Some("de"));
        assert_eq!(lang.script(), None);
        assert_eq!(lang.region(), None);
    }

    #[test]
    fn the_empty_tag_is_an_unknown_language() {
        let lang = Language::from_tag("");
        assert_eq!(lang, Language::default());
        assert_eq!(lang.primary(), None);
    }
}

#[cfg(test)]
mod css_wide_tests {
    use super::*;
    use crate::style::parse::PropertyDeclarationBlock;

    fn child(parent: &ComputedValues, css: &str) -> ComputedValues {
        let mut builder = StyleBuilder::new(parent.clone());
        for decl in PropertyDeclarationBlock::parse_inline(css).declarations() {
            builder.add_declaration(decl);
        }
        builder.build()
    }

    #[test]
    fn inherit_copies_the_value_of_the_parent_even_for_non_inherited_properties() {
        let parent = child(&ComputedValues::default(), "width: 10px; margin-left: 7px");
        let c = child(&parent, "");
        assert!(matches!(c.position.width, GenericSize::Auto));
        let c = child(&parent, "width: inherit; margin: inherit");
        assert!(!matches!(c.position.width, GenericSize::Auto));
        assert!(!matches!(c.position.margin_left, GenericSize::Auto));
        assert!(!matches!(c.position.margin_left, GenericSize::Auto));
    }

    #[test]
    fn font_shorthand_with_a_keyword_resets_the_font_properties() {
        let parent = child(&ComputedValues::default(), "font-weight: normal");
        // a user agent style makes the element bold, the author style undoes it
        let c = child(&parent, "font-weight: bold; font: inherit");
        assert_eq!(c.font.font_weight, FontWeight::NORMAL);
        let c = child(&parent, "font-weight: bold; font: initial");
        assert_eq!(c.font.font_weight, FontWeight::NORMAL);
        // later declarations still win
        let c = child(&parent, "font: inherit; font-weight: bold");
        assert_eq!(c.font.font_weight, FontWeight::BOLD);
    }

    #[test]
    fn initial_and_unset() {
        let parent = child(&ComputedValues::default(), "font-weight: bold; width: 10px");
        assert_eq!(child(&parent, "").font.font_weight, FontWeight::BOLD);
        assert_eq!(child(&parent, "font-weight: initial").font.font_weight, FontWeight::NORMAL);
        // font-weight is inherited, width is not
        assert_eq!(child(&parent, "font-weight: unset").font.font_weight, FontWeight::BOLD);
        assert!(matches!(child(&parent, "width: unset").position.width, GenericSize::Auto));
    }

    #[test]
    fn later_keywords_win_and_unknown_properties_are_ignored() {
        let parent = child(&ComputedValues::default(), "font-weight: bold");
        let c = child(&parent, "font-weight: normal; font-weight: inherit");
        assert_eq!(c.font.font_weight, FontWeight::BOLD);
        child(&parent, "bogus: inherit");
    }

    fn weight(parent: &ComputedValues, css: &str) -> f32 {
        child(parent, css).font.font_weight.0
    }

    #[test]
    fn numeric_font_weights() {
        let root = ComputedValues::default();
        assert_eq!(weight(&root, "font-weight: 100"), 100.0);
        assert_eq!(weight(&root, "font-weight: 650"), 650.0);
        assert_eq!(weight(&root, "font-weight: 1"), 1.0);
        assert_eq!(weight(&root, "font-weight: 1000"), 1000.0);
        assert_eq!(weight(&root, "font-weight: 450.5"), 450.5);
        assert_eq!(weight(&root, "font-weight: normal"), 400.0);
        assert_eq!(weight(&root, "font-weight: bold"), 700.0);
        // values outside of 1 to 1000 are invalid and the declaration is ignored
        let parent = child(&root, "font-weight: 300");
        assert_eq!(weight(&parent, "font-weight: 0"), 300.0);
        assert_eq!(weight(&parent, "font-weight: 1001"), 300.0);
        assert_eq!(weight(&parent, "font-weight: -100"), 300.0);
        assert_eq!(weight(&parent, "font-weight: semibold"), 300.0);
    }

    #[test]
    fn relative_font_weights() {
        let root = ComputedValues::default();
        let at = |w: &str| child(&root, &format!("font-weight: {w}"));
        // https://www.w3.org/TR/css-fonts-4/#relative-weights
        for (parent, bolder, lighter) in [
            (100.0, 400.0, 100.0),
            (300.0, 400.0, 100.0),
            (400.0, 700.0, 100.0),
            (500.0, 700.0, 100.0),
            (600.0, 900.0, 400.0),
            (700.0, 900.0, 400.0),
            (800.0, 900.0, 700.0),
            (900.0, 900.0, 700.0),
        ] {
            let p = at(&parent.to_string());
            assert_eq!(weight(&p, "font-weight: bolder"), bolder, "bolder than {parent}");
            assert_eq!(weight(&p, "font-weight: lighter"), lighter, "lighter than {parent}");
        }
        // the steps follow the inherited weight
        let bold = at("bold");
        let bolder = child(&bold, "font-weight: bolder");
        assert_eq!(weight(&bolder, ""), 900.0);
    }

    #[test]
    fn font_shorthand_sets_all_the_font_properties() {
        use super::super::specified::{FontFamily, FontStyle};
        let root = ComputedValues::default();
        let c = child(&root, "font: italic bold 20px/1.5 \"Helvetica\", serif");
        assert_eq!(c.font.font_style, FontStyle::Italic);
        assert_eq!(c.font.font_weight, FontWeight::BOLD);
        assert_eq!(c.font.font_size, PixelLength(20.0));
        assert!(matches!(c.font.line_height, LineHeight::LengthPercentage(LengthPercentage::Percentage(Percentage(n))) if n == 1.5));
        assert_eq!(c.font.font_family, [FontFamily::Named("Helvetica".into()), FontFamily::Serif]);
        // numeric weights and the order of the optional values
        let c = child(&root, "font: 300 oblique 10px monospace");
        assert_eq!(c.font.font_weight, FontWeight(300.0));
        assert_eq!(c.font.font_style, FontStyle::Oblique);
        assert_eq!(c.font.font_family, [FontFamily::Monospace]);
        // `normal`, small-caps and stretch keywords are accepted
        let c = child(&root, "font: normal small-caps condensed bold 12px serif");
        assert_eq!(c.font.font_weight, FontWeight::BOLD);
    }

    #[test]
    fn font_shorthand_resets_what_it_leaves_out() {
        let parent = child(
            &ComputedValues::default(),
            "font-weight: bold; font-style: italic; line-height: 2",
        );
        // the parent values are inherited, the shorthand sets all of them
        let c = child(&parent, "font: 10px serif");
        assert_eq!(c.font.font_weight, FontWeight::NORMAL);
        assert_eq!(c.font.font_style, super::super::specified::FontStyle::Normal);
        assert!(matches!(c.font.line_height, LineHeight::Normal));
        // the longhands that follow win
        let c = child(&parent, "font: 10px serif; font-weight: 600");
        assert_eq!(c.font.font_weight, FontWeight(600.0));
        // and the shorthand wins over the longhands before it
        let c = child(&parent, "font-weight: 600; font: 10px serif");
        assert_eq!(c.font.font_weight, FontWeight::NORMAL);
    }

    #[test]
    fn font_shorthand_needs_size_and_family() {
        let parent = child(&ComputedValues::default(), "font-weight: 600; font-size: 20px");
        for css in ["font: bold", "font: 12px", "font: bold serif", "font: 12px/ serif", "font: 1500 12px serif"] {
            let c = child(&parent, css);
            assert_eq!(c.font.font_weight, FontWeight(600.0), "{css}");
            assert_eq!(c.font.font_size, PixelLength(20.0), "{css}");
        }
    }

    #[test]
    fn font_family_names_can_be_unquoted_words() {
        use super::super::specified::FontFamily;
        let root = ComputedValues::default();
        let c = child(&root, "font-family: Times New Roman, 'Foo Bar', Sans-Serif, Monospace");
        assert_eq!(
            c.font.font_family,
            [
                FontFamily::Named("Times New Roman".into()),
                FontFamily::Named("Foo Bar".into()),
                FontFamily::SansSerif,
                FontFamily::Monospace,
            ]
        );
        let c = child(&root, "font: 14px Courier New");
        assert_eq!(c.font.font_family, [FontFamily::Named("Courier New".into())]);
        assert_eq!(c.font.font_size, PixelLength(14.0));
    }
}
