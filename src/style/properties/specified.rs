use std::borrow::{Cow, ToOwned};
use std::str::FromStr;

use super::super::parse::StyleParseError;
use super::computed::PixelLength;
pub use super::font_features::{
    FontFeatureSettings, FontKerning, FontVariant, FontVariantCaps, FontVariantEastAsian,
    FontVariantLigatures, FontVariantNumeric, FontVariantPosition,
};
use cssparser::{CowRcStr, ParseError, Parser, Token, match_ignore_ascii_case};
use cssparser_color::Color;
use helium_derive::{ParseDeclaration, PropertyValue};

use formel::unicode_math::Family as MathFamily2;

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
        "list-style-type", "list-style-position", "letter-spacing", "word-spacing", "white-space", "text-indent", "text-align", "hyphens", "direction",
        "border-spacing", "border-collapse", "quotes", "widows", "orphans", "looseness", "page", "math-style", "math-shift",
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
            || self.property.eq_ignore_ascii_case("all")
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
            // (`all` resets everything but `direction`, `unicode-bidi` and the custom properties)
            || (self.property.eq_ignore_ascii_case("all") && name != "direction" && name != "unicode-bidi")
    }

    pub fn apply(
        &self,
        computed: &mut super::computed::ComputedValues,
        parent: &super::computed::ComputedValues,
    ) {
        let initial = super::computed::ComputedValues::default();
        if self.property.eq_ignore_ascii_case("all") {
            // every property but `direction`, `unicode-bidi` and the custom properties
            let source = match self.keyword {
                CssWide::Inherit => parent.clone(),
                CssWide::Initial => initial,
                CssWide::Unset => super::computed::StyleBuilder::unset_values(parent),
            };
            let (direction, bidi) = (computed.text.direction, computed.box_.unicode_bidi);
            let (lang, root_font_size) = (computed.font.lang.clone(), computed.font.root_font_size);
            computed.box_ = source.box_;
            computed.position = source.position;
            computed.font = source.font;
            computed.text = source.text;
            computed.background = source.background;
            computed.page = source.page;
            computed.math = source.math;
            std::sync::Arc::make_mut(&mut computed.text).direction = direction;
            std::sync::Arc::make_mut(&mut computed.box_).unicode_bidi = bidi;
            let font = std::sync::Arc::make_mut(&mut computed.font);
            font.lang = lang;
            font.root_font_size = root_font_size;
        } else if self.property.eq_ignore_ascii_case("font") {
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

/// The declaration of a custom property, `--name: value`. The cascade of an element computes its
/// custom properties before it applies the other declarations (`StyleBuilder::apply_cascade`).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomDeclaration {
    /// With the leading `--`.
    pub name: String,
    /// The text of the value, without `!important`.
    pub value: String,
}

/// The declaration of a property whose value uses `var()`. It can only be parsed when the custom
/// properties of the element are known, so the text is kept until then.
#[derive(Debug, Clone, PartialEq)]
pub struct UnparsedDeclaration {
    pub name: String,
    pub value: String,
}

macro_rules! text_declaration {
    ($ty:ident) => {
        impl<'i> ParseDeclaration<'i> for $ty {
            type Error = StyleParseError;

            /// Made by the style sheet parser, which knows the text of the value.
            fn parse_declaration(
                name: CowRcStr<'i>,
                _input: &mut Parser<'i>,
            ) -> Result<Self, ParseError<StyleParseError>> {
                Err(ParseError::custom(StyleParseError::UnknownProperty { name: name.to_string() }))
            }
        }

        impl ApplyToComputedValues for $ty {
            fn apply_to(&self, _: &mut super::computed::ComputedValues) {}
        }

        impl CssWideProperties for $ty {
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
    };
}

text_declaration!(CustomDeclaration);
text_declaration!(UnparsedDeclaration);

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
    Logical(super::logical::Logical),
    CssWide(CssWideDeclaration),
    Custom(CustomDeclaration),
    Unparsed(UnparsedDeclaration),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Position"]
pub enum Position<'a> {
    #[property]
    Width(AutoOrQuantityOrPercentage),
    #[property]
    Height(AutoOrQuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/visudet.html#min-max-widths
    #[property]
    MinWidth(AutoOrQuantityOrPercentage),
    #[property]
    MaxWidth(NoneOrQuantityOrPercentage),
    /// https://www.w3.org/TR/CSS22/visudet.html#min-max-heights
    #[property]
    MinHeight(AutoOrQuantityOrPercentage),
    #[property]
    MaxHeight(NoneOrQuantityOrPercentage),

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
    /// `inset: <top> <right>? <bottom>? <left>?`
    /// https://www.w3.org/TR/css-position-3/#propdef-inset
    #[property]
    #[shorthand("top", method = "top")]
    #[shorthand("bottom", method = "bottom")]
    #[shorthand("left", method = "left")]
    #[shorthand("right", method = "right")]
    Inset(List<'a, AutoOrQuantityOrPercentage>),

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

    /// https://www.w3.org/TR/css-backgrounds-3/#border-radius
    #[property]
    BorderTopLeftRadius(QuantityOrPercentage),
    #[property]
    BorderTopRightRadius(QuantityOrPercentage),
    #[property]
    BorderBottomRightRadius(QuantityOrPercentage),
    #[property]
    BorderBottomLeftRadius(QuantityOrPercentage),
    #[property]
    #[shorthand("border-top-left-radius", method = "top_left")]
    #[shorthand("border-top-right-radius", method = "top_right")]
    #[shorthand("border-bottom-right-radius", method = "bottom_right")]
    #[shorthand("border-bottom-left-radius", method = "bottom_left")]
    BorderRadius(BorderRadiusShorthand),

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
    /// https://www.w3.org/TR/css-fonts-4/#font-feature-settings-prop
    #[property]
    FontFeatureSettings(FontFeatureSettings),
    #[property]
    FontKerning(FontKerning),
    #[property]
    FontVariantLigatures(FontVariantLigatures),
    #[property]
    FontVariantCaps(FontVariantCaps),
    #[property]
    FontVariantNumeric(FontVariantNumeric),
    #[property]
    FontVariantEastAsian(FontVariantEastAsian),
    #[property]
    FontVariantPosition(FontVariantPosition),
    /// https://www.w3.org/TR/css-fonts-4/#font-variant-prop
    #[property]
    #[shorthand("font-variant-ligatures", method = "ligatures")]
    #[shorthand("font-variant-caps", method = "caps")]
    #[shorthand("font-variant-numeric", method = "numeric")]
    #[shorthand("font-variant-east-asian", method = "east_asian")]
    #[shorthand("font-variant-position", method = "position")]
    FontVariant(FontVariant),
    /// https://www.w3.org/TR/css-fonts-4/#font-prop. The system font keywords and `font-stretch`
    /// are not supported (the latter is accepted and ignored). `font-variant` is the one of CSS 2
    /// (`small-caps`); like the other longhands that the shorthand resets, the other
    /// `font-variant-*` properties and `font-feature-settings` go back to their initial values.
    #[property]
    #[shorthand("font-style", method = "style")]
    #[shorthand("font-weight", method = "weight")]
    #[shorthand("font-size", method = "size")]
    #[shorthand("line-height", method = "line_height")]
    #[shorthand("font-family", method = "family")]
    #[shorthand("font-variant-caps", method = "caps")]
    #[shorthand("font-variant-ligatures", method = "ligatures")]
    #[shorthand("font-variant-numeric", method = "numeric")]
    #[shorthand("font-variant-east-asian", method = "east_asian")]
    #[shorthand("font-variant-position", method = "position")]
    #[shorthand("font-feature-settings", method = "feature_settings")]
    #[shorthand("font-kerning", method = "kerning")]
    Font(FontShorthand),
    /// Inherited like the font properties, so it lives in the same style struct.
    #[property]
    Color(Color),
    /// Inherited, so it lives here as well.
    #[property]
    ListStyleType(ListStyleType),
    /// Inherited too.
    #[property]
    ListStylePosition(ListStylePosition),
    /// The marker type and position in one declaration. Markers are always outside of their
    /// item, so the position is accepted and not used.
    #[property]
    #[shorthand("list-style-type", method = "style_type")]
    #[shorthand("list-style-position", method = "position")]
    ListStyle(ListStyleShorthand),
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
    /// https://www.w3.org/TR/CSS22/visudet.html#propdef-vertical-align
    #[property]
    VerticalAlign(VerticalAlign),
    /// https://www.w3.org/TR/css-sizing-4/#aspect-ratio
    #[property]
    AspectRatio(AspectRatio),
    /// https://www.w3.org/TR/css-sizing-3/#box-sizing
    #[property]
    BoxSizing(BoxSizing),

    // Generated content: https://www.w3.org/TR/css-content-3/ and
    // https://www.w3.org/TR/css-lists-3/#auto-numbering
    #[property]
    Content(super::generated::Content),
    #[property]
    CounterReset(super::generated::CounterList),
    #[property]
    CounterIncrement(super::generated::CounterList),
    #[property]
    CounterSet(super::generated::CounterList),

    // Flexbox: https://www.w3.org/TR/css-flexbox-1/
    #[property]
    FlexDirection(FlexDirection),
    #[property]
    FlexWrap(FlexWrap),
    /// `flex-flow: <flex-direction> || <flex-wrap>`
    #[property]
    #[shorthand("flex-direction", method = "direction")]
    #[shorthand("flex-wrap", method = "wrap")]
    FlexFlow(FlexFlowShorthand),
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

    /// https://www.w3.org/TR/css-flexbox-1/#order-property
    #[property]
    Order(i32),

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
    /// Also the gap between the columns of a multi-column container.
    /// https://www.w3.org/TR/css-multicol-1/#column-gaps-and-rules
    #[property]
    ColumnGap(Gap),
    /// `place-content: <align-content> <justify-content>?`
    #[property]
    #[shorthand("align-content", method = "align")]
    #[shorthand("justify-content", method = "justify")]
    PlaceContent(PlaceContentShorthand),
    /// `place-items: <align-items> <justify-items>?`
    #[property]
    #[shorthand("align-items", method = "align")]
    #[shorthand("justify-items", method = "justify")]
    PlaceItems(PlaceItemsShorthand),
    /// `place-self: <align-self> <justify-self>?`
    #[property]
    #[shorthand("align-self", method = "align")]
    #[shorthand("justify-self", method = "justify")]
    PlaceSelf(PlaceSelfShorthand),
    /// `gap: <row-gap> <column-gap>?` (the one value is for both)
    #[property]
    #[shorthand("row-gap", method = "row")]
    #[shorthand("column-gap", method = "column")]
    Gap(GapShorthand),
    /// The names that `gap` and its longhands had in the grid layout module.
    #[property]
    #[shorthand("row-gap", method = "row")]
    #[shorthand("column-gap", method = "column")]
    GridGap(GapShorthand),
    #[property]
    #[shorthand("row-gap", method = "clone")]
    GridRowGap(Gap),
    #[property]
    #[shorthand("column-gap", method = "clone")]
    GridColumnGap(Gap),

    // Multi-column layout: https://www.w3.org/TR/css-multicol-1/
    #[property]
    ColumnCount(ColumnCount),
    #[property]
    ColumnWidth(ColumnWidth),
    /// `columns: <column-width> || <column-count>`
    #[property]
    #[shorthand("column-width", method = "width")]
    #[shorthand("column-count", method = "count")]
    Columns(ColumnsShorthand),
    /// Parsed. `auto` balances the columns like `balance`.
    #[property]
    ColumnFill(ColumnFill),
    /// Parsed and not used: a box does not span the columns.
    #[property]
    ColumnSpan(ColumnSpan),
    #[property]
    ColumnRuleWidth(BorderWidth),
    #[property]
    ColumnRuleStyle(BorderStyle),
    #[property]
    ColumnRuleColor(Color),
    #[property]
    #[shorthand("column-rule-width", method = "width")]
    #[shorthand("column-rule-style", method = "style")]
    #[shorthand("column-rule-color", method = "color")]
    ColumnRule(ColumnRuleShorthand),

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
    /// https://www.w3.org/TR/css-transforms-1/
    #[property]
    Transform(super::transform::Transform),
    #[property]
    TransformOrigin(super::transform::TransformOrigin),
    /// https://www.w3.org/TR/css-color-4/#transparency
    #[property]
    Opacity(Opacity),
    /// https://www.w3.org/TR/css-backgrounds-3/#box-shadow
    #[property]
    BoxShadow(ShadowList),
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
    #[property]
    #[shorthand("grid-row-start", method = "row_start")]
    #[shorthand("grid-column-start", method = "column_start")]
    #[shorthand("grid-row-end", method = "row_end")]
    #[shorthand("grid-column-end", method = "column_end")]
    GridArea(GridAreaLines),
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
    /// https://www.w3.org/TR/css-text-3/#letter-spacing-property
    #[property]
    LetterSpacing(NormalOrQuantity),
    /// https://www.w3.org/TR/css-text-3/#word-spacing-property
    #[property]
    WordSpacing(NormalOrQuantity),
    /// https://www.w3.org/TR/css-text-3/#text-transform-property
    #[property]
    TextTransform(TextTransform),
    /// https://www.w3.org/TR/css-writing-modes-3/#direction
    #[property]
    Direction(Direction),
    /// https://www.w3.org/TR/css-writing-modes-3/#block-flow
    #[property]
    WritingMode(WritingMode),
    /// https://www.w3.org/TR/css-writing-modes-3/#text-orientation
    #[property]
    TextOrientation(TextOrientation),
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
    /// Inherited, for the quotation marks of `open-quote` and `close-quote`.
    /// https://www.w3.org/TR/css-content-3/#quotes
    #[property]
    Quotes(super::generated::Quotes),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Background"]
pub enum Background {
    /// https://www.w3.org/TR/CSS22/text.html#white-space-prop
    #[property]
    BackgroundColor(Color),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-image
    #[property]
    BackgroundImage(CommaSeparated<super::gradient::SpecifiedImage>),
    /// https://www.w3.org/TR/compositing-1/#background-blend-mode
    #[property]
    BackgroundBlendMode(CommaSeparated<BlendMode>),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-clip
    #[property]
    BackgroundClip(CommaSeparated<BackgroundBox>),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-origin
    #[property]
    BackgroundOrigin(CommaSeparated<BackgroundBox>),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-size
    #[property]
    BackgroundSize(CommaSeparated<super::background::SpecifiedSize>),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-position
    #[property]
    BackgroundPosition(CommaSeparated<super::background::SpecifiedPosition>),
    /// https://www.w3.org/TR/css-backgrounds-3/#background-repeat
    #[property]
    BackgroundRepeat(CommaSeparated<super::background::Repeat>),
    /// The shorthand supports the colour, the images with position, size, repeat, origin and clip;
    /// not `-attachment`. It resets `background-blend-mode`.
    #[property]
    #[shorthand("background-color", method = "color")]
    #[shorthand("background-image", method = "images")]
    #[shorthand("background-blend-mode", method = "blend_modes")]
    #[shorthand("background-origin", method = "origins")]
    #[shorthand("background-clip", method = "clips")]
    #[shorthand("background-size", method = "sizes")]
    #[shorthand("background-position", method = "positions")]
    #[shorthand("background-repeat", method = "repeats")]
    Background(super::gradient::BackgroundShorthand),
}

#[derive(Debug, Clone, ParseDeclaration)]
#[computed = "super::computed::Page"]
pub enum Page {
    /// https://www.w3.org/TR/css-page-3/#page-size-prop
    #[property]
    Size(PageSize),
    /// https://www.w3.org/TR/css-page-3/#using-named-pages
    #[property]
    Page(PageName),
    #[property]
    Orphans(usize),
    #[property]
    Widows(usize),
    /// Not in CSS. The number of lines more or fewer than it has when it is broken as well as it
    /// can be that a paragraph may have, so that a page that fits it better can take it (TeX's
    /// `\looseness`, for the pages to choose, and in both directions). 0, the initial value, is the
    /// way that the paragraph is broken as well as it can be.
    #[property]
    Looseness(usize),
    /// https://www.w3.org/TR/css-gcpm-3/#bookmark-level
    #[property]
    BookmarkLevel(BookmarkLevel),
    /// https://www.w3.org/TR/css-gcpm-3/#bookmark-label
    #[property]
    BookmarkLabel(BookmarkLabel),
    /// https://www.w3.org/TR/css-gcpm-3/#bookmark-state
    #[property]
    BookmarkState(BookmarkState),
    /// https://www.w3.org/TR/css-break-3/#break-between
    #[property]
    BreakBefore(Break),
    #[property]
    BreakAfter(Break),
    /// https://www.w3.org/TR/css-break-3/#break-within
    #[property]
    BreakInside(BreakInside),
    /// The CSS 2 names of `break-before`, `break-after` and `break-inside`.
    /// https://www.w3.org/TR/CSS22/page.html#page-break-props
    #[property]
    #[shorthand("break-before", method = "to_break")]
    PageBreakBefore(LegacyBreak),
    #[property]
    #[shorthand("break-after", method = "to_break")]
    PageBreakAfter(LegacyBreak),
    #[property]
    #[shorthand("break-inside", method = "to_break_inside")]
    PageBreakInside(LegacyBreakInside),
    /// Float placement in the manner of TeX: `top` and `bottom` of CSS GCPM, for a block-level box
    /// between the blocks of a block container.
    /// https://www.w3.org/TR/css-gcpm-3/#float-positions
    #[property]
    Float(Float),
    /// What `float: top` and `bottom` place a float relative to: the page, or the column that
    /// the float is called for in (only in a container with columns that is split over pages).
    /// https://www.w3.org/TR/css-gcpm-3/#float-reference
    #[property]
    FloatReference(FloatReference),
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

/// The box that the background is painted into (`background-clip`).
/// https://www.w3.org/TR/css-backgrounds-3/#typedef-bg-clip
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum BackgroundBox {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
}

/// https://www.w3.org/TR/compositing-1/#ltblendmodegt
#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum Display {
    None,
    Block,
    /// A block box that counts in the `list-item` counter and has a marker.
    ListItem,
    /// The element has no box of its own: its children (and its pseudo-elements) take its place.
    Contents,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
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
                | Display::ListItem
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

    /// Whether the box is an atomic inline-level box (`inline-block`, `inline-table`,
    /// `inline-flex` and `inline-grid`), whose contents are laid out on their own and which is
    /// placed on a line as a whole.
    pub fn is_atomic_inline(self) -> bool {
        matches!(
            self,
            Display::InlineBlock | Display::InlineTable | Display::InlineFlex | Display::InlineGrid
        )
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

    /// Whether the box is a flex or grid container (also an inline one), whose children are flex
    /// or grid items.
    pub fn is_flex_or_grid(self) -> bool {
        matches!(
            self,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        )
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
    /// The start and end of the box itself, which can be another side than the container's.
    SelfStart,
    SelfEnd,
    /// Physical sides (`justify-self` only).
    Left,
    Right,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    /// `first baseline`, the same as `baseline`.
    FirstBaseline(FirstBaselineKeyword),
    /// `last baseline`: aligns to the end, as a box that shares its baseline with no other does.
    LastBaseline(LastBaselineKeyword),
}

/// The keywords `first baseline` and `last baseline`, which are two words.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct FirstBaselineKeyword;
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct LastBaselineKeyword;

impl<'i> PropertyValue<'i> for FirstBaselineKeyword {
    type Error = StyleParseError;
    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        input.expect_ident_matching("first")?;
        input.expect_ident_matching("baseline")?;
        Ok(FirstBaselineKeyword)
    }
}

impl<'i> PropertyValue<'i> for LastBaselineKeyword {
    type Error = StyleParseError;
    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        input.expect_ident_matching("last")?;
        input.expect_ident_matching("baseline")?;
        Ok(LastBaselineKeyword)
    }
}

/// The values of `place-content`, `place-items` and `place-self`: the alignment in the block axis
/// and the one in the inline axis, which is the same if there is only one value.
macro_rules! place_shorthand {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone)]
        pub struct $name {
            align: $value,
            justify: $value,
        }

        impl $name {
            pub fn align(&self) -> $value {
                self.align.clone()
            }
            pub fn justify(&self) -> $value {
                self.justify.clone()
            }
        }

        impl<'i> PropertyValue<'i> for $name {
            type Error = StyleParseError;

            fn from_css_property_value(
                input: &mut Parser<'i>,
            ) -> Result<Self, ParseError<Self::Error>> {
                let align = <$value>::from_css_property_value(input)?;
                let justify = input
                    .try_parse(<$value>::from_css_property_value)
                    .unwrap_or_else(|_| align.clone());
                Ok($name { align, justify })
            }
        }
    };
}

place_shorthand!(PlaceContentShorthand, ContentAlignment);
place_shorthand!(PlaceItemsShorthand, ItemAlignment);
place_shorthand!(PlaceSelfShorthand, SelfAlignment);

/// The value of the `flex-flow` shorthand: the direction and the wrapping, in any order. What is
/// left out has its initial value.
#[derive(Debug, Copy, Clone)]
pub struct FlexFlowShorthand {
    direction: FlexDirection,
    wrap: FlexWrap,
}

impl FlexFlowShorthand {
    pub fn direction(&self) -> FlexDirection {
        self.direction
    }
    pub fn wrap(&self) -> FlexWrap {
        self.wrap
    }
}

impl<'i> PropertyValue<'i> for FlexFlowShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut direction = None;
        let mut wrap = None;
        for _ in 0..2 {
            if direction.is_none()
                && let Ok(value) = input.try_parse(FlexDirection::from_css_property_value)
            {
                direction = Some(value);
            } else if wrap.is_none()
                && let Ok(value) = input.try_parse(FlexWrap::from_css_property_value)
            {
                wrap = Some(value);
            } else {
                break;
            }
        }
        if direction.is_none() && wrap.is_none() {
            return Err(input.new_error_for_next_token());
        }
        Ok(FlexFlowShorthand {
            direction: direction.unwrap_or_default(),
            wrap: wrap.unwrap_or_default(),
        })
    }
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

