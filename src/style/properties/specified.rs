use std::borrow::{Cow, ToOwned};
use std::str::FromStr;

use super::super::parse::StyleParseError;
use super::computed::PixelLength;
use cssparser::{CowRcStr, ParseError, Parser, Token, match_ignore_ascii_case};
use cssparser_color::Color;
use helium_derive::{ParseDeclaration, PropertyValue};

use math_render::unicode_math::Family as MathFamily2;

pub trait ParseDeclaration<'i> {
    type Error: 'i;
    fn parse_declaration(
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>>
    where
        Self: Sized;
}

pub trait ApplyToComputedValues {
    fn apply_to(&self, computed: &mut super::computed::ComputedValues);
}

/// The CSS-wide keywords.
/// https://www.w3.org/TR/css-cascade-4/#defaulting-keywords
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssWide {
    Inherit,
    Initial,
    /// `inherit` for inherited properties and `initial` for the others.
    Unset,
}

/// Resets a property by name to the value of its parent or its initial value. Implemented for
/// the declaration enums by `#[derive(ParseDeclaration)]`; returns whether the property is known.
pub trait CssWideProperties {
    fn apply_css_wide(
        name: &str,
        keyword: CssWide,
        computed: &mut super::computed::ComputedValues,
        parent: &super::computed::ComputedValues,
        initial: &super::computed::ComputedValues,
    ) -> bool;
}

/// Whether a property is inherited (for `unset`). Must agree with `StyleBuilder::new`.
fn is_inherited_property(name: &str) -> bool {
    const INHERITED: &[&str] = &[
        "font-family", "font-style", "font-weight", "font-size", "line-height", "color",
        "list-style-type", "white-space", "text-indent", "text-align", "hyphens", "direction",
        "border-spacing", "border-collapse", "widows", "orphans", "math-style", "math-shift",
        "math-depth",
    ];
    INHERITED.iter().any(|inherited| name.eq_ignore_ascii_case(inherited))
}

/// A property set to a CSS-wide keyword, e.g. `font: inherit`. It is resolved against the values
/// of the parent element when the style is built (`StyleBuilder::add_declaration`).
#[derive(Debug, Clone)]
pub struct CssWideDeclaration {
    pub keyword: CssWide,
    /// The name of the property or shorthand, as written.
    pub property: String,
}

/// The properties that the `font` shorthand resets.
const FONT_LONGHANDS: &[&str] = &["font-family", "font-style", "font-weight", "font-size", "line-height"];

impl CssWideDeclaration {
    /// Parses a CSS-wide keyword as the whole value of the declaration of property `name`, if
    /// that is a supported property.
    pub fn parse<'i>(name: &str, input: &mut Parser<'i>) -> Option<Self> {
        let state = input.state();
        let keyword = input.try_parse(|input| {
            let ident = input.expect_ident()?.clone();
            Ok::<_, ParseError<()>>(match_ignore_ascii_case! { &ident,
                "inherit" => CssWide::Inherit,
                "initial" => CssWide::Initial,
                "unset" => CssWide::Unset,
                _ => return Err(ParseError::custom(())),
            })
        });
        // nothing but `!important` may follow
        let keyword = keyword.ok().filter(|_| {
            let rest = input.state();
            let ok = match input.next() {
                Err(_) => true,
                Ok(Token::Delim('!')) => true,
                Ok(_) => false,
            };
            input.reset(&rest);
            ok
        });
        let declaration = keyword.map(|keyword| CssWideDeclaration { keyword, property: name.to_owned() });
        match declaration {
            Some(declaration) if declaration.is_supported() => Some(declaration),
            _ => {
                input.reset(&state);
                None
            }
        }
    }

    fn is_supported(&self) -> bool {
        let scratch = super::computed::ComputedValues::default();
        self.property.eq_ignore_ascii_case("font")
            || PropertyDeclaration::apply_css_wide(
                &self.property,
                self.keyword,
                &mut scratch.clone(),
                &scratch,
                &scratch,
            )
    }

    /// Whether this declaration resets the property `name` (directly or as part of `font`).
    pub fn resets_property(&self, name: &str) -> bool {
        self.property.eq_ignore_ascii_case(name)
            || (self.property.eq_ignore_ascii_case("font") && FONT_LONGHANDS.contains(&name))
    }

    pub fn apply(
        &self,
        computed: &mut super::computed::ComputedValues,
        parent: &super::computed::ComputedValues,
    ) {
        let initial = super::computed::ComputedValues::default();
        if self.property.eq_ignore_ascii_case("font") {
            for longhand in FONT_LONGHANDS {
                PropertyDeclaration::apply_css_wide(longhand, self.keyword, computed, parent, &initial);
            }
        } else {
            PropertyDeclaration::apply_css_wide(&self.property, self.keyword, computed, parent, &initial);
        }
    }
}

impl<'i> ParseDeclaration<'i> for CssWideDeclaration {
    type Error = StyleParseError;

    /// CSS-wide keywords are recognized before the property is parsed (`CssWideDeclaration::parse`).
    fn parse_declaration(
        name: CowRcStr<'i>,
        _input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<StyleParseError>> {
        Err(ParseError::custom(StyleParseError::UnknownProperty { name: name.to_string() }))
    }
}

impl ApplyToComputedValues for CssWideDeclaration {
    fn apply_to(&self, _: &mut super::computed::ComputedValues) {}
}

impl CssWideProperties for CssWideDeclaration {
    fn apply_css_wide(
        _: &str,
        _: CssWide,
        _: &mut super::computed::ComputedValues,
        _: &super::computed::ComputedValues,
        _: &super::computed::ComputedValues,
    ) -> bool {
        false
    }
}

pub trait PropertyValue<'i>: Sized {
    type Error: 'i;
    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>>
    where
        Self: Sized;

