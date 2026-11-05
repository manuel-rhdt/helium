use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};
use std::sync::Arc;

use super::specified::{
    self, ApplyToComputedValues, AutoBool, BorderStyle, ComputedValue, MathFamily, MathOperatorForm,
};
pub use super::specified::{
    ContentAlignment, Display, FlexDirection, FlexWrap, FontFamily, FontStyle, FontWeight, GridAutoFlow,
    GridPlacement, ItemAlignment, MathDepth, MathShift, MathStyle, PageBreak, Percentage, PositionSpec,
    SelfAlignment, TextAlign,
};

pub use cssparser_color::Color;

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

/// The color a CSS color resolves to when painting.
///
/// Colors that are not defined in sRGB (lab, oklch, `color()`) are not supported yet and
/// resolve to transparent. `currentcolor` also resolves to transparent for now.
pub fn color_to_rgba(color: Color) -> RGBA {
    use cssparser_color::{hsl_to_rgb, hwb_to_rgb};

    fn channel(value: f32) -> u8 {
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    }
    fn from_floats((r, g, b): (f32, f32, f32), alpha: Option<f32>) -> RGBA {
        RGBA::new(
            channel(r),
            channel(g),
            channel(b),
            channel(alpha.unwrap_or(1.0)),
        )
    }

    match color {
        Color::Rgba(c) => RGBA::new(c.red, c.green, c.blue, channel(c.alpha)),
        Color::Hsl(c) => from_floats(
            hsl_to_rgb(
                (c.hue.unwrap_or(0.0) / 360.0).rem_euclid(1.0),
                c.saturation.unwrap_or(0.0),
                c.lightness.unwrap_or(0.0),
            ),
            c.alpha,
        ),
        Color::Hwb(c) => from_floats(
            hwb_to_rgb(
                (c.hue.unwrap_or(0.0) / 360.0).rem_euclid(1.0),
                c.whiteness.unwrap_or(0.0),
                c.blackness.unwrap_or(0.0),
            ),
            c.alpha,
        ),
        Color::CurrentColor
        | Color::Lab(_)
        | Color::Lch(_)
        | Color::Oklab(_)
        | Color::Oklch(_)
        | Color::ColorFunction(_) => RGBA::transparent(),
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
    pub top: GenericSize,
    pub right: GenericSize,
    pub bottom: GenericSize,
    pub left: GenericSize,
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
            top: GenericSize::Auto,
            right: GenericSize::Auto,
            bottom: GenericSize::Auto,
            left: GenericSize::Auto,
            margin_top: GenericSize::zero(),
            margin_bottom: GenericSize::zero(),
            margin_left: GenericSize::zero(),
            margin_right: GenericSize::zero(),
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
    pub page: Arc<Page>,
    pub math: Arc<Math>,
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
impl AsMut<Page> for ComputedValues {
    fn as_mut(&mut self) -> &mut Page {
        Arc::make_mut(&mut self.page)
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
        let inherited = ComputedValues {
            font: inherited_values.font,
            text: Arc::new(Text {
                white_space: inherited_values.text.white_space,
                text_indent: inherited_values.text.text_indent,
                text_align: inherited_values.text.text_align,
                hyphens: inherited_values.text.hyphens,
                border_spacing: inherited_values.text.border_spacing,
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
                self.parent_math_depth
                    + i32::from(self.parent_math_style == MathStyle::Inline)
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
    pub position: PositionSpec,

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
            position: Default::default(),
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
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Text {
    pub white_space: specified::WhiteSpace,
    pub text_indent: LengthPercentage,
    pub text_align: specified::TextAlign,
    pub hyphens: specified::Hyphens,
    pub line_break: specified::LineBreak,
    pub border_spacing: BorderSpacing,
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
}

impl Default for Background {
    fn default() -> Self {
        Background {
            background_color: Color::Rgba(cssparser_color::RgbaLegacy::new(0, 0, 0, 0.0)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Page {
    pub widows: usize,
    pub orphans: usize,
    pub page_break_after: PageBreak,
    pub page_break_before: PageBreak,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            widows: 2,
            orphans: 2,
            page_break_after: PageBreak::Auto,
            page_break_before: PageBreak::Auto,
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
        assert_eq!(child(&p, "math-depth: add(1); math-depth: add(1)").math.depth(), 2);
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
            child(&display, "-helium-math-style: inline; math-depth: auto-add").math.depth(),
            0
        );
        assert_eq!(
            child(&compact, "math-depth: auto-add; -helium-math-style: display").math.depth(),
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