/// The value of `border-radius`: one to four radii in the order top-left, top-right,
/// bottom-right, bottom-left, with missing values taken from the opposite corner like for
/// `margin`. Only circular corners are supported, so the `/` syntax for elliptical corners is
/// a parse error.
/// https://www.w3.org/TR/css-backgrounds-3/#border-radius
#[derive(Debug, Copy, Clone)]
pub struct BorderRadiusShorthand {
    pub top_left: QuantityOrPercentage,
    pub top_right: QuantityOrPercentage,
    pub bottom_right: QuantityOrPercentage,
    pub bottom_left: QuantityOrPercentage,
}

impl BorderRadiusShorthand {
    pub fn top_left(&self) -> QuantityOrPercentage {
        self.top_left
    }
    pub fn top_right(&self) -> QuantityOrPercentage {
        self.top_right
    }
    pub fn bottom_right(&self) -> QuantityOrPercentage {
        self.bottom_right
    }
    pub fn bottom_left(&self) -> QuantityOrPercentage {
        self.bottom_left
    }
}

impl<'i> PropertyValue<'i> for BorderRadiusShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let first = input
            .try_parse(QuantityOrPercentage::from_css_property_value)
            .map_err(|_| input.new_error_for_next_token())?;
        let second = input
            .try_parse(QuantityOrPercentage::from_css_property_value)
            .ok();
        let third = second.and_then(|_| {
            input
                .try_parse(QuantityOrPercentage::from_css_property_value)
                .ok()
        });
        let fourth = third.and_then(|_| {
            input
                .try_parse(QuantityOrPercentage::from_css_property_value)
                .ok()
        });
        let second_value = second.unwrap_or(first);
        Ok(BorderRadiusShorthand {
            top_left: first,
            top_right: second_value,
            bottom_right: third.unwrap_or(first),
            bottom_left: fourth.unwrap_or(second_value),
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

/// The value of `row-gap` and `column-gap`. `normal` is no gap in a grid or flex container and
/// `1em` between columns.
#[derive(Debug, Copy, Clone, PartialEq, PropertyValue)]
pub enum Gap {
    Normal,
    Quantity(Quantity),
    Percentage(Percentage),
}

/// The value of `gap`: the gap between the rows, and the one between the columns if it is not the
/// same.
#[derive(Debug, Copy, Clone)]
pub struct GapShorthand {
    row: Gap,
    column: Gap,
}

impl GapShorthand {
    pub fn row(&self) -> Gap {
        self.row
    }
    pub fn column(&self) -> Gap {
        self.column
    }
}

impl<'i> PropertyValue<'i> for GapShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let row = Gap::from_css_property_value(input)?;
        let column = input.try_parse(Gap::from_css_property_value).unwrap_or(row);
        Ok(GapShorthand { row, column })
    }
}

