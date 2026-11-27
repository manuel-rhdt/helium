//! The logical properties: `margin-inline-start`, `padding-block`, `inset-inline-end`,
//! `inline-size` and the like.
//!
//! They name the sides of a box by the flow of the text (CSS Logical Properties 1): the block
//! axis is the one the lines stack in and the inline axis the one they run in. Which physical
//! side that is depends on the `writing-mode` and `direction` of the element itself, wherever they
//! are declared, so the declaration is turned into the declarations of the physical properties
//! when it is applied (the cascade applies `writing-mode` and `direction` first).
//!
//! Not supported: the logical `border-*` properties and `border-radius` corners, and the CSS-wide
//! keywords (`inherit`, ...) for the logical properties.
//! <https://www.w3.org/TR/css-logical-1/>

use cssparser::{CowRcStr, ParseError, Parser};

use super::computed::ComputedValues;
use super::specified::{
    ApplyToComputedValues, AutoOrQuantityOrPercentage, CssWide, CssWideProperties, Direction,
    NonNegative, NoneOrQuantityOrPercentage, ParseDeclaration, Position, PropertyValue, QuantityOrPercentage,
    WritingMode,
};
use crate::style::parse::StyleParseError;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Axis {
    Block,
    Inline,
}

/// The values of the four logical sides that a declaration sets.
#[derive(Debug, Copy, Clone)]
struct Edges<T> {
    block_start: Option<T>,
    block_end: Option<T>,
    inline_start: Option<T>,
    inline_end: Option<T>,
}

impl<T> Edges<T> {
    fn map(self, f: impl Fn(T) -> T) -> Edges<T> {
        Edges {
            block_start: self.block_start.map(&f),
            block_end: self.block_end.map(&f),
            inline_start: self.inline_start.map(&f),
            inline_end: self.inline_end.map(&f),
        }
    }
}

impl<T> Default for Edges<T> {
    fn default() -> Self {
        Edges { block_start: None, block_end: None, inline_start: None, inline_end: None }
    }
}

/// A logical declaration.
#[derive(Debug, Clone)]
pub enum Logical {
    Margin(Edges<AutoOrQuantityOrPercentage>),
    Padding(Edges<QuantityOrPercentage>),
    Inset(Edges<AutoOrQuantityOrPercentage>),
    /// `inline-size` or `block-size`
    Size(Axis, super::specified::SizeValue),
    /// `min-inline-size` or `min-block-size`
    MinSize(Axis, AutoOrQuantityOrPercentage),
    /// `max-inline-size` or `max-block-size`
    MaxSize(Axis, NoneOrQuantityOrPercentage),
}

/// The physical side that a logical one is on, for a box with this writing mode and direction.
fn physical(writing_mode: WritingMode, direction: Direction, axis: Axis, end: bool) -> Side {
    let vertical = writing_mode != WritingMode::HorizontalTb;
    let rtl = direction == Direction::Rtl;
    match axis {
        Axis::Block => match (writing_mode, end) {
            (WritingMode::HorizontalTb, false) => Side::Top,
            (WritingMode::HorizontalTb, true) => Side::Bottom,
            (WritingMode::VerticalRl | WritingMode::SidewaysRl, false) => Side::Right,
            (WritingMode::VerticalRl | WritingMode::SidewaysRl, true) => Side::Left,
            (WritingMode::VerticalLr | WritingMode::SidewaysLr, false) => Side::Left,
            (WritingMode::VerticalLr | WritingMode::SidewaysLr, true) => Side::Right,
        },
        Axis::Inline => {
            // the end side is the start side of the opposite direction
            let end = end != rtl;
            match (vertical, end) {
                (false, false) => Side::Left,
                (false, true) => Side::Right,
                (true, false) => Side::Top,
                (true, true) => Side::Bottom,
            }
        }
    }
}

/// The width or the height, for a box with this writing mode, that an axis is the size in.
fn is_width(writing_mode: WritingMode, axis: Axis) -> bool {
    (writing_mode == WritingMode::HorizontalTb) == (axis == Axis::Inline)
}

fn two<'i, T: Copy + PropertyValue<'i>>(
    input: &mut Parser<'i>,
) -> Result<(T, T), ParseError<T::Error>> {
    let start = T::from_css_property_value(input)?;
    let end = input.try_parse(T::from_css_property_value).unwrap_or(start);
    Ok((start, end))
}

impl<'i> ParseDeclaration<'i> for Logical {
    type Error = StyleParseError;