    fn computed_value<T>(&self, computed_values: &super::computed::ComputedValues) -> T
    where
        T: ComputedValue<Self>,
    {
        T::from_specified(self, computed_values)
    }
}

pub trait ComputedValue<T>: Sized {
    fn from_specified(
        specified_value: &T,
        computed_values: &super::computed::ComputedValues,
    ) -> Self;
}

impl<'i> PropertyValue<'i> for String {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        Ok(input.expect_ident_or_string()?.to_string())
    }
}

impl<'i, T> PropertyValue<'i> for Vec<T>
where
    T: PropertyValue<'i>,
{
    type Error = T::Error;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        input.parse_entirely(|input| {
            let mut result = vec![];
            loop {
                if input.is_exhausted() {
                    break Ok(result);
                }
                match T::from_css_property_value(input) {
                    Ok(val) => result.push(val),
                    Err(err) => break Err(err),
                }
            }
        })
    }
}

impl<Specified, Computed> ComputedValue<Specified> for Computed
where
    Computed: From<Specified>,
    Specified: Clone,
{
    fn from_specified(
        specified_value: &Specified,
        _: &super::computed::ComputedValues,
    ) -> Computed {
        specified_value.clone().into()
    }
}

/// Holds any property declaration that is supported by helium, grouped
/// by style structs.
#[derive(Debug, Clone, ParseDeclaration)]
pub enum PropertyDeclaration<'a> {
    Position(Position<'a>),
    Font(Font),
    Box(Box),
    Text(Text),
    Background(Background),
    Page(Page),
    Math(Math),
    CssWide(CssWideDeclaration),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Position"]
pub enum Position<'a> {
    #[property]
    Width(AutoOrQuantityOrPercentage),
    #[property]
    Height(AutoOrQuantityOrPercentage),

    /// Offsets of positioned boxes.
    /// https://www.w3.org/TR/CSS22/visuren.html#position-props
    #[property]
    Top(AutoOrQuantityOrPercentage),
    #[property]
    Right(AutoOrQuantityOrPercentage),
    #[property]
    Bottom(AutoOrQuantityOrPercentage),
    #[property]
    Left(AutoOrQuantityOrPercentage),

    #[property]
    #[shorthand("margin-top", method = "top")]
    #[shorthand("margin-bottom", method = "bottom")]
    #[shorthand("margin-left", method = "left")]
    #[shorthand("margin-right", method = "right")]
    /// https://www.w3.org/TR/CSS22/box.html#propdef-margin
    Margin(List<'a, AutoOrQuantityOrPercentage>),

    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-top
    #[property]
    MarginTop(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-bottom
    #[property]
    MarginBottom(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-left
    #[property]
    MarginLeft(AutoOrQuantityOrPercentage),
    /// Spec: https://www.w3.org/TR/CSS22/box.html#propdef-margin-right
    #[property]
    MarginRight(AutoOrQuantityOrPercentage),

    #[property]
    #[shorthand("padding-top", method = "top")]
    #[shorthand("padding-bottom", method = "bottom")]
    #[shorthand("padding-left", method = "left")]
    #[shorthand("padding-right", method = "right")]
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding
    Padding(List<'a, QuantityOrPercentage>),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-top
    #[property]
    PaddingTop(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-bottom
    #[property]
    PaddingBottom(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-left
    #[property]
    PaddingLeft(QuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-padding-right
    #[property]
    PaddingRight(QuantityOrPercentage),

    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-top-width
    #[property]
    BorderTopWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-bottom-width
    #[property]
    BorderBottomWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-left-width
    #[property]
    BorderLeftWidth(BorderWidth),
    /// https://www.w3.org/TR/CSS22/box.html#propdef-border-right-width
    #[property]
    BorderRightWidth(BorderWidth),
    #[property]
    #[shorthand("border-top-width", method = "top")]
    #[shorthand("border-bottom-width", method = "bottom")]
    #[shorthand("border-left-width", method = "left")]
    #[shorthand("border-right-width", method = "right")]
    BorderWidth(List<'a, BorderWidth>),

    #[property]
    BorderTopStyle(BorderStyle),
    #[property]
    BorderBottomStyle(BorderStyle),
    #[property]
    BorderLeftStyle(BorderStyle),
    #[property]
    BorderRightStyle(BorderStyle),
    #[property]
    #[shorthand("border-top-style", method = "top")]
    #[shorthand("border-bottom-style", method = "bottom")]
    #[shorthand("border-left-style", method = "left")]
    #[shorthand("border-right-style", method = "right")]
    BorderStyle(List<'a, BorderStyle>),

    #[property]
    BorderTopColor(Color),
    #[property]
    BorderBottomColor(Color),
    #[property]
    BorderLeftColor(Color),
    #[property]
    BorderRightColor(Color),
    #[property]
    #[shorthand("border-top-color", method = "top")]
    #[shorthand("border-bottom-color", method = "bottom")]
    #[shorthand("border-left-color", method = "left")]
    #[shorthand("border-right-color", method = "right")]
    BorderColor(List<'a, Color>),

    #[property]
    #[shorthand("border-top-width", method = "width")]
    #[shorthand("border-top-style", method = "style")]
    #[shorthand("border-top-color", method = "color")]
    BorderTop(List<'a, Border>),
    #[property]
    #[shorthand("border-bottom-width", method = "width")]
    #[shorthand("border-bottom-style", method = "style")]
    #[shorthand("border-bottom-color", method = "color")]
    BorderBottom(List<'a, Border>),
    #[property]
    #[shorthand("border-left-width", method = "width")]
    #[shorthand("border-left-style", method = "style")]
    #[shorthand("border-left-color", method = "color")]
    BorderLeft(List<'a, Border>),
    #[property]
    #[shorthand("border-right-width", method = "width")]
    #[shorthand("border-right-style", method = "style")]
    #[shorthand("border-right-color", method = "color")]
    BorderRight(List<'a, Border>),

    #[property]
    #[shorthand("border-top", method = "make_ref")]
    #[shorthand("border-bottom", method = "make_ref")]
    #[shorthand("border-left", method = "make_ref")]
    #[shorthand("border-right", method = "make_ref")]
    Border(List<'a, Border>),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Font"]
pub enum Font {
    #[property]
    FontFamily(CommaSeparated<FontFamily>),
    #[property]
    FontStyle(FontStyle),
    #[property]
    FontWeight(FontWeight),
    #[property]
    FontSize(FontSize),
    #[property]
    LineHeight(LineHeight),
    /// Inherited like the font properties, so it lives in the same style struct.
    #[property]
    Color(Color),
    /// Inherited, so it lives here as well.
    #[property]
    ListStyleType(ListStyleType),
    /// Inherited instead of propagated to the inline boxes of the descendants, which has the same
    /// effect for everything that is supported.
    #[property]
    TextDecoration(TextDecoration),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Box"]
pub enum Box {
    #[property]
    Display(Display),
    /// https://www.w3.org/TR/css-writing-modes-3/#unicode-bidi
    #[property]
    UnicodeBidi(UnicodeBidi),
    #[property]
    Position(PositionSpec),

    // Flexbox: https://www.w3.org/TR/css-flexbox-1/
    #[property]
    FlexDirection(FlexDirection),
    #[property]
    FlexWrap(FlexWrap),
    #[property]
    FlexGrow(f32),
    #[property]
    FlexShrink(f32),
    #[property]
    FlexBasis(AutoOrQuantityOrPercentage),
    #[property]
    #[shorthand("flex-grow", method = "grow")]
    #[shorthand("flex-shrink", method = "shrink")]
    #[shorthand("flex-basis", method = "basis")]
    Flex(FlexShorthand),

    // Box alignment: https://www.w3.org/TR/css-align-3/
    #[property]
    JustifyContent(ContentAlignment),
    #[property]
    AlignContent(ContentAlignment),
    #[property]
    JustifyItems(ItemAlignment),
    #[property]
    AlignItems(ItemAlignment),
    #[property]
    JustifySelf(SelfAlignment),
    #[property]
    AlignSelf(SelfAlignment),
    #[property]
    RowGap(Gap),
    #[property]
    ColumnGap(Gap),
    #[property]
    #[shorthand("row-gap", method = "clone")]
    #[shorthand("column-gap", method = "clone")]
    Gap(Gap),

    // Grid: https://www.w3.org/TR/css-grid-1/
    #[property]
    GridTemplateColumns(TrackList),
    #[property]
    GridTemplateRows(TrackList),
    #[property]
    GridAutoColumns(TrackList),
    #[property]
    GridAutoRows(TrackList),
    #[property]
    GridAutoFlow(GridAutoFlow),
    #[property]
    GridColumnStart(GridPlacement),
    #[property]
    GridColumnEnd(GridPlacement),
    #[property]
    GridRowStart(GridPlacement),
    #[property]
    GridRowEnd(GridPlacement),
    #[property]
    #[shorthand("grid-column-start", method = "start")]
    #[shorthand("grid-column-end", method = "end")]
    GridColumn(GridLines),
    #[property]
    #[shorthand("grid-row-start", method = "start")]
    #[shorthand("grid-row-end", method = "end")]
    GridRow(GridLines),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Text"]
pub enum Text {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    WhiteSpace(WhiteSpace),
    #[property]
    TextIndent(QuantityOrPercentage),
    #[property]
    TextAlign(TextAlign),
    #[property]
    Hyphens(Hyphens),
    /// https://www.w3.org/TR/css-writing-modes-3/#direction
    #[property]
    Direction(Direction),
    #[ua_property]
    LineBreak(LineBreak),
    /// Inherited like `hyphens`, so it lives in the same style struct.
    /// https://www.w3.org/TR/CSS22/tables.html#separated-borders
    #[property]
    BorderSpacing(BorderSpacing),
    /// Inherited like `border-spacing`.
    /// https://www.w3.org/TR/CSS22/tables.html#borders
    #[property]
    BorderCollapse(BorderCollapse),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Background"]
pub enum Background {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    BackgroundColor(Color),
    /// Only the `<color>` form of the shorthand is supported.
    #[property]
    #[shorthand("background-color", method = "clone")]
    Background(Color),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Page"]
pub enum Page {
    #[property]
    Orphans(usize),
    #[property]
    Widows(usize),
    #[property]
    PageBreakAfter(PageBreak),
    #[property]
    PageBreakBefore(PageBreak),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Math"]
pub enum Math {
    #[ua_property]
    MathVariant(MathFamily),
    #[ua_property]
    Form(MathOperatorForm),
    #[ua_property]
    OperatorIsStretchy(AutoBool),
    #[ua_property]
    MathStyle(MathStyle),
    #[property]
    MathDepth(MathDepth),
    #[property]
    MathShift(MathShift),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum Display {
    None,
    Block,
    Inline,
    InlineBlock,
    Flex,
    Grid,
    // https://www.w3.org/TR/CSS22/tables.html#table-display
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableCaption,
    TableColumnGroup,
    TableColumn,
}

impl Default for Display {
    fn default() -> Self {
        Display::Inline
    }
}

impl Display {
    /// Whether the box is block-level, i.e. it takes part in a block formatting context as a
    /// block (`block`, `flex`, `grid` and `table`).
    ///
    /// The internal table boxes (rows, cells, ...) count as block-level here as well: the box
    /// tree keeps them as block boxes, and the table layout picks them out of the children of
    /// the table.
    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            Display::Block
                | Display::Flex
                | Display::Grid
                | Display::Table
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableRow
                | Display::TableCell
                | Display::TableCaption
        )
    }

    /// Whether the box is an atomic inline-level box (`inline-block` and `inline-table`), whose
    /// contents are laid out on their own and which is placed on a line as a whole.
    pub fn is_atomic_inline(self) -> bool {
        matches!(self, Display::InlineBlock | Display::InlineTable)
    }

    /// Whether the box is a table (`table` or `inline-table`).
    pub fn is_table(self) -> bool {
        matches!(self, Display::Table | Display::InlineTable)
    }

    /// Whether the box is a row group (`table-row-group`, `table-header-group` or
    /// `table-footer-group`).
    pub fn is_table_row_group(self) -> bool {
        matches!(
            self,
            Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup
        )
    }

    /// Whether the box is a column or column group, which does not generate a box. The widths of
    /// the columns are taken from these elements by the table layout.
    pub fn is_table_column(self) -> bool {
        matches!(self, Display::TableColumn | Display::TableColumnGroup)
    }

    /// Whether the box is a flex or grid container, whose children are flex or grid items.
    pub fn is_flex_or_grid(self) -> bool {
        matches!(self, Display::Flex | Display::Grid)
    }
}

/// https://www.w3.org/TR/css-flexbox-1/#flex-direction-property
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// https://www.w3.org/TR/css-flexbox-1/#flex-wrap-property
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum FlexWrap {
    #[default]
    Nowrap,
    Wrap,
    WrapReverse,
}

/// The values of `justify-content` and `align-content`.
/// https://www.w3.org/TR/css-align-3/#propdef-justify-content
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum ContentAlignment {
    #[default]
    Normal,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

/// The values of `align-items` and `justify-items`.
/// https://www.w3.org/TR/css-align-3/#propdef-align-items
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum ItemAlignment {
    #[default]
    Normal,
    Stretch,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}

/// The values of `align-self` and `justify-self`.
/// https://www.w3.org/TR/css-align-3/#propdef-align-self
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum SelfAlignment {
    #[default]
    Auto,
    Normal,
    Stretch,
    Start,
    End,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
}

/// The value of the `flex` shorthand: `none | auto | <grow> <shrink>? <basis>? | <basis>`
/// https://www.w3.org/TR/css-flexbox-1/#flex-property
#[derive(Debug, Copy, Clone)]
pub struct FlexShorthand {
    pub grow: f32,
    pub shrink: f32,
    pub basis: AutoOrQuantityOrPercentage,
}

impl FlexShorthand {
    pub fn grow(&self) -> f32 {
        self.grow
    }
    pub fn shrink(&self) -> f32 {
        self.shrink
    }
    pub fn basis(&self) -> AutoOrQuantityOrPercentage {
        self.basis
    }
}

impl<'i> PropertyValue<'i> for FlexShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(FlexShorthand {
                grow: 0.0,
                shrink: 0.0,
                basis: AutoOrQuantityOrPercentage::Auto,
            });
        }
        if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(FlexShorthand {
                grow: 1.0,
                shrink: 1.0,
                basis: AutoOrQuantityOrPercentage::Auto,
            });
        }
        let grow = input.try_parse(f32::from_css_property_value).ok();
        let shrink = grow.and_then(|_| input.try_parse(f32::from_css_property_value).ok());
        // a unitless zero is a number, which is taken as the flex grow factor above
        let basis = input
            .try_parse(AutoOrQuantityOrPercentage::from_css_property_value)
            .ok();
        if grow.is_none() && basis.is_none() {
            return Err(input.new_error_for_next_token());
        }
        Ok(FlexShorthand {
            grow: grow.unwrap_or(1.0),
            shrink: shrink.unwrap_or(1.0),
            // an omitted basis is 0
            basis: basis.unwrap_or(AutoOrQuantityOrPercentage::Percentage(Percentage(0.0))),
        })
    }
}

/// The value of `border-spacing`: `<length> <length>?`. One length is the spacing in both
/// directions, with two lengths the first is the horizontal spacing.
#[derive(Debug, Copy, Clone)]
pub struct BorderSpacing {
    pub horizontal: Quantity,
    pub vertical: Quantity,
}

impl<'i> PropertyValue<'i> for BorderSpacing {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let horizontal = input
            .try_parse(Quantity::from_css_property_value)
            .map_err(|_| input.new_error_for_next_token())?;
        let vertical = input
            .try_parse(Quantity::from_css_property_value)
            .unwrap_or(horizontal);
        Ok(BorderSpacing {
            horizontal,
            vertical,
        })
    }
}

/// The value of `gap`. Only the form with one value (for both directions) is supported.
pub type Gap = QuantityOrPercentage;

/// A track size of `grid-template-columns`, `grid-template-rows`, `grid-auto-columns` and
/// `grid-auto-rows`.
/// https://www.w3.org/TR/css-grid-1/#track-sizing
#[derive(Debug, Copy, Clone)]
pub enum TrackSize {
    Auto,
    MinContent,
    MaxContent,
    /// A flexible length (`<number>fr`)
    Fraction(f32),
    Length(QuantityOrPercentage),
}

impl<'i> PropertyValue<'i> for TrackSize {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if let Ok(length) = input.try_parse(QuantityOrPercentage::from_css_property_value) {
            return Ok(TrackSize::Length(length));
        }
        let token = input.next()?.clone();
        match &token {
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("fr") => {
                Ok(TrackSize::Fraction(*value))
            }
            Token::Ident(ident) if ident.eq_ignore_ascii_case("auto") => Ok(TrackSize::Auto),
            Token::Ident(ident) if ident.eq_ignore_ascii_case("min-content") => {
                Ok(TrackSize::MinContent)
            }
            Token::Ident(ident) if ident.eq_ignore_ascii_case("max-content") => {
                Ok(TrackSize::MaxContent)
            }
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

/// A list of track sizes. `none` is the empty list.
#[derive(Debug, Clone, Default)]
pub struct TrackList(pub Vec<TrackSize>);

impl<'i> PropertyValue<'i> for TrackList {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(TrackList(vec![]));
        }
        Vec::<TrackSize>::from_css_property_value(input).map(TrackList)
    }
}

/// Where a grid item starts or ends: `auto | <integer> | span <integer>`
/// https://www.w3.org/TR/css-grid-1/#line-placement
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum GridPlacement {
    #[default]
    Auto,
    Line(i16),
    Span(u16),
}

impl<'i> PropertyValue<'i> for GridPlacement {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(GridPlacement::Auto);
        }
        if input.try_parse(|i| i.expect_ident_matching("span")).is_ok() {
            let span = i32::from_css_property_value(input)?;
            return Ok(GridPlacement::Span(span.clamp(1, u16::MAX as i32) as u16));
        }
        let line = i32::from_css_property_value(input)?;
        Ok(GridPlacement::Line(
            line.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        ))
    }
}

/// The value of the `grid-column` and `grid-row` shorthands: `<start> [ / <end> ]?`
#[derive(Debug, Copy, Clone)]
pub struct GridLines {
    pub start: GridPlacement,
    pub end: GridPlacement,
}

impl GridLines {
    pub fn start(&self) -> GridPlacement {
        self.start
    }
    pub fn end(&self) -> GridPlacement {
        self.end
    }
}

impl<'i> PropertyValue<'i> for GridLines {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let start = GridPlacement::from_css_property_value(input)?;
        let end = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
            GridPlacement::from_css_property_value(input)?
        } else {
            GridPlacement::Auto
        };
        Ok(GridLines { start, end })
    }
}

/// https://www.w3.org/TR/css-grid-1/#grid-auto-flow-property
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Unit {
    Px,
    Cm,
    Mm,
    Pt,
    Pc,
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
        match_ignore_ascii_case! { s,
            "px" => Ok(Unit::Px),
            "cm" => Ok(Unit::Cm),
            "pt" => Ok(Unit::Pt),
            "pc" => Ok(Unit::Pc),
            "em" => Ok(Unit::Em),
            "mm" => Ok(Unit::Mm),
            "in" => Ok(Unit::In),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct List<'a, T: 'a>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    elements: Cow<'a, [T]>,
}

impl<'a, 'i, T: PropertyValue<'i>> PropertyValue<'i> for List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    type Error = T::Error;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<T::Error>> {
        // TODO: Error if zero elements.
        Vec::<T>::from_css_property_value(input).map(|elements| List {
            elements: Cow::Owned(elements),
        })
    }
}

impl<'a, T> List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    pub fn elements(&self) -> &[T] {
        &self.elements
    }

    fn make_ref(&'a self) -> Self {
        List {
            elements: Cow::Borrowed(&self.elements),
        }
    }
}

// TODO: Don't panic!
impl<'a, T: Copy + PropertyValue<'a>> List<'a, T>
where
    [T]: ToOwned<Owned = Vec<T>>,
{
    fn top(&self) -> T {
        match self.elements.len() {
            1 | 2 | 3 | 4 => self.elements[0],
            _ => panic!(),
        }
    }

    fn bottom(&self) -> T {
        match self.elements.len() {
            1 | 2 => self.elements[0],
            3 | 4 => self.elements[2],
            _ => panic!(),
        }
    }

    fn left(&self) -> T {
        match self.elements.len() {
            1 => self.elements[0],
            2 | 3 => self.elements[1],
            4 => self.elements[3],
            _ => panic!(),
        }
    }

    fn right(&self) -> T {
        match self.elements.len() {
            1 => self.elements[0],
            2 | 3 | 4 => self.elements[1],
            _ => panic!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommaSeparated<T> {
    elements: Vec<T>,
}

impl<'i, T: PropertyValue<'i>> PropertyValue<'i> for CommaSeparated<T> {
    type Error = T::Error;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<T::Error>> {
        input
            .parse_comma_separated(|input| T::from_css_property_value(input))
            .map(|elements| CommaSeparated { elements })
    }
}

impl<T: Clone> ComputedValue<CommaSeparated<T>> for Vec<T> {
    fn from_specified(
        comma_separated: &CommaSeparated<T>,
        _: &super::computed::ComputedValues,
    ) -> Self {
        comma_separated.elements.clone()
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Quantity {
    pub value: super::CssFloat,
    pub unit: Unit,
}

impl<'i> PropertyValue<'i> for Quantity {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<()>> {
        let token = input.next()?;
        let (value, unit) = match token {
            Token::Dimension { value, unit, .. } => (*value, unit.parse()),
            Token::Number {
                int_value: Some(val),
                ..
            } if *val == 0 => (0.0, Ok(Unit::Px)),
            _ => {
                let _token = token.clone();
                return Err(ParseError::unexpected_token());
            }
        };

        let unit = match unit {
            Err(_) => {
                let _token = token.clone();
                return Err(ParseError::unexpected_token());
            }
            Ok(unit) => unit,
        };

        Ok(Quantity { value, unit })
    }
}

/// A value between 0 and 1 which is interpreted as a relative size.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Default)]
pub struct Percentage(pub super::CssFloat);

impl<'i> PropertyValue<'i> for Percentage {
    type Error = ();

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<()>> {
        let token = input.next()?;
        match token {
            Token::Percentage { unit_value, .. } => Ok(Percentage(*unit_value)),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum QuantityOrPercentage {
    Quantity(Quantity),
    Percentage(Percentage),
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum AutoOrQuantityOrPercentage {
    Auto,
    Quantity(Quantity),
    Percentage(Percentage),
}

impl Default for AutoOrQuantityOrPercentage {
    fn default() -> Self {
        AutoOrQuantityOrPercentage::Auto
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum BorderWidth {
    Thin,
    Medium,
    Thick,
    Quantity(Quantity),
}

impl Default for BorderWidth {
    fn default() -> BorderWidth {
        BorderWidth::Medium
    }
}

impl ComputedValue<BorderWidth> for super::computed::PixelLength {
    fn from_specified(bw: &BorderWidth, style: &super::computed::ComputedValues) -> Self {
        match bw {
            BorderWidth::Thin => PixelLength(1.0),
            BorderWidth::Medium => PixelLength(3.0),
            BorderWidth::Thick => PixelLength(5.0),
            &BorderWidth::Quantity(Quantity { value, unit }) => {
                PixelLength::from_val_unit(value, unit, style.font.font_size)
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum FontSize {
    Quantity(Quantity),
    Percentage(Percentage),
    Smaller,
    Larger,
    /// `font-size: math`. Scales the font size according to the change in
    /// `math-depth`; resolved in `StyleBuilder::build` once the final depth is
    /// known, so `from_specified` below only sees a placeholder.
    Math,
}

impl ComputedValue<FontSize> for super::computed::PixelLength {
    fn from_specified(font_size: &FontSize, style: &super::computed::ComputedValues) -> Self {
        match *font_size {
            FontSize::Quantity(Quantity { value, unit }) => {
                PixelLength::from_val_unit(value, unit, style.font.font_size)
            }
            FontSize::Percentage(percentage) => percentage.0 * style.font.font_size,
            FontSize::Smaller => 0.8 * style.font.font_size,
            FontSize::Larger => 1.2 * style.font.font_size,
            FontSize::Math => style.font.font_size,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum WhiteSpace {
    Normal,
    Pre,
}

impl Default for WhiteSpace {
    fn default() -> Self {
        WhiteSpace::Normal
    }
}

/// https://www.w3.org/TR/CSS22/tables.html#propdef-border-collapse
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum Hyphens {
    None,
    Manual,
    Auto,
}

impl Default for Hyphens {
    fn default() -> Self {
        Self::Manual
    }
}

/// The lines of `text-decoration-line`. Styles and colors of the lines are not supported and
/// ignored.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct TextDecoration {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

impl TextDecoration {
    pub fn is_none(self) -> bool {
        self == TextDecoration::default()
    }
}

impl<'i> PropertyValue<'i> for TextDecoration {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut result = TextDecoration::default();
        // at least one component is required
        let mut token = input.next()?.clone();
        loop {
            if let Token::Ident(ident) = &token {
                match &*ident.to_ascii_lowercase() {
                    "underline" => result.underline = true,
                    "overline" => result.overline = true,
                    "line-through" => result.line_through = true,
                    // `none` and the unsupported style and color components
                    _ => {}
                }
            }
            if input.is_exhausted() {
                return Ok(result);
            }
            token = input.next()?.clone();
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum ListStyleType {
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    None,
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Disc
    }
}

impl ListStyleType {
    /// The text of the marker of the `ordinal`th (1-based) list item, without any spacing.
    /// The ordinal can be zero or negative when `<li value>` or a negative `<ol start>` says so.
    pub fn marker_text(self, ordinal: i64) -> Option<String> {
        fn alpha(mut n: usize, first: u8) -> String {
            let mut letters = vec![];
            while n > 0 {
                n -= 1;
                letters.push(first + (n % 26) as u8);
                n /= 26;
            }
            letters.reverse();
            String::from_utf8(letters).unwrap()
        }
        fn roman(mut n: usize) -> String {
            const NUMERALS: [(usize, &str); 13] = [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ];
            let mut out = String::new();
            for &(value, numeral) in &NUMERALS {
                while n >= value {
                    out.push_str(numeral);
                    n -= value;
                }
            }
            out
        }
        Some(match self {
            ListStyleType::Disc => "\u{2022}".to_string(),
            ListStyleType::Circle => "\u{25E6}".to_string(),
            ListStyleType::Square => "\u{25AA}".to_string(),
            ListStyleType::None => return None,
            // roman numerals and letters are only defined for positive numbers
            ListStyleType::Decimal => format!("{}.", ordinal),
            _ if ordinal <= 0 => format!("{}.", ordinal),
            ListStyleType::LowerAlpha => format!("{}.", alpha(ordinal as usize, b'a')),
            ListStyleType::UpperAlpha => format!("{}.", alpha(ordinal as usize, b'A')),
            ListStyleType::LowerRoman => format!("{}.", roman(ordinal as usize).to_lowercase()),
            ListStyleType::UpperRoman => format!("{}.", roman(ordinal as usize)),
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum TextAlign {
    Left,
    Justify,
    Center,
    Right,
    /// `left` in left-to-right and `right` in right-to-left paragraphs.
    Start,
    /// `right` in left-to-right and `left` in right-to-left paragraphs.
    End,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Start
    }
}

impl TextAlign {
    /// Replaces `start` and `end` by `left` or `right` for a paragraph of the given direction.
    pub fn resolve(self, rtl: bool) -> TextAlign {
        match (self, rtl) {
            (TextAlign::Start, false) | (TextAlign::End, true) => TextAlign::Left,
            (TextAlign::Start, true) | (TextAlign::End, false) => TextAlign::Right,
            (align, _) => align,
        }
    }
}

/// The inline base direction of an element.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// How an element takes part in the Unicode bidirectional algorithm.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}

impl UnicodeBidi {
    /// The bidi control characters that are inserted at the start and the end of an inline
    /// element with this value and the given direction
    /// (https://www.w3.org/TR/css-writing-modes-3/#bidi-control-codes-injection-table).
    pub fn control_characters(self, direction: Direction) -> Option<(&'static str, &'static str)> {
        let rtl = direction == Direction::Rtl;
        Some(match self {
            UnicodeBidi::Normal => return None,
            UnicodeBidi::Embed if rtl => ("\u{202B}", "\u{202C}"),
            UnicodeBidi::Embed => ("\u{202A}", "\u{202C}"),
            UnicodeBidi::Isolate if rtl => ("\u{2067}", "\u{2069}"),
            UnicodeBidi::Isolate => ("\u{2066}", "\u{2069}"),
            UnicodeBidi::BidiOverride if rtl => ("\u{202E}", "\u{202C}"),
            UnicodeBidi::BidiOverride => ("\u{202D}", "\u{202C}"),
            UnicodeBidi::IsolateOverride if rtl => ("\u{2067}\u{202E}", "\u{202C}\u{2069}"),
            UnicodeBidi::IsolateOverride => ("\u{2066}\u{202D}", "\u{202C}\u{2069}"),
            UnicodeBidi::Plaintext => ("\u{2068}", "\u{2069}"),
        })
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum BorderStyle {
    None,
    Solid,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::None
    }
}

impl<'i> PropertyValue<'i> for Color {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        Color::parse(input).map_err(|error| match error.kind {
            cssparser::ParseErrorKind::Basic(kind) => ParseError::from_basic_kind(kind),
            cssparser::ParseErrorKind::Custom(()) => ParseError::unexpected_token(),
        })
    }
}

impl<'i> PropertyValue<'i> for f32 {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let token = input.next()?;
        match token {
            Token::Number { value, .. } => Ok(*value),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

impl<'i> PropertyValue<'i> for usize {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let token = input.next()?;
        match token {
            Token::Number {
                int_value: Some(val),
                ..
            } => Ok(*val as usize),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

impl<'i> PropertyValue<'i> for i32 {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let token = input.next()?;
        match token {
            Token::Number {
                int_value: Some(val),
                ..
            } => Ok(*val),
            _ => {
                let _token = token.clone();
                Err(ParseError::unexpected_token())
            }
        }
    }
}

impl<'i> PropertyValue<'i> for bool {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let val = input.expect_ident_or_string()?;
        if val.eq_ignore_ascii_case("true") {
            Ok(true)
        } else if val.eq_ignore_ascii_case("false") {
            Ok(false)
        } else {
            let _val = val.clone();
            Err(ParseError::unexpected_token())
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum AutoBool {
    True,
    False,
    Auto,
}

impl Default for AutoBool {
    fn default() -> Self {
        AutoBool::Auto
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum Border {
    Width(BorderWidth),
    Style(BorderStyle),
    Color(Color),
}

impl<'a> List<'a, Border> {
    fn width(&self) -> BorderWidth {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Width(width) => Some(*width),
                _ => None,
            })
            .next()
            .unwrap_or_default()
    }

    fn style(&self) -> BorderStyle {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Style(style) => Some(*style),
                _ => None,
            })
            .next()
            .unwrap_or_default()
    }

    fn color(&self) -> Color {
        self.elements
            .iter()
            .rev()
            .filter_map(|border| match border {
                Border::Color(color) => Some(*color),
                _ => None,
            })
            .next()
            .unwrap_or(Color::CurrentColor)
    }
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum LineHeight {
    QuantityOrPercentage(QuantityOrPercentage),
    Number(f32),
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

impl Default for FontStyle {
    fn default() -> FontStyle {
        FontStyle::Normal
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum FontWeight {
    Normal,
    Bold,
}

impl Default for FontWeight {
    fn default() -> FontWeight {
        FontWeight::Normal
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum PageBreak {
    Auto,
    Always,
}

impl Default for PageBreak {
    fn default() -> Self {
        PageBreak::Auto
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathFamily {
    Normal,
    Italic,
    Bold,
    BoldItalic,
    Script,
    BoldScript,
    Fraktur,
    DoubleStruck,
    BoldFraktur,
    SansSerif,
    BoldSansSerif,
    SansSerifItalic,
    SansSerifBoldItalic,
    Monospace,
}

impl Default for MathFamily {
    fn default() -> Self {
        MathFamily::Normal
    }
}

impl From<MathFamily> for MathFamily2 {
    fn from(fam: MathFamily) -> MathFamily2 {
        match fam {
            MathFamily::Normal => MathFamily2::Normal,
            MathFamily::Italic => MathFamily2::Italics,
            MathFamily::Bold => MathFamily2::Bold,
            MathFamily::BoldItalic => MathFamily2::BoldItalics,
            MathFamily::Script => MathFamily2::Script,
            MathFamily::BoldScript => MathFamily2::BoldScript,
            MathFamily::Fraktur => MathFamily2::Fraktur,
            MathFamily::DoubleStruck => MathFamily2::DoubleStruck,
            MathFamily::BoldFraktur => MathFamily2::BoldFraktur,
            MathFamily::SansSerif => MathFamily2::SansSerif,
            MathFamily::BoldSansSerif => MathFamily2::SansSerifBold,
            MathFamily::SansSerifItalic => MathFamily2::SansSerifItalics,
            MathFamily::SansSerifBoldItalic => MathFamily2::SansSerifBoldItalics,
            MathFamily::Monospace => MathFamily2::Monospace,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum PositionSpec {
    Static,
    Absolute,
}

impl Default for PositionSpec {
    fn default() -> Self {
        PositionSpec::Static
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathOperatorForm {
    Auto,
    Prefix,
    Infix,
    Postfix,
}

impl Default for MathOperatorForm {
    fn default() -> MathOperatorForm {
        MathOperatorForm::Auto
    }
}

impl From<MathOperatorForm> for Option<math_render::host::Form> {
    fn from(other: MathOperatorForm) -> Self {
        match other {
            MathOperatorForm::Auto => None,
            MathOperatorForm::Prefix => Some(math_render::host::Form::Prefix),
            MathOperatorForm::Infix => Some(math_render::host::Form::Infix),
            MathOperatorForm::Postfix => Some(math_render::host::Form::Postfix),
        }
    }
}

/// The `math-depth` property: `auto-add | add(<integer>) | <integer>`.
///
/// https://www.w3.org/TR/css-fonts-4/#math-depth-prop
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathDepth {
    AutoAdd,
    #[function]
    Add(i32),
    Absolute(i32),
}

impl Default for MathDepth {
    fn default() -> MathDepth {
        MathDepth::Absolute(0)
    }
}

/// The `math-shift` property: `normal | compact`.
///
/// https://w3c.github.io/mathml-core/#the-math-shift
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathShift {
    Normal,
    Compact,
}

impl Default for MathShift {
    fn default() -> MathShift {
        MathShift::Normal
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum MathStyle {
    Inline,
    Display,
}

impl Default for MathStyle {
    fn default() -> MathStyle {
        MathStyle::Inline
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PropertyValue)]
pub enum LineBreak {
    Normal,
    Force,
}

impl Default for LineBreak {
    fn default() -> LineBreak {
        LineBreak::Normal
    }
}

#[cfg(test)]
mod function_syntax_tests {
    use super::*;

    #[derive(Debug, PartialEq, PropertyValue)]
    enum Test {
        Plain,
        #[function(comma)]
        Pair(i32, i32),
        #[function(comma, name = "opt")]
        Optional(i32, Option<i32>),
        #[function]
        Spaced(i32, i32),
    }

    fn parse<T: for<'i> PropertyValue<'i, Error = StyleParseError>>(css: &str) -> Option<T> {
        let mut parser = Parser::new(css);
        parser
            .parse_entirely(|p| T::from_css_property_value(p))
            .ok()
    }

    #[test]
    fn math_depth() {
        assert_eq!(parse("auto-add"), Some(MathDepth::AutoAdd));
        assert_eq!(parse("AUTO-ADD"), Some(MathDepth::AutoAdd));
        assert_eq!(parse("add(2)"), Some(MathDepth::Add(2)));
        assert_eq!(parse("ADD(-1)"), Some(MathDepth::Add(-1)));
        assert_eq!(parse("3"), Some(MathDepth::Absolute(3)));
        assert_eq!(parse("-3"), Some(MathDepth::Absolute(-3)));
        assert_eq!(parse::<MathDepth>("1.5"), None);
        assert_eq!(parse::<MathDepth>("add(1.5)"), None);
        assert_eq!(parse::<MathDepth>("add(1 2)"), None);
        assert_eq!(parse::<MathDepth>("add()"), None);
        assert_eq!(parse::<MathDepth>("add"), None);
    }

    #[test]
    fn function_arguments() {
        assert_eq!(parse("plain"), Some(Test::Plain));
        assert_eq!(parse("pair(1, 2)"), Some(Test::Pair(1, 2)));
        assert_eq!(parse::<Test>("pair(1 2)"), None);
        assert_eq!(parse::<Test>("pair(1, 2, 3)"), None);
        assert_eq!(parse::<Test>("pair(1)"), None);
        assert_eq!(parse("opt(1)"), Some(Test::Optional(1, None)));
        assert_eq!(parse("opt(1, 2)"), Some(Test::Optional(1, Some(2))));
        assert_eq!(parse::<Test>("opt(1,)"), None);
        assert_eq!(parse("spaced(1 2)"), Some(Test::Spaced(1, 2)));
        assert_eq!(parse::<Test>("spaced(1, 2)"), None);
    }
}