/// The value of `column-count`: `auto` or a positive integer.
/// https://www.w3.org/TR/css-multicol-1/#cc
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum ColumnCount {
    #[default]
    Auto,
    Count(usize),
}

impl<'i> PropertyValue<'i> for ColumnCount {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(ColumnCount::Auto);
        }
        let token = input.next()?.clone();
        match token {
            Token::Number { int_value: Some(count), .. } if count >= 1 => {
                Ok(ColumnCount::Count(count as usize))
            }
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

/// The value of `column-width`: `auto` or a positive length.
/// https://www.w3.org/TR/css-multicol-1/#cw
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum ColumnWidth {
    #[default]
    Auto,
    Quantity(Quantity),
}

impl<'i> PropertyValue<'i> for ColumnWidth {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(ColumnWidth::Auto);
        }
        let quantity = Quantity::from_css_property_value(input).map_err(|_| input.new_error_for_next_token())?;
        if quantity.value < 0.0 {
            return Err(input.new_error_for_next_token());
        }
        Ok(ColumnWidth::Quantity(quantity))
    }
}

/// The value of `columns`: `<column-width> || <column-count>`. The one that is not given is
/// `auto`.
#[derive(Debug, Copy, Clone)]
pub struct ColumnsShorthand {
    count: ColumnCount,
    width: ColumnWidth,
}

impl ColumnsShorthand {
    pub fn count(&self) -> ColumnCount {
        self.count
    }
    pub fn width(&self) -> ColumnWidth {
        self.width
    }
}

impl<'i> PropertyValue<'i> for ColumnsShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut count = None;
        let mut width = None;
        let mut autos = 0;
        // at most two values, each of them once
        for _ in 0..2 {
            if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
                autos += 1;
            } else if count.is_none() && let Ok(value) = input.try_parse(ColumnCount::from_css_property_value) {
                count = Some(value);
            } else if width.is_none() && let Ok(value) = input.try_parse(ColumnWidth::from_css_property_value) {
                width = Some(value);
            } else {
                break;
            }
        }
        if count.is_none() && width.is_none() && autos == 0 {
            return Err(input.new_error_for_next_token());
        }
        Ok(ColumnsShorthand {
            count: count.unwrap_or_default(),
            width: width.unwrap_or_default(),
        })
    }
}