    fn parse_declaration(
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self, ParseError<Self::Error>> {
        let lower = name.to_ascii_lowercase();
        let unknown = || ParseError::custom(StyleParseError::UnknownProperty { name: name.to_string() });
        // `margin-inline-start`: the property, the axis, and the side or the pair of sides
        let (property, rest) = lower.split_once('-').ok_or_else(unknown)?;
        match (property, rest) {
            ("inline" | "block", "size") => {
                let axis = if property == "inline" { Axis::Inline } else { Axis::Block };
                return Ok(Logical::Size(axis, super::specified::SizeValue::from_css_property_value(input)?.non_negative()));
            }
            ("min" | "max", "inline-size" | "block-size") => {
                let axis = if rest.starts_with("inline") { Axis::Inline } else { Axis::Block };
                return Ok(if property == "min" {
                    Logical::MinSize(axis, AutoOrQuantityOrPercentage::from_css_property_value(input)?.non_negative())
                } else {
                    Logical::MaxSize(axis, NoneOrQuantityOrPercentage::from_css_property_value(input)?.non_negative())
                });
            }
            _ => {}
        }
        let axis = match rest {
            "block" | "block-start" | "block-end" => Axis::Block,
            "inline" | "inline-start" | "inline-end" => Axis::Inline,
            _ => return Err(unknown()),
        };
        // the sides that are set: both for the shorthand
        let (start, end) = match rest {
            "block" | "inline" => (true, true),
            r if r.ends_with("-start") => (true, false),
            _ => (false, true),
        };
        macro_rules! edges {
            ($ty:ty) => {{
                let (a, b): ($ty, $ty) = if start && end {
                    two(input)?
                } else {
                    let value = <$ty>::from_css_property_value(input)?;
                    (value, value)
                };
                let mut edges = Edges::default();
                match (axis, start, end) {
                    (Axis::Block, true, true) => (edges.block_start, edges.block_end) = (Some(a), Some(b)),
                    (Axis::Inline, true, true) => (edges.inline_start, edges.inline_end) = (Some(a), Some(b)),
                    (Axis::Block, true, false) => edges.block_start = Some(a),
                    (Axis::Block, false, true) => edges.block_end = Some(b),
                    (Axis::Inline, true, false) => edges.inline_start = Some(a),
                    (Axis::Inline, false, true) => edges.inline_end = Some(b),
                    _ => unreachable!(),
                }
                edges
            }};
        }
        match property {
            "margin" => Ok(Logical::Margin(edges!(AutoOrQuantityOrPercentage))),
            "padding" => Ok(Logical::Padding(edges!(QuantityOrPercentage).map(NonNegative::non_negative))),
            "inset" => Ok(Logical::Inset(edges!(AutoOrQuantityOrPercentage))),
            _ => Err(unknown()),
        }
    }
}

impl CssWideProperties for Logical {
    fn apply_css_wide(
        _name: &str,
        _keyword: CssWide,
        _computed: &mut ComputedValues,
        _parent: &ComputedValues,
        _initial: &ComputedValues,
    ) -> bool {
        false
    }
}

impl ApplyToComputedValues for Logical {
    fn apply_to(&self, computed: &mut ComputedValues) {
        let (mode, direction) = (computed.text.writing_mode, computed.text.direction);
        let side = |axis, end| physical(mode, direction, axis, end);
        // applies a value to the property of the physical side that the logical side is on
        macro_rules! sides {
            ($edges:expr, $top:ident, $right:ident, $bottom:ident, $left:ident) => {{
                let edges = $edges;
                for (axis, end, value) in [
                    (Axis::Block, false, edges.block_start),
                    (Axis::Block, true, edges.block_end),
                    (Axis::Inline, false, edges.inline_start),
                    (Axis::Inline, true, edges.inline_end),
                ] {
                    let Some(value) = value else { continue };
                    let declaration = match side(axis, end) {
                        Side::Top => Position::$top(value),
                        Side::Right => Position::$right(value),
                        Side::Bottom => Position::$bottom(value),
                        Side::Left => Position::$left(value),
                    };
                    declaration.apply_to(computed);
                }
            }};
        }
        match self {
            Logical::Margin(edges) => sides!(edges, MarginTop, MarginRight, MarginBottom, MarginLeft),
            Logical::Padding(edges) => sides!(edges, PaddingTop, PaddingRight, PaddingBottom, PaddingLeft),
            Logical::Inset(edges) => sides!(edges, Top, Right, Bottom, Left),
            Logical::Size(axis, value) => if is_width(mode, *axis) {
                Position::Width(*value)
            } else {
                Position::Height(*value)
            }
            .apply_to(computed),
            Logical::MinSize(axis, value) => if is_width(mode, *axis) {
                Position::MinWidth(*value)
            } else {
                Position::MinHeight(*value)
            }
            .apply_to(computed),
            Logical::MaxSize(axis, value) => if is_width(mode, *axis) {
                Position::MaxWidth(*value)
            } else {
                Position::MaxHeight(*value)
            }
            .apply_to(computed),
        }
    }
}