/// The value of `column-fill`: only the last fragment of a container that is split over pages
/// is balanced with `balance`, every fragment that is not full with `balance-all`, and none with
/// `auto` (see `docs/pagination.md`).
/// https://www.w3.org/TR/css-multicol-1/#cf
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum ColumnFill {
    #[default]
    Balance,
    BalanceAll,
    Auto,
}

/// The value of `column-span`. Not used: a box never spans the columns.
/// https://www.w3.org/TR/css-multicol-1/#column-span
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum ColumnSpan {
    #[default]
    None,
    All,
}

/// The value of `column-rule`: a width, a style and a colour in any order.
#[derive(Debug, Clone)]
pub struct ColumnRuleShorthand(Vec<Border>);

impl ColumnRuleShorthand {
    pub fn width(&self) -> BorderWidth {
        self.0
            .iter()
            .rev()
            .find_map(|border| match border {
                Border::Width(width) => Some(*width),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn style(&self) -> BorderStyle {
        self.0
            .iter()
            .rev()
            .find_map(|border| match border {
                Border::Style(style) => Some(*style),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn color(&self) -> Color {
        self.0
            .iter()
            .rev()
            .find_map(|border| match border {
                Border::Color(color) => Some(*color),
                _ => None,
            })
            .unwrap_or(Color::CurrentColor)
    }
}

impl<'i> PropertyValue<'i> for ColumnRuleShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let borders = Vec::<Border>::from_css_property_value(input)?;
        if borders.is_empty() {
            return Err(input.new_error_for_next_token());
        }
        Ok(ColumnRuleShorthand(borders))
    }
}

/// A track breadth: one end of a track size.
/// https://www.w3.org/TR/css-grid-1/#typedef-track-breadth
#[derive(Debug, Copy, Clone)]
pub enum Breadth {
    Auto,
    MinContent,
    MaxContent,
    /// A flexible length (`<number>fr`)
    Fraction(f32),
    Length(QuantityOrPercentage),
}

impl<'i> PropertyValue<'i> for Breadth {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if let Ok(length) = input.try_parse(QuantityOrPercentage::from_css_property_value) {
            return Ok(Breadth::Length(length));
        }
        let token = input.next()?.clone();
        match &token {
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("fr") => {
                Ok(Breadth::Fraction(*value))
            }
            Token::Ident(ident) if ident.eq_ignore_ascii_case("auto") => Ok(Breadth::Auto),
            Token::Ident(ident) if ident.eq_ignore_ascii_case("min-content") => {
                Ok(Breadth::MinContent)
            }
            Token::Ident(ident) if ident.eq_ignore_ascii_case("max-content") => {
                Ok(Breadth::MaxContent)
            }
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

/// A track size of `grid-template-columns`, `grid-template-rows`, `grid-auto-columns` and
/// `grid-auto-rows`: a breadth, `minmax(<min>, <max>)` or `fit-content(<limit>)`.
/// https://www.w3.org/TR/css-grid-1/#track-sizing
#[derive(Debug, Copy, Clone)]
pub enum TrackSize {
    Breadth(Breadth),
    MinMax(Breadth, Breadth),
    FitContent(QuantityOrPercentage),
}

impl<'i> PropertyValue<'i> for TrackSize {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_function_matching("minmax")).is_ok() {
            return input.parse_nested_block(|input| {
                let min = Breadth::from_css_property_value(input)?;
                input.expect_comma()?;
                let max = Breadth::from_css_property_value(input)?;
                input.expect_exhausted()?;
                // A flexible minimum is not allowed
                if matches!(min, Breadth::Fraction(_)) {
                    return Err(ParseError::custom(StyleParseError::InvalidValue));
                }
                Ok(TrackSize::MinMax(min, max))
            });
        }
        if input.try_parse(|i| i.expect_function_matching("fit-content")).is_ok() {
            return input.parse_nested_block(|input| {
                let limit = QuantityOrPercentage::from_css_property_value(input)?;
                input.expect_exhausted()?;
                Ok(TrackSize::FitContent(limit))
            });
        }
        Breadth::from_css_property_value(input).map(TrackSize::Breadth)
    }
}

/// An entry of a list of tracks: a track size, or `repeat(auto-fill | auto-fit, <tracks>)`.
/// `repeat(<integer>, <tracks>)` is expanded when it is parsed.
#[derive(Debug, Clone)]
pub enum TrackEntry {
    Size(TrackSize),
    AutoRepeat { fit: bool, tracks: Vec<TrackSize> },
}

/// The most tracks one `repeat(<integer>, ...)` may expand to.
const MAX_REPEATED_TRACKS: usize = 1000;

/// A list of track sizes. `none` is the empty list.
#[derive(Debug, Clone, Default)]
pub struct TrackList(pub Vec<TrackEntry>);

impl<'i> PropertyValue<'i> for TrackList {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(TrackList(vec![]));
        }
        input.parse_entirely(|input| {
            let mut entries = vec![];
            let mut auto_repeats = 0;
            while !input.is_exhausted() {
                if input.try_parse(|i| i.expect_function_matching("repeat")).is_ok() {
                    input.parse_nested_block(|input| {
                        let count = input.try_parse(|i| i.expect_integer());
                        let fit = match count {
                            Ok(_) => false,
                            Err(_) => {
                                let ident = input.expect_ident()?.clone();
                                if ident.eq_ignore_ascii_case("auto-fill") {
                                    false
                                } else if ident.eq_ignore_ascii_case("auto-fit") {
                                    true
                                } else {
                                    return Err(ParseError::unexpected_token());
                                }
                            }
                        };
                        input.expect_comma()?;
                        let tracks = Vec::<TrackSize>::from_css_property_value(input)?;
                        if tracks.is_empty() {
                            return Err(ParseError::custom(StyleParseError::InvalidValue));
                        }
                        match count {
                            Ok(count) => {
                                if count < 1 || count as usize * tracks.len() > MAX_REPEATED_TRACKS {
                                    return Err(ParseError::custom(StyleParseError::InvalidValue));
                                }
                                for _ in 0..count {
                                    entries.extend(tracks.iter().copied().map(TrackEntry::Size));
                                }
                            }
                            Err(_) => {
                                auto_repeats += 1;
                                entries.push(TrackEntry::AutoRepeat { fit, tracks });
                            }
                        }
                        Ok(())
                    })?;
                } else {
                    entries.push(TrackEntry::Size(TrackSize::from_css_property_value(input)?));
                }
            }
            // Only one auto repeat is allowed, and an empty list is not a list
            if auto_repeats > 1 || entries.is_empty() {
                return Err(ParseError::custom(StyleParseError::InvalidValue));
            }
            Ok(TrackList(entries))
        })
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

/// The value of the `grid-area` shorthand: `<row-start> [ / <column-start> [ / <row-end> [ /
/// <column-end> ]? ]? ]?`. What is left out is `auto` (there are no line names to copy).
#[derive(Debug, Copy, Clone)]
pub struct GridAreaLines {
    row_start: GridPlacement,
    column_start: GridPlacement,
    row_end: GridPlacement,
    column_end: GridPlacement,
}

impl GridAreaLines {
    pub fn row_start(&self) -> GridPlacement {
        self.row_start
    }
    pub fn column_start(&self) -> GridPlacement {
        self.column_start
    }
    pub fn row_end(&self) -> GridPlacement {
        self.row_end
    }
    pub fn column_end(&self) -> GridPlacement {
        self.column_end
    }
}

impl<'i> PropertyValue<'i> for GridAreaLines {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut lines = [GridPlacement::Auto; 4];
        lines[0] = GridPlacement::from_css_property_value(input)?;
        for line in &mut lines[1..] {
            if input.try_parse(|i| i.expect_delim('/')).is_err() {
                break;
            }
            *line = GridPlacement::from_css_property_value(input)?;
        }
        let [row_start, column_start, row_end, column_end] = lines;
        Ok(GridAreaLines { row_start, column_start, row_end, column_end })
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
    /// The font size of the root element.
    Rem,
    /// The x-height of the font.
    Ex,
    /// The advance of the glyph `0` of the font.
    Ch,
    /// 1% of the width and of the height of the page area, and of the smaller and the larger of
    /// the two.
    Vw,
    Vh,
    /// The page area in the inline and in the block axis.
    Vi,
    Vb,
    Vmin,
    Vmax,
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
            "rem" => Ok(Unit::Rem),
            "ex" => Ok(Unit::Ex),
            "ch" => Ok(Unit::Ch),
            "vw" | "svw" | "lvw" | "dvw" => Ok(Unit::Vw),
            "vh" | "svh" | "lvh" | "dvh" => Ok(Unit::Vh),
            "vi" | "svi" | "lvi" | "dvi" => Ok(Unit::Vi),
            "vb" | "svb" | "lvb" | "dvb" => Ok(Unit::Vb),
            "vmin" | "svmin" | "lvmin" | "dvmin" => Ok(Unit::Vmin),
            "vmax" | "svmax" | "lvmax" | "dvmax" => Ok(Unit::Vmax),
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

impl<T> CommaSeparated<T> {
    pub fn new(elements: Vec<T>) -> Self {
        CommaSeparated { elements }
    }

    pub fn elements(&self) -> &[T] {
        &self.elements
    }
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

#[derive(Debug, Copy, Clone, PartialEq)]
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

#[derive(Debug, Copy, Clone, PartialEq, PropertyValue)]
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

/// The value of `text-transform`. `full-width` and `full-size-kana` are not supported.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PropertyValue)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// The value of `letter-spacing` and `word-spacing`: `normal` or a length.
#[derive(Debug, Copy, Clone, Default, PropertyValue)]
pub enum NormalOrQuantity {
    #[default]
    Normal,
    Quantity(Quantity),
}

/// The value of `max-width` and `max-height`: `none` means there is no limit.
#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum NoneOrQuantityOrPercentage {
    None,
    Quantity(Quantity),
    Percentage(Percentage),
}

impl Default for NoneOrQuantityOrPercentage {
    fn default() -> Self {
        NoneOrQuantityOrPercentage::None
    }
}

/// https://www.w3.org/TR/css-sizing-3/#box-sizing
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum BoxSizing {
    /// `width` and `height` set the size of the content box.
    #[default]
    ContentBox,
    /// `width` and `height` set the size of the border box, i.e. they include padding and border.
    BorderBox,
}

#[derive(Debug, Copy, Clone)]
pub enum BorderWidth {
    Thin,
    Medium,
    Thick,
    Quantity(Quantity),
}

impl<'i> PropertyValue<'i> for BorderWidth {
    type Error = StyleParseError;

    /// `thin | medium | thick | <length [0,∞]>`: a negative length is invalid.
    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let start = input.state();
        if let Ok(ident) = input.expect_ident_cloned() {
            return match_ignore_ascii_case! { &ident,
                "thin" => Ok(BorderWidth::Thin),
                "medium" => Ok(BorderWidth::Medium),
                "thick" => Ok(BorderWidth::Thick),
                _ => {
                    input.reset(&start);
                    Err(input.new_error_for_next_token())
                },
            };
        }
        input.reset(&start);
        let quantity = match Quantity::from_css_property_value(input) {
            Ok(quantity) => quantity,
            Err(_) => {
                input.reset(&start);
                return Err(input.new_error_for_next_token());
            }
        };
        if quantity.value < 0.0 {
            input.reset(&start);
            return Err(input.new_error_for_next_token());
        }
        Ok(BorderWidth::Quantity(quantity))
    }
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
                PixelLength::from_val_unit(value, unit, super::computed::FontSizes::of(style))
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
                PixelLength::from_val_unit(value, unit, super::computed::FontSizes::of(style))
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
    /// White space is collapsed like for `normal`, but lines do not wrap.
    Nowrap,
    Pre,
    /// Spaces and new lines are kept, and lines wrap.
    PreWrap,
    /// New lines are kept, other white space is collapsed, and lines wrap.
    PreLine,
}

impl WhiteSpace {
    /// Whether text may be broken into lines at soft wrap opportunities.
    pub fn wraps(self) -> bool {
        matches!(self, WhiteSpace::Normal | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }

    /// Whether the new lines of the text are line breaks.
    pub fn preserves_newlines(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }

    /// Whether the spaces of the text are kept (and not collapsed).
    pub fn preserves_spaces(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }
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

/// A quoted string, as the marker of `list-style-type: "→ "`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerString(pub std::sync::Arc<str>);

impl<'i> PropertyValue<'i> for MarkerString {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        match input.next()? {
            Token::QuotedString(string) => Ok(MarkerString(string.as_ref().into())),
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PropertyValue)]
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
    /// The marker is this text, without anything added.
    String(MarkerString),
}

/// `inside` or `outside`: whether the marker of a list item is in the first line of the item
/// or outside of it.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

/// The value of the `list-style` shorthand: the marker type and the position, in any order. What
/// is left out has its initial value. `<image>` markers are not supported.
#[derive(Debug, Clone)]
pub struct ListStyleShorthand {
    style_type: ListStyleType,
    position: ListStylePosition,
}

impl ListStyleShorthand {
    pub fn style_type(&self) -> ListStyleType {
        self.style_type.clone()
    }

    pub fn position(&self) -> ListStylePosition {
        self.position
    }
}

impl<'i> PropertyValue<'i> for ListStyleShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut style_type = None;
        let mut position = None;
        for _ in 0..2 {
            if style_type.is_none()
                && let Ok(value) = input.try_parse(ListStyleType::from_css_property_value)
            {
                style_type = Some(value);
            } else if position.is_none()
                && let Ok(value) = input.try_parse(ListStylePosition::from_css_property_value)
            {
                position = Some(value);
            } else {
                break;
            }
        }
        if style_type.is_none() && position.is_none() {
            return Err(input.new_error_for_next_token());
        }
        Ok(ListStyleShorthand {
            style_type: style_type.unwrap_or_default(),
            position: position.unwrap_or_default(),
        })
    }
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Disc
    }
}

impl ListStyleType {
    /// The text of the marker of the `ordinal`th (1-based) list item, without any spacing.
    /// The ordinal can be zero or negative when `<li value>` or a negative `<ol start>` says so.
    pub fn marker_text(&self, ordinal: i64) -> Option<String> {
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
            ListStyleType::String(MarkerString(text)) => text.to_string(),
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

/// The direction in which lines are laid out and in which they stack: the block flow.
/// https://www.w3.org/TR/css-writing-modes-3/#block-flow
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum WritingMode {
    /// Lines run from left to right (for `direction: ltr`) and stack from top to bottom.
    #[default]
    HorizontalTb,
    /// Lines run from top to bottom and stack from right to left.
    VerticalRl,
    /// Lines run from top to bottom and stack from left to right.
    VerticalLr,
    /// Like `vertical-rl`, with all glyphs rotated.
    SidewaysRl,
    /// Lines run from bottom to top and stack from left to right, with all glyphs rotated.
    SidewaysLr,
}

/// The orientation of the glyphs in vertical lines.
/// https://www.w3.org/TR/css-writing-modes-3/#text-orientation
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, PropertyValue)]
pub enum TextOrientation {
    /// Characters of scripts that are written vertically stand upright, the rest is rotated.
    #[default]
    Mixed,
    /// All characters stand upright.
    Upright,
    /// All characters are rotated.
    Sideways,
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

/// https://www.w3.org/TR/css-backgrounds-3/#border-style
#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum BorderStyle {
    None,
    /// Like `none`, but wins over the other borders in the conflicts of collapsed borders.
    Hidden,
    Dotted,
    Dashed,
    Solid,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderStyle {
    /// Whether the border takes space and is painted: with the style `none` or `hidden` its
    /// width is zero.
    pub fn is_visible(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }

    /// The rank of the style in the conflicts of collapsed borders of the same width and of the
    /// same kind of box: the greater one wins.
    /// https://www.w3.org/TR/CSS22/tables.html#border-conflict-resolution
    pub fn collapse_rank(self) -> u8 {
        match self {
            BorderStyle::None | BorderStyle::Hidden => 0,
            BorderStyle::Inset => 1,
            BorderStyle::Groove => 2,
            BorderStyle::Outset => 3,
            BorderStyle::Ridge => 4,
            BorderStyle::Dotted => 5,
            BorderStyle::Dashed => 6,
            BorderStyle::Solid => 7,
            BorderStyle::Double => 8,
        }
    }
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::None
    }
}

/// A colour, or a `color-mix()` of colours (which is computed here: see
/// [`super::color_mix`]).
fn parse_css_color<'i>(input: &mut Parser<'i>) -> Result<Color, ()> {
    let state = input.state();
    match Color::parse(input) {
        Ok(color) => Ok(color),
        Err(_) => {
            input.reset(&state);
            super::color_mix::parse(input, &mut parse_css_color)
        }
    }
}

impl<'i> PropertyValue<'i> for Color {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let state = input.state();
        match Color::parse(input) {
            Ok(color) => Ok(color),
            Err(error) => {
                input.reset(&state);
                if let Ok(mixed) = super::color_mix::parse(input, &mut parse_css_color) {
                    return Ok(mixed);
                }
                input.reset(&state);
                Err(match error.kind {
                    cssparser::ParseErrorKind::Basic(kind) => ParseError::from_basic_kind(kind),
                    cssparser::ParseErrorKind::Custom(()) => ParseError::unexpected_token(),
                })
            }
        }
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

/// One shadow of `box-shadow`.
#[derive(Debug, Clone)]
pub struct Shadow {
    pub x: Quantity,
    pub y: Quantity,
    pub blur: Quantity,
    pub spread: Quantity,
    /// `None` is `currentcolor`.
    pub color: Option<Color>,
    pub inset: bool,
}

/// The value of `box-shadow`: `none` or a list of shadows, the first of which is on top.
#[derive(Debug, Clone, Default)]
pub struct ShadowList(pub Vec<Shadow>);

impl<'i> PropertyValue<'i> for ShadowList {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|input| input.expect_ident_matching("none")).is_ok() {
            return Ok(ShadowList(vec![]));
        }
        let shadows = input.parse_comma_separated(|input| {
            let (mut color, mut inset) = (None, false);
            let mut lengths: Vec<Quantity> = vec![];
            // the color and `inset` may come before, after or between the lengths
            loop {
                if !inset && input.try_parse(|input| input.expect_ident_matching("inset")).is_ok() {
                    inset = true;
                    continue;
                }
                if color.is_none() {
                    if let Ok(parsed) = input.try_parse(|input| Color::from_css_property_value(input).map_err(|_| ())) {
                        color = Some(parsed);
                        continue;
                    }
                }
                if lengths.len() < 4 {
                    if let Ok(length) = input.try_parse(Quantity::from_css_property_value) {
                        lengths.push(length);
                        continue;
                    }
                }
                break;
            }
            let zero = Quantity { value: 0.0, unit: Unit::Px };
            if lengths.len() < 2 {
                return Err(ParseError::custom(StyleParseError::InvalidValue));
            }
            // a negative blur radius is an error
            if lengths.get(2).is_some_and(|blur| blur.value < 0.0) {
                return Err(ParseError::custom(StyleParseError::InvalidValue));
            }
            Ok(Shadow {
                x: lengths[0],
                y: lengths[1],
                blur: lengths.get(2).copied().unwrap_or(zero),
                spread: lengths.get(3).copied().unwrap_or(zero),
                color,
                inset,
            })
        })?;
        Ok(ShadowList(shadows))
    }
}

impl ComputedValue<ShadowList> for Vec<super::computed::BoxShadow> {
    fn from_specified(list: &ShadowList, style: &super::computed::ComputedValues) -> Self {
        let px = |quantity: Quantity| {
            PixelLength::from_val_unit(quantity.value, quantity.unit, super::computed::FontSizes::of(style)).0
        };
        list.0
            .iter()
            .map(|shadow| super::computed::BoxShadow {
                x: px(shadow.x),
                y: px(shadow.y),
                blur: px(shadow.blur).max(0.0),
                spread: px(shadow.spread),
                color: shadow.color.unwrap_or(Color::CurrentColor),
                inset: shadow.inset,
            })
            .collect()
    }
}

/// The value of `opacity`: a number or a percentage, clamped to the range from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opacity(pub f32);

impl From<Opacity> for f32 {
    fn from(opacity: Opacity) -> f32 {
        opacity.0
    }
}

impl<'i> PropertyValue<'i> for Opacity {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let value = match input.next()? {
            Token::Number { value, .. } => *value,
            Token::Percentage { unit_value, .. } => *unit_value,
            _ => return Err(ParseError::unexpected_token()),
        };
        // `NaN` cannot be written in CSS, but a huge number overflows to infinity
        Ok(Opacity(if value.is_nan() { 1.0 } else { value.clamp(0.0, 1.0) }))
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

/// `aspect-ratio: auto | <ratio> | auto && <ratio>`: the preferred ratio of the width to the height
/// of the box. With `auto`, a replaced element has the ratio of its content instead (and the ratio
/// is the one of the boxes that have no content ratio); a ratio of `0` or `/ 0` is no ratio.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AspectRatio {
    pub auto: bool,
    pub ratio: Option<f32>,
}

impl Default for AspectRatio {
    fn default() -> Self {
        AspectRatio { auto: true, ratio: None }
    }
}

impl<'i> PropertyValue<'i> for AspectRatio {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut auto = false;
        let mut ratio: Option<Option<f32>> = None;
        // `auto` and the ratio, in any order
        for _ in 0..2 {
            if !auto && input.try_parse(|input| input.expect_ident_matching("auto")).is_ok() {
                auto = true;
            } else if ratio.is_none()
                && let Ok(value) = input.try_parse(parse_ratio)
            {
                ratio = Some(value);
            } else {
                break;
            }
        }
        if !auto && ratio.is_none() {
            return Err(input.new_error_for_next_token());
        }
        // a degenerate ratio is no ratio
        let ratio = ratio.flatten();
        Ok(AspectRatio { auto: auto || ratio.is_none(), ratio })
    }
}

/// `<number [0,∞]> [ / <number [0,∞]> ]?`: the width divided by the height, or `None` for a
/// degenerate ratio (zero or infinite).
fn parse_ratio<'i>(input: &mut Parser<'i>) -> Result<Option<f32>, ParseError<StyleParseError>> {
    let width = f32::from_css_property_value(input)?;
    let height = if input.try_parse(|input| input.expect_delim('/')).is_ok() {
        f32::from_css_property_value(input)?
    } else {
        1.0
    };
    if width < 0.0 || height < 0.0 {
        return Err(input.new_error_for_next_token());
    }
    Ok((width > 0.0 && height > 0.0 && (width / height).is_finite()).then(|| width / height))
}

/// `vertical-align`: how an inline-level box is aligned in its line, or the content of a table
/// cell in the cell.
#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum VerticalAlign {
    /// Raises the box by the length, or by the percentage of the `line-height` of the box.
    QuantityOrPercentage(QuantityOrPercentage),
    Baseline,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Middle,
    Top,
    Bottom,
}

#[derive(Debug, Copy, Clone, PropertyValue)]
pub enum LineHeight {
    QuantityOrPercentage(QuantityOrPercentage),
    Number(f32),
    Normal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FontFamily {
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

impl<'i> PropertyValue<'i> for FontFamily {
    type Error = StyleParseError;

    /// A generic family keyword, a string, or a name of one or more identifiers that are joined
    /// by single spaces (https://www.w3.org/TR/css-fonts-4/#family-name-syntax).
    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if let Ok(name) = input.try_parse(|input| input.expect_string().map(|s| s.to_string())) {
            return Ok(FontFamily::Named(name));
        }
        let first = input.expect_ident()?.clone();
        let generic = match_ignore_ascii_case! { &first,
            "sans-serif" => Some(FontFamily::SansSerif),
            "serif" => Some(FontFamily::Serif),
            "monospace" => Some(FontFamily::Monospace),
            _ => None,
        };
        if let Some(generic) = generic {
            return Ok(generic);
        }
        let mut name = first.to_string();
        while let Ok(next) = input.try_parse(|input| input.expect_ident().cloned()) {
            name.push(' ');
            name.push_str(&next);
        }
        Ok(FontFamily::Named(name))
    }
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

/// The value of the `font` shorthand:
/// `[ <font-style> || <font-variant-css2> || <font-weight> || <font-stretch-css3> ]? <font-size>
/// [ / <line-height> ]? <font-family>#`. What is left out is reset to its initial value.
#[derive(Debug, Clone)]
pub struct FontShorthand {
    style: FontStyle,
    weight: FontWeight,
    size: FontSize,
    line_height: LineHeight,
    family: Vec<FontFamily>,
    caps: FontVariantCaps,
}

impl FontShorthand {
    pub fn caps(&self) -> FontVariantCaps {
        self.caps
    }

    pub fn ligatures(&self) -> FontVariantLigatures {
        FontVariantLigatures::default()
    }

    pub fn numeric(&self) -> FontVariantNumeric {
        FontVariantNumeric::default()
    }

    pub fn east_asian(&self) -> FontVariantEastAsian {
        FontVariantEastAsian::default()
    }

    pub fn position(&self) -> FontVariantPosition {
        FontVariantPosition::default()
    }

    pub fn feature_settings(&self) -> FontFeatureSettings {
        FontFeatureSettings::default()
    }

    pub fn kerning(&self) -> FontKerning {
        FontKerning::default()
    }

    pub fn style(&self) -> FontStyle {
        self.style.clone()
    }

    pub fn weight(&self) -> FontWeight {
        self.weight
    }

    pub fn size(&self) -> FontSize {
        self.size
    }

    pub fn line_height(&self) -> LineHeight {
        self.line_height
    }

    pub fn family(&self) -> CommaSeparated<FontFamily> {
        CommaSeparated::new(self.family.clone())
    }
}

impl<'i> PropertyValue<'i> for FontShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        let mut style = None;
        let mut weight = None;
        let mut caps = FontVariantCaps::Normal;
        // `normal` stands for the initial value of any of the four, it is only counted
        let mut normals = 0;
        let mut others = 0;
        while normals + others < 4 {
            if input.try_parse(|input| input.expect_ident_matching("normal")).is_ok() {
                normals += 1;
            } else if style.is_none() && let Ok(parsed) = input.try_parse(FontStyle::from_css_property_value) {
                style = Some(parsed);
                others += 1;
            } else if weight.is_none() && let Ok(parsed) = input.try_parse(FontWeight::from_css_property_value) {
                weight = Some(parsed);
                others += 1;
            } else if input
                .try_parse(|input| {
                    // `font-variant` of CSS 2 and the keywords of `font-stretch`
                    let ident = input.expect_ident()?.clone();
                    match_ignore_ascii_case! { &ident,
                        "small-caps" => Ok(true),
                        "ultra-condensed" | "extra-condensed" | "condensed"
                        | "semi-condensed" | "semi-expanded" | "expanded" | "extra-expanded"
                        | "ultra-expanded" => Ok(false),
                        _ => Err(input.new_error_for_next_token::<()>()),
                    }
                })
                .inspect(|small_caps| {
                    if *small_caps {
                        caps = FontVariantCaps::SmallCaps;
                    }
                })
                .is_ok()
            {
                others += 1;
            } else {
                break;
            }
        }
        let size = FontSize::from_css_property_value(input).map_err(|_| input.new_error_for_next_token())?;
        let line_height = if input.try_parse(|input| input.expect_delim('/')).is_ok() {
            LineHeight::from_css_property_value(input).map_err(|_| input.new_error_for_next_token())?
        } else {
            LineHeight::Normal
        };
        let family = <CommaSeparated<FontFamily>>::from_css_property_value(input)
            .map_err(|_| input.new_error_for_next_token())?
            .elements()
            .to_vec();
        Ok(FontShorthand {
            style: style.unwrap_or_default(),
            weight: weight.unwrap_or_default(),
            size,
            line_height,
            family,
            caps,
        })
    }
}

/// https://www.w3.org/TR/css-fonts-4/#font-weight-prop: a keyword or a number from 1 to 1000.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum FontWeight {
    Normal,
    Bold,
    /// One step heavier than the weight of the parent.
    Bolder,
    /// One step lighter than the weight of the parent.
    Lighter,
    Number(f32),
}

impl Default for FontWeight {
    fn default() -> FontWeight {
        FontWeight::Normal
    }
}

impl<'i> PropertyValue<'i> for FontWeight {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        let token = input.next()?.clone();
        match token {
            Token::Ident(ident) => match_ignore_ascii_case! { &ident,
                "normal" => Ok(FontWeight::Normal),
                "bold" => Ok(FontWeight::Bold),
                "bolder" => Ok(FontWeight::Bolder),
                "lighter" => Ok(FontWeight::Lighter),
                _ => Err(ParseError::unexpected_token()),
            },
            Token::Number { value, .. } if (1.0..=1000.0).contains(&value) => {
                Ok(FontWeight::Number(value))
            }
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

impl ComputedValue<FontWeight> for super::computed::FontWeight {
    fn from_specified(weight: &FontWeight, style: &super::computed::ComputedValues) -> Self {
        use super::computed::FontWeight as Computed;
        let parent = style.font.font_weight.0;
        match *weight {
            FontWeight::Normal => Computed::NORMAL,
            FontWeight::Bold => Computed::BOLD,
            FontWeight::Number(value) => Computed(value),
            // https://www.w3.org/TR/css-fonts-4/#relative-weights
            FontWeight::Bolder => Computed(match parent {
                w if w < 350.0 => 400.0,
                w if w < 550.0 => 700.0,
                _ => 900.0,
            }),
            FontWeight::Lighter => Computed(match parent {
                w if w < 100.0 => w,
                w if w < 550.0 => 100.0,
                w if w < 750.0 => 400.0,
                _ => 700.0,
            }),
        }
    }
}

/// The sizes with a name for `@page { size }`, in portrait orientation.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NamedPageSize {
    A5,
    A4,
    A3,
    B5,
    B4,
    JisB5,
    JisB4,
    Letter,
    Legal,
    Ledger,
}

impl NamedPageSize {
    /// The width and height in millimetres, or inches for the American sizes.
    pub fn dimensions(self) -> (f32, f32, Unit) {
        match self {
            NamedPageSize::A5 => (148.0, 210.0, Unit::Mm),
            NamedPageSize::A4 => (210.0, 297.0, Unit::Mm),
            NamedPageSize::A3 => (297.0, 420.0, Unit::Mm),
            NamedPageSize::B5 => (176.0, 250.0, Unit::Mm),
            NamedPageSize::B4 => (250.0, 353.0, Unit::Mm),
            NamedPageSize::JisB5 => (182.0, 257.0, Unit::Mm),
            NamedPageSize::JisB4 => (257.0, 364.0, Unit::Mm),
            NamedPageSize::Letter => (8.5, 11.0, Unit::In),
            NamedPageSize::Legal => (8.5, 14.0, Unit::In),
            NamedPageSize::Ledger => (11.0, 17.0, Unit::In),
        }
    }
}

/// The value of `page`: the name of the kind of page the element is printed on, or `auto`
/// (`None`), which is the name of the parent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageName(pub Option<std::sync::Arc<str>>);

impl<'i> PropertyValue<'i> for PageName {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        input.parse_entirely(|input| {
            let ident = input.expect_ident()?.clone();
            if ident.eq_ignore_ascii_case("auto") {
                Ok(PageName(None))
            } else if ident.eq_ignore_ascii_case("inherit")
                || ident.eq_ignore_ascii_case("initial")
                || ident.eq_ignore_ascii_case("unset")
            {
                Err(input.new_error_for_next_token())
            } else {
                Ok(PageName(Some(std::sync::Arc::from(&*ident))))
            }
        })
    }
}

/// The value of `size` in `@page`: `auto`, one or two lengths, or a page size name and/or an
/// orientation.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum PageSize {
    Auto,
    /// One length is both width and height.
    Lengths(Quantity, Option<Quantity>),
    /// A named size or only an orientation. A name without orientation is portrait; without a
    /// name, the orientation applies to the default page size.
    Keywords {
        name: Option<NamedPageSize>,
        landscape: Option<bool>,
    },
}

impl Default for PageSize {
    fn default() -> Self {
        PageSize::Auto
    }
}

impl<'i> PropertyValue<'i> for PageSize {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        input.parse_entirely(|input| {
            if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
                return Ok(PageSize::Auto);
            }
            if let Ok(width) = input.try_parse(Quantity::from_css_property_value) {
                let height = input.try_parse(Quantity::from_css_property_value).ok();
                return Ok(PageSize::Lengths(width, height));
            }
            let (mut name, mut landscape) = (None, None);
            while !input.is_exhausted() {
                let ident = input.expect_ident()?.clone();
                let named = match_ignore_ascii_case! { &ident,
                    "a5" => Some(NamedPageSize::A5),
                    "a4" => Some(NamedPageSize::A4),
                    "a3" => Some(NamedPageSize::A3),
                    "b5" => Some(NamedPageSize::B5),
                    "b4" => Some(NamedPageSize::B4),
                    "jis-b5" => Some(NamedPageSize::JisB5),
                    "jis-b4" => Some(NamedPageSize::JisB4),
                    "letter" => Some(NamedPageSize::Letter),
                    "legal" => Some(NamedPageSize::Legal),
                    "ledger" => Some(NamedPageSize::Ledger),
                    _ => None,
                };
                let orientation = match_ignore_ascii_case! { &ident,
                    "landscape" => Some(true),
                    "portrait" => Some(false),
                    _ => None,
                };
                match (named, orientation) {
                    (Some(size), _) if name.is_none() => name = Some(size),
                    (_, Some(value)) if landscape.is_none() => landscape = Some(value),
                    _ => return Err(input.new_error_for_next_token()),
                }
            }
            if name.is_none() && landscape.is_none() {
                return Err(input.new_error_for_next_token());
            }
            Ok(PageSize::Keywords { name, landscape })
        })
    }
}

/// The values of `break-before` and `break-after` that apply to paged media.
/// https://www.w3.org/TR/css-break-3/#break-between
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default, PropertyValue)]
pub enum Break {
    #[default]
    Auto,
    /// Avoid a break of any kind.
    Avoid,
    AvoidPage,
    AvoidColumn,
    /// A forced page break (the CSS 2 name; the same as `page`).
    Always,
    Page,
    /// A forced break between columns.
    Column,
    /// Forced breaks to a left or right page. An empty page is left where the next page would be
    /// of the other side.
    Left,
    Right,
    Recto,
    Verso,
}

impl Break {
    /// Whether this value forces a break between pages.
    pub fn forces_page(self) -> bool {
        matches!(
            self,
            Break::Always | Break::Page | Break::Left | Break::Right | Break::Recto | Break::Verso
        )
    }

    /// Whether this value forces a break between columns.
    pub fn forces_column(self) -> bool {
        self == Break::Column
    }

    /// Whether this value asks to avoid a break between pages.
    pub fn avoids_page(self) -> bool {
        matches!(self, Break::Avoid | Break::AvoidPage)
    }

    /// Whether this value asks to avoid a break between columns.
    pub fn avoids_column(self) -> bool {
        matches!(self, Break::Avoid | Break::AvoidColumn)
    }
}

/// `bookmark-level`: `none` or the level (1 or more) of the entry that the element has in the
/// outline (the bookmarks) of the PDF. Not inherited.
/// https://www.w3.org/TR/css-gcpm-3/#bookmark-level
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum BookmarkLevel {
    #[default]
    None,
    Level(u8),
}

impl<'i> PropertyValue<'i> for BookmarkLevel {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(BookmarkLevel::None);
        }
        match input.next()?.clone() {
            // levels deeper than 255 are not told apart
            Token::Number { int_value: Some(level), .. } if level >= 1 => {
                Ok(BookmarkLevel::Level(level.min(255) as u8))
            }
            _ => Err(ParseError::unexpected_token()),
        }
    }
}

/// `bookmark-state`: whether the entries below the entry of the element are shown when the PDF is
/// opened. Not inherited.
/// https://www.w3.org/TR/css-gcpm-3/#bookmark-state
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default, PropertyValue)]
pub enum BookmarkState {
    #[default]
    Open,
    Closed,
}

/// A part of the value of `bookmark-label`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarkLabelPart {
    String(String),
    /// `content()` and `content(text)`: the text of the element.
    Content,
    /// `attr(name)`: the value of the attribute of the element.
    Attr(String),
}

/// `bookmark-label`: the text of the entry of the element in the outline. The initial value is
/// `content(text)`. Of the functions of CSS GCPM only `content()`, `content(text)` and
/// `attr(<name>)` are supported (not `content(before)`, `string()` and `counter()`). Not
/// inherited.
/// https://www.w3.org/TR/css-gcpm-3/#bookmark-label
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BookmarkLabel(pub Vec<BookmarkLabelPart>);

impl BookmarkLabel {
    /// The label of an element, with `text` its text and `attribute` the lookup of its attributes.
    pub fn resolve(&self, text: &str, attribute: impl Fn(&str) -> Option<String>) -> String {
        if self.0.is_empty() {
            return text.to_owned();
        }
        let mut label = String::new();
        for part in &self.0 {
            match part {
                BookmarkLabelPart::String(string) => label.push_str(string),
                BookmarkLabelPart::Content => label.push_str(text),
                BookmarkLabelPart::Attr(name) => label.push_str(&attribute(name).unwrap_or_default()),
            }
        }
        label
    }
}

impl<'i> PropertyValue<'i> for BookmarkLabel {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<Self::Error>> {
        let mut parts = Vec::new();
        // `none` is the empty label
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(BookmarkLabel(vec![BookmarkLabelPart::String(String::new())]));
        }
        while let Ok(token) = input.next().cloned() {
            match token {
                Token::QuotedString(string) => parts.push(BookmarkLabelPart::String(string.as_ref().into())),
                Token::Function(name) if name.eq_ignore_ascii_case("content") => {
                    input.parse_nested_block(|input| {
                        if !input.is_exhausted() {
                            input.expect_ident_matching("text")?;
                        }
                        Ok(())
                    })?;
                    parts.push(BookmarkLabelPart::Content);
                }
                Token::Function(name) if name.eq_ignore_ascii_case("attr") => {
                    let attribute = input.parse_nested_block(|input| {
                        let name = input.expect_ident()?.to_string();
                        input.expect_exhausted()?;
                        Ok(name)
                    })?;
                    parts.push(BookmarkLabelPart::Attr(attribute));
                }
                _ => return Err(ParseError::unexpected_token()),
            }
        }
        if parts.is_empty() {
            return Err(ParseError::unexpected_token());
        }
        Ok(BookmarkLabel(parts))
    }
}

/// https://www.w3.org/TR/css-break-3/#break-within
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default, PropertyValue)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
    AvoidPage,
    AvoidColumn,
}

impl BreakInside {
    /// Whether breaks between pages inside the box are to be avoided.
    pub fn avoids_page(self) -> bool {
        matches!(self, BreakInside::Avoid | BreakInside::AvoidPage)
    }

    /// Whether breaks between columns inside the box are to be avoided.
    pub fn avoids_column(self) -> bool {
        matches!(self, BreakInside::Avoid | BreakInside::AvoidColumn)
    }
}

/// `float`: where a box is on the page that the place in the text where it is called for is on.
/// `top` and `bottom` are the placements of TeX's `[t]` and `[b]`: the box is taken out of the
/// text, and it is placed at the top or at the bottom of the page if it fits there, and else on a
/// later page. The other values of `float` are not supported.
/// https://www.w3.org/TR/css-gcpm-3/#float-positions
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default, PropertyValue)]
pub enum Float {
    #[default]
    None,
    Top,
    Bottom,
}

/// `float-reference`: `column` places a float in the column that it is called for in (outside of
/// a container with columns, the page is the column), and `page` places it on the page, as wide as
/// the page, taking room from the container that it is in. The other values of GCPM (`inline` and
/// `region`) are not supported.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default, PropertyValue)]
pub enum FloatReference {
    #[default]
    Column,
    Page,
}

/// The values of `page-break-before` and `page-break-after`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum LegacyBreak {
    Auto,
    Always,
    Avoid,
    Left,
    Right,
}

impl LegacyBreak {
    pub fn to_break(&self) -> Break {
        match self {
            LegacyBreak::Auto => Break::Auto,
            LegacyBreak::Always => Break::Page,
            LegacyBreak::Avoid => Break::Avoid,
            LegacyBreak::Left => Break::Left,
            LegacyBreak::Right => Break::Right,
        }
    }
}

/// The values of `page-break-inside`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PropertyValue)]
pub enum LegacyBreakInside {
    Auto,
    Avoid,
}

impl LegacyBreakInside {
    pub fn to_break_inside(&self) -> BreakInside {
        match self {
            LegacyBreakInside::Auto => BreakInside::Auto,
            LegacyBreakInside::Avoid => BreakInside::Avoid,
        }
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
    Relative,
    Absolute,
    /// Computes to `absolute`: the box is on the first page only; it is not repeated on the
    /// others.
    Fixed,
    /// Computes to `relative`: on a page there is nothing to stick to.
    Sticky,
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

impl From<MathOperatorForm> for Option<formel::host::Form> {
    fn from(other: MathOperatorForm) -> Self {
        match other {
            MathOperatorForm::Auto => None,
            MathOperatorForm::Prefix => Some(formel::host::Form::Prefix),
            MathOperatorForm::Infix => Some(formel::host::Form::Infix),
            MathOperatorForm::Postfix => Some(formel::host::Form::Postfix),
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
    fn box_shadow() {
        let list: ShadowList = parse("0 2px 8px rgb(0 0 0 / .2), inset 1px 1px red").unwrap();
        assert_eq!(list.0.len(), 2);
        let first = &list.0[0];
        assert_eq!((first.x.value, first.y.value, first.blur.value, first.spread.value), (0.0, 2.0, 8.0, 0.0));
        assert!(first.color.is_some() && !first.inset);
        assert!(list.0[1].inset);
        // the color may come first or last; no color is `currentcolor`
        assert!(parse::<ShadowList>("red 1px 2px 3px 4px").unwrap().0[0].color.is_some());
        let plain = parse::<ShadowList>("1px 2px").unwrap();
        assert!(plain.0[0].color.is_none());
        assert_eq!(parse::<ShadowList>("none").unwrap().0.len(), 0);
        // two lengths at least, and no negative blur
        assert!(parse::<ShadowList>("1px").is_none());
        assert!(parse::<ShadowList>("1px 1px -2px").is_none());
        assert!(parse::<ShadowList>("1px 1px 1px 1px 1px").is_none());
    }

    #[test]
    fn grid_track_lists() {
        let list: TrackList = parse("repeat(3, minmax(10px, 1fr)) auto fit-content(5px)").unwrap();
        assert_eq!(list.0.len(), 5);
        assert!(matches!(list.0[0], TrackEntry::Size(TrackSize::MinMax(Breadth::Length(_), Breadth::Fraction(_)))));
        assert!(matches!(list.0[3], TrackEntry::Size(TrackSize::Breadth(Breadth::Auto))));
        assert!(matches!(list.0[4], TrackEntry::Size(TrackSize::FitContent(_))));
        let auto: TrackList = parse("100px repeat(auto-fit, 50px 1fr)").unwrap();
        assert!(matches!(&auto.0[1], TrackEntry::AutoRepeat { fit: true, tracks } if tracks.len() == 2));
        assert_eq!(parse::<TrackList>("none").unwrap().0.len(), 0);
        // no repeat of nothing, no flexible minimum, one automatic repeat at most, and no
        // unreasonably large grid
        assert!(parse::<TrackList>("repeat(0, 1fr)").is_none());
        assert!(parse::<TrackList>("repeat(2)").is_none());
        assert!(parse::<TrackList>("minmax(1fr, 2fr)").is_none());
        assert!(parse::<TrackList>("repeat(auto-fill, 1fr) repeat(auto-fill, 1fr)").is_none());
        assert!(parse::<TrackList>("repeat(100000, 1fr)").is_none());
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
    fn page_names() {
        assert_eq!(parse("auto"), Some(PageName(None)));
        assert_eq!(parse("chapter"), Some(PageName(Some("chapter".into()))));
        // names are case-sensitive
        assert_ne!(parse::<PageName>("Chapter"), parse::<PageName>("chapter"));
        assert_eq!(parse::<PageName>("10"), None);
        assert_eq!(parse::<PageName>("a b"), None);
        assert_eq!(parse::<PageName>("inherit"), None);
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
