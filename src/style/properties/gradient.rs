//! CSS gradients: `linear-gradient()`, `radial-gradient()` and `conic-gradient()` with their
//! `repeating-` variants, as images of `background-image`.
//!
//! A gradient goes through three stages. The parsed gradient (`Gradient<QuantityOrPercentage,
//! Color>`) keeps lengths as written. The computed gradient has its lengths in pixels
//! (`Gradient<LengthPercentage, Color>`), and once `currentcolor` is known also real colours
//! (`Gradient<LengthPercentage, RGBA>`). Percentages need the size of the box, so the last step
//! is [`Gradient::resolve`], which produces the geometry and the colour stops in the space of
//! the page.
//!
//! https://www.w3.org/TR/css-images-3/#gradients and
//! https://www.w3.org/TR/css-images-4/#conic-gradients
//!
//! Not supported: colour hints (a bare percentage between two stops), `<position>` with four
//! values (`right 10px bottom 5px`), and interpolation in colour spaces other than sRGB.

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};
use cssparser_color::Color;

use super::computed::{ComputedValues, LengthPercentage, PixelLength, RGBA};
use super::specified::{CommaSeparated, ComputedValue, PropertyValue, QuantityOrPercentage};
use crate::style::parse::StyleParseError;

type ParseResult<T> = Result<T, ParseError<StyleParseError>>;

/// A value of `background-image`.
#[derive(Debug, Clone, PartialEq)]
pub enum Image<L, C> {
    None,
    Gradient(Gradient<L, C>),
}

/// A gradient with lengths of type `L` and colours of type `C`.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient<L, C> {
    pub kind: GradientKind<L>,
    pub repeating: bool,
    /// At least two stops.
    pub stops: Vec<Stop<L, C>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GradientKind<L> {
    Linear(LinearDirection),
    Radial(Radial<L>),
    Conic(Conic<L>),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// The direction of a linear gradient.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LinearDirection {
    /// In degrees, clockwise from pointing up (`to top`).
    Angle(f32),
    /// `to top`, `to right`, ...
    Side(Side),
    /// `to top right`, ... The direction depends on the aspect ratio of the box such that the
    /// 50% line goes through the two other corners.
    Corner { horizontal: Side, vertical: Side },
}

impl Default for LinearDirection {
    fn default() -> Self {
        LinearDirection::Side(Side::Bottom)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Extent {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RadialSize<L> {
    Extent(Extent),
    /// The horizontal and vertical radius. A circle has the same value twice.
    Explicit(L, L),
}

/// A position in the box: the offset of the center of the gradient from the left and top edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Position<L> {
    pub x: L,
    pub y: L,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Radial<L> {
    pub circle: bool,
    pub size: RadialSize<L>,
    pub position: Position<L>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conic<L> {
    /// The angle of the start of the gradient, in degrees clockwise from pointing up.
    pub from: f32,
    pub position: Position<L>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopPosition<L> {
    Length(L),
    /// In degrees. Only for conic gradients.
    Angle(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stop<L, C> {
    pub color: C,
    /// A stop with two positions is two stops with the same colour.
    pub positions: Vec<StopPosition<L>>,
}

/// The parsed value of `background-image`.
pub type SpecifiedImage = Image<QuantityOrPercentage, Color>;
/// With lengths in pixels. The colour may still be `currentcolor`.
pub type ComputedImage = Image<LengthPercentage, Color>;
/// With colours resolved against `color`.
pub type ResolvedGradient = Gradient<LengthPercentage, RGBA>;

// ---------------------------------------------------------------------------------------------
// parsing

enum Function {
    Linear,
    Radial,
    Conic,
}

impl<'i> PropertyValue<'i> for SpecifiedImage {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
            return Ok(Image::None);
        }
        let name = input.expect_function()?.clone();
        let (function, repeating) = match_ignore_ascii_case! { &name,
            "linear-gradient" => (Function::Linear, false),
            "repeating-linear-gradient" => (Function::Linear, true),
            "radial-gradient" => (Function::Radial, false),
            "repeating-radial-gradient" => (Function::Radial, true),
            "conic-gradient" => (Function::Conic, false),
            "repeating-conic-gradient" => (Function::Conic, true),
            _ => return Err(ParseError::unexpected_token()),
        };
        input
            .parse_nested_block(|input| parse_gradient(function, repeating, input))
            .map(Image::Gradient)
    }
}

fn parse_gradient<'i>(
    function: Function,
    repeating: bool,
    input: &mut Parser<'i>,
) -> ParseResult<Gradient<QuantityOrPercentage, Color>> {
    // the optional first argument, which configures the shape, is followed by a comma
    fn leading<'i, T>(
        input: &mut Parser<'i>,
        parse: impl FnOnce(&mut Parser<'i>) -> ParseResult<T>,
    ) -> Option<T> {
        input
            .try_parse(|input| {
                let value = parse(input)?;
                input.expect_comma()?;
                Ok::<_, ParseError<StyleParseError>>(value)
            })
            .ok()
    }

    let kind = match function {
        Function::Linear => GradientKind::Linear(leading(input, parse_linear_direction).unwrap_or_default()),
        Function::Radial => GradientKind::Radial(leading(input, parse_radial).unwrap_or_else(|| Radial {
            circle: false,
            size: RadialSize::Extent(Extent::FarthestCorner),
            position: Position::center(),
        })),
        Function::Conic => GradientKind::Conic(leading(input, parse_conic).unwrap_or_else(|| Conic {
            from: 0.0,
            position: Position::center(),
        })),
    };
    let conic = matches!(kind, GradientKind::Conic(_));

    let mut stops = vec![];
    loop {
        stops.push(parse_stop(input, conic)?);
        if input.try_parse(|input| input.expect_comma()).is_err() {
            break;
        }
    }
    if !input.is_exhausted() || stops.len() < 2 {
        return Err(input.new_error_for_next_token());
    }
    Ok(Gradient { kind, repeating, stops })
}

fn parse_angle<'i>(input: &mut Parser<'i>) -> ParseResult<f32> {
    let token = input.next()?.clone();
    match token {
        Token::Dimension { value, ref unit, .. } => {
            match_ignore_ascii_case! { unit,
                "deg" => Ok(value),
                "grad" => Ok(value * 0.9),
                "rad" => Ok(value.to_degrees()),
                "turn" => Ok(value * 360.0),
                _ => Err(ParseError::unexpected_token()),
            }
        }
        // a unitless zero is an angle only in the legacy syntax of `linear-gradient()`
        Token::Number { value, .. } if value == 0.0 => Ok(0.0),
        _ => Err(ParseError::unexpected_token()),
    }
}

fn parse_side<'i>(input: &mut Parser<'i>) -> ParseResult<Side> {
    let ident = input.expect_ident()?.clone();
    match_ignore_ascii_case! { &ident,
        "left" => Ok(Side::Left),
        "right" => Ok(Side::Right),
        "top" => Ok(Side::Top),
        "bottom" => Ok(Side::Bottom),
        _ => Err(ParseError::unexpected_token()),
    }
}

fn parse_linear_direction<'i>(input: &mut Parser<'i>) -> ParseResult<LinearDirection> {
    if let Ok(angle) = input.try_parse(parse_angle) {
        return Ok(LinearDirection::Angle(angle));
    }
    input.expect_ident_matching("to")?;
    let first = parse_side(input)?;
    let second = input.try_parse(parse_side).ok();
    let horizontal = |s: Side| matches!(s, Side::Left | Side::Right);
    match second {
        None => Ok(LinearDirection::Side(first)),
        Some(second) if horizontal(first) && !horizontal(second) => {
            Ok(LinearDirection::Corner { horizontal: first, vertical: second })
        }
        Some(second) if !horizontal(first) && horizontal(second) => {
            Ok(LinearDirection::Corner { horizontal: second, vertical: first })
        }
        Some(_) => Err(input.new_error_for_next_token()),
    }
}

fn parse_length_percentage<'i>(input: &mut Parser<'i>) -> ParseResult<QuantityOrPercentage> {
    input
        .try_parse(QuantityOrPercentage::from_css_property_value)
        .map_err(|_| input.new_error_for_next_token())
}

fn parse_radial<'i>(input: &mut Parser<'i>) -> ParseResult<Radial<QuantityOrPercentage>> {
    enum Shape {
        Circle,
        Ellipse,
    }
    enum Size {
        Extent(Extent),
        Explicit(QuantityOrPercentage, Option<QuantityOrPercentage>),
    }
    fn shape<'i>(input: &mut Parser<'i>) -> ParseResult<Shape> {
        let ident = input.expect_ident()?.clone();
        match_ignore_ascii_case! { &ident,
            "circle" => Ok(Shape::Circle),
            "ellipse" => Ok(Shape::Ellipse),
            _ => Err(ParseError::unexpected_token()),
        }
    }
    fn size<'i>(input: &mut Parser<'i>) -> ParseResult<Size> {
        let keyword = input.try_parse(|input| -> ParseResult<Extent> {
            let ident = input.expect_ident()?.clone();
            match_ignore_ascii_case! { &ident,
                "closest-side" => Ok(Extent::ClosestSide),
                "closest-corner" => Ok(Extent::ClosestCorner),
                "farthest-side" => Ok(Extent::FarthestSide),
                "farthest-corner" => Ok(Extent::FarthestCorner),
                _ => Err(ParseError::unexpected_token()),
            }
        });
        if let Ok(extent) = keyword {
            return Ok(Size::Extent(extent));
        }
        let first = parse_length_percentage(input)?;
        let second = input.try_parse(parse_length_percentage).ok();
        Ok(Size::Explicit(first, second))
    }

    let mut parsed_shape = None;
    let mut parsed_size = None;
    loop {
        if parsed_shape.is_none() {
            if let Ok(s) = input.try_parse(shape) {
                parsed_shape = Some(s);
                continue;
            }
        }
        if parsed_size.is_none() {
            if let Ok(s) = input.try_parse(size) {
                parsed_size = Some(s);
                continue;
            }
        }
        break;
    }
    let position = input
        .try_parse(|input| {
            input.expect_ident_matching("at")?;
            parse_position(input)
        })
        .ok();
    if parsed_shape.is_none() && parsed_size.is_none() && position.is_none() {
        return Err(input.new_error_for_next_token());
    }

    // a circle takes one radius that is a length, an ellipse two lengths or percentages
    let is_length = |q: &QuantityOrPercentage| matches!(q, QuantityOrPercentage::Quantity(_));
    let (circle, size) = match (parsed_shape, parsed_size) {
        (Some(Shape::Circle), None) => (true, RadialSize::Extent(Extent::FarthestCorner)),
        (Some(Shape::Ellipse) | None, None) => (false, RadialSize::Extent(Extent::FarthestCorner)),
        (Some(Shape::Circle) | None, Some(Size::Explicit(r, None))) if is_length(&r) => {
            (true, RadialSize::Explicit(r, r))
        }
        (Some(Shape::Ellipse) | None, Some(Size::Explicit(rx, Some(ry)))) => {
            (false, RadialSize::Explicit(rx, ry))
        }
        (Some(Shape::Circle), Some(Size::Extent(extent))) => (true, RadialSize::Extent(extent)),
        (Some(Shape::Ellipse) | None, Some(Size::Extent(extent))) => (false, RadialSize::Extent(extent)),
        _ => return Err(input.new_error_for_next_token()),
    };
    Ok(Radial {
        circle,
        size,
        position: position.unwrap_or_else(Position::center),
    })
}

fn parse_conic<'i>(input: &mut Parser<'i>) -> ParseResult<Conic<QuantityOrPercentage>> {
    let from = input
        .try_parse(|input| {
            input.expect_ident_matching("from")?;
            parse_angle(input)
        })
        .ok();
    let position = input
        .try_parse(|input| {
            input.expect_ident_matching("at")?;
            parse_position(input)
        })
        .ok();
    if from.is_none() && position.is_none() {
        return Err(input.new_error_for_next_token());
    }
    Ok(Conic {
        from: from.unwrap_or(0.0),
        position: position.unwrap_or_else(Position::center),
    })
}

impl Position<QuantityOrPercentage> {
    fn center() -> Self {
        Position {
            x: QuantityOrPercentage::Percentage(super::specified::Percentage(0.5)),
            y: QuantityOrPercentage::Percentage(super::specified::Percentage(0.5)),
        }
    }
}

/// `<position>` with one or two values.
fn parse_position<'i>(input: &mut Parser<'i>) -> ParseResult<Position<QuantityOrPercentage>> {
    #[derive(Copy, Clone)]
    enum Item {
        Keyword(Option<Side>),
        Value(QuantityOrPercentage),
    }
    #[derive(PartialEq)]
    enum Axis {
        X,
        Y,
        Either,
    }
    fn item<'i>(input: &mut Parser<'i>) -> ParseResult<Item> {
        if let Ok(value) = input.try_parse(parse_length_percentage) {
            return Ok(Item::Value(value));
        }
        if input.try_parse(|input| input.expect_ident_matching("center")).is_ok() {
            return Ok(Item::Keyword(None));
        }
        parse_side(input).map(|side| Item::Keyword(Some(side)))
    }
    let axis = |item: &Item| match item {
        Item::Keyword(Some(Side::Left | Side::Right)) => Axis::X,
        Item::Keyword(Some(Side::Top | Side::Bottom)) => Axis::Y,
        _ => Axis::Either,
    };
    let percentage = |p: f32| QuantityOrPercentage::Percentage(super::specified::Percentage(p));
    let value = |item: Item| match item {
        Item::Value(value) => value,
        Item::Keyword(None) => percentage(0.5),
        Item::Keyword(Some(Side::Left | Side::Top)) => percentage(0.0),
        Item::Keyword(Some(Side::Right | Side::Bottom)) => percentage(1.0),
    };

    let first = item(input)?;
    let second = input.try_parse(item).ok();
    let (x, y) = match second {
        None => match axis(&first) {
            Axis::Y => (Item::Keyword(None), first),
            _ => (first, Item::Keyword(None)),
        },
        Some(second) => {
            let both_keywords = matches!((first, second), (Item::Keyword(_), Item::Keyword(_)));
            if axis(&first) != Axis::Y && axis(&second) != Axis::X {
                (first, second)
            } else if both_keywords && axis(&first) != Axis::X && axis(&second) != Axis::Y {
                (second, first)
            } else {
                return Err(input.new_error_for_next_token());
            }
        }
    };
    Ok(Position { x: value(x), y: value(y) })
}

fn parse_stop<'i>(
    input: &mut Parser<'i>,
    conic: bool,
) -> ParseResult<Stop<QuantityOrPercentage, Color>> {
    // A bare position between two colours is a colour hint, which is not supported and fails
    // here because it is not a colour.
    let color = Color::from_css_property_value(input)?;
    let mut positions = vec![];
    while positions.len() < 2 {
        let position = input.try_parse(|input| -> ParseResult<StopPosition<QuantityOrPercentage>> {
            if conic {
                if let Ok(angle) = input.try_parse(parse_angle) {
                    return Ok(StopPosition::Angle(angle));
                }
                // a percentage is a fraction of the full turn, a length is not allowed
                match parse_length_percentage(input)? {
                    value @ QuantityOrPercentage::Percentage(_) => Ok(StopPosition::Length(value)),
                    QuantityOrPercentage::Quantity(_) => Err(input.new_error_for_next_token()),
                }
            } else {
                parse_length_percentage(input).map(StopPosition::Length)
            }
        });
        match position {
            Ok(position) => positions.push(position),
            Err(_) => break,
        }
    }
    Ok(Stop { color, positions })
}

/// The shorthand `background`: `<bg-layer>#? , <final-bg-layer>` where a layer is an image,
/// and the last layer may also have a colour. Positions, sizes, repeat and the other values of
/// a layer are not supported.
#[derive(Debug, Clone)]
pub struct BackgroundShorthand {
    color: Color,
    images: Vec<SpecifiedImage>,
}

impl BackgroundShorthand {
    pub fn color(&self) -> Color {
        self.color
    }

    pub fn images(&self) -> CommaSeparated<SpecifiedImage> {
        CommaSeparated::new(self.images.clone())
    }
}

impl<'i> PropertyValue<'i> for BackgroundShorthand {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        let mut color = None;
        let mut images = vec![];
        loop {
            // the image and the colour of a layer can come in either order
            let mut image = None;
            loop {
                if image.is_none() {
                    if let Ok(parsed) = input.try_parse(SpecifiedImage::from_css_property_value) {
                        image = Some(parsed);
                        continue;
                    }
                }
                if color.is_none() {
                    if let Ok(parsed) = input.try_parse(Color::from_css_property_value) {
                        color = Some(parsed);
                        continue;
                    }
                }
                break;
            }
            if image.is_none() && color.is_none() {
                return Err(input.new_error_for_next_token());
            }
            images.push(image.unwrap_or(Image::None));
            if input.try_parse(|input| input.expect_comma()).is_err() {
                break;
            }
            // only the final layer has a colour
            if color.is_some() {
                return Err(input.new_error_for_next_token());
            }
        }
        if !input.is_exhausted() {
            return Err(input.new_error_for_next_token());
        }
        Ok(BackgroundShorthand {
            color: color.unwrap_or(Color::Rgba(cssparser_color::RgbaLegacy::new(0, 0, 0, 0.0))),
            images,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// computed values

impl ComputedValue<CommaSeparated<SpecifiedImage>> for Vec<ComputedImage> {
    fn from_specified(specified: &CommaSeparated<SpecifiedImage>, computed: &ComputedValues) -> Self {
        specified
            .elements()
            .iter()
            .map(|image| match image {
                Image::None => Image::None,
                Image::Gradient(gradient) => Image::Gradient(gradient.compute(computed)),
            })
            .collect()
    }
}

impl Gradient<QuantityOrPercentage, Color> {
    fn compute(&self, computed: &ComputedValues) -> Gradient<LengthPercentage, Color> {
        let length = |q: &QuantityOrPercentage| LengthPercentage::from_specified(q, computed);
        let position = |p: &Position<QuantityOrPercentage>| Position { x: length(&p.x), y: length(&p.y) };
        let kind = match &self.kind {
            GradientKind::Linear(direction) => GradientKind::Linear(*direction),
            GradientKind::Radial(radial) => GradientKind::Radial(Radial {
                circle: radial.circle,
                size: match &radial.size {
                    RadialSize::Extent(extent) => RadialSize::Extent(*extent),
                    RadialSize::Explicit(rx, ry) => RadialSize::Explicit(length(rx), length(ry)),
                },
                position: position(&radial.position),
            }),
            GradientKind::Conic(conic) => GradientKind::Conic(Conic {
                from: conic.from,
                position: position(&conic.position),
            }),
        };
        let stops = self
            .stops
            .iter()
            .map(|stop| Stop {
                color: stop.color,
                positions: stop
                    .positions
                    .iter()
                    .map(|p| match p {
                        StopPosition::Length(q) => StopPosition::Length(length(q)),
                        StopPosition::Angle(angle) => StopPosition::Angle(*angle),
                    })
                    .collect(),
            })
            .collect();
        Gradient { kind, repeating: self.repeating, stops }
    }
}

impl Gradient<LengthPercentage, Color> {
    /// Resolves `currentcolor` to `current`.
    pub fn with_current_color(&self, current: RGBA) -> ResolvedGradient {
        use super::color::color_to_rgba;
        Gradient {
            kind: self.kind.clone(),
            repeating: self.repeating,
            stops: self
                .stops
                .iter()
                .map(|stop| Stop {
                    color: match stop.color {
                        Color::CurrentColor => current,
                        color => color_to_rgba(color),
                    },
                    positions: stop.positions.clone(),
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// resolving against the box

/// The box a gradient is painted into (the padding box).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BoxSize {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where a gradient goes in the space of the page (y pointing down).
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Geometry {
    /// Offset 0 is at `start` and offset 1 at `end`.
    Linear { start: (f32, f32), end: (f32, f32) },
    /// The ellipse with the center and radii at offset 1.
    Radial { center: (f32, f32), radius: (f32, f32) },
    /// Offset 0 is at `from` degrees clockwise from pointing up and offset 1 is a full turn later.
    /// `radius` reaches the farthest corner of the box.
    Conic { center: (f32, f32), from: f32, radius: f32 },
}

/// A colour stop at `offset` along the gradient, with straight (not premultiplied) components
/// in `0..=1`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ResolvedStop {
    pub offset: f32,
    pub color: [f32; 4],
}

/// A gradient for a given box, ready to be painted.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The box that the gradient has to cover: the border box, which is larger than the box
    /// that positions the gradient when there is a border.
    pub paint: BoxSize,
    pub geometry: Geometry,
    /// Offsets are increasing in `0..=1`; with `Linear` and `Radial` the colours outside are the
    /// ones of the first and last stop. There is at least one stop.
    pub stops: Vec<ResolvedStop>,
}

/// More repeats than this are painted as the average colour.
const MAX_REPEATS: f32 = 1000.0;
/// The number of steps a transition between different opacities is split into, to interpolate
/// like premultiplied colours.
const ALPHA_STEPS: usize = 8;

impl ResolvedGradient {
    /// The gradient for the box `area` (the padding box), to paint into `paint` (the border
    /// box). `None` if the box has no area.
    pub fn resolve(&self, area: BoxSize, paint: BoxSize) -> Option<Resolved> {
        if area.width <= 0.0 || area.height <= 0.0 {
            return None;
        }
        let px = |lp: &LengthPercentage, container: f32| lp.absolute(PixelLength(container)).0;
        let position = |p: &Position<LengthPercentage>| {
            (area.x + px(&p.x, area.width), area.y + px(&p.y, area.height))
        };

        // the geometry, and the length that stop positions in lengths are relative to
        let (geometry, length) = match &self.kind {
            GradientKind::Linear(direction) => {
                let (w, h) = (area.width, area.height);
                let (dx, dy) = match *direction {
                    LinearDirection::Angle(degrees) => {
                        let radians = degrees.to_radians();
                        (radians.sin(), -radians.cos())
                    }
                    LinearDirection::Side(Side::Top) => (0.0, -1.0),
                    LinearDirection::Side(Side::Bottom) => (0.0, 1.0),
                    LinearDirection::Side(Side::Left) => (-1.0, 0.0),
                    LinearDirection::Side(Side::Right) => (1.0, 0.0),
                    LinearDirection::Corner { horizontal, vertical } => {
                        let sx = if horizontal == Side::Right { 1.0 } else { -1.0 };
                        let sy = if vertical == Side::Bottom { 1.0 } else { -1.0 };
                        let (vx, vy) = (sx * h, sy * w);
                        let norm = vx.hypot(vy);
                        (vx / norm, vy / norm)
                    }
                };
                let length = (w * dx).abs() + (h * dy).abs();
                let (cx, cy) = (area.x + w / 2.0, area.y + h / 2.0);
                let half = length / 2.0;
                (
                    Geometry::Linear {
                        start: (cx - dx * half, cy - dy * half),
                        end: (cx + dx * half, cy + dy * half),
                    },
                    length,
                )
            }
            GradientKind::Radial(radial) => {
                let (cx, cy) = position(&radial.position);
                let (left, right) = (cx - area.x, area.x + area.width - cx);
                let (top, bottom) = (cy - area.y, area.y + area.height - cy);
                let (left, right, top, bottom) = (left.abs(), right.abs(), top.abs(), bottom.abs());
                let (rx, ry) = match &radial.size {
                    RadialSize::Explicit(rx, ry) => (px(rx, area.width), px(ry, area.height)),
                    RadialSize::Extent(extent) => {
                        let (nearest_x, nearest_y) = (left.min(right), top.min(bottom));
                        let (far_x, far_y) = (left.max(right), top.max(bottom));
                        match (extent, radial.circle) {
                            (Extent::ClosestSide, true) => {
                                let r = nearest_x.min(nearest_y);
                                (r, r)
                            }
                            (Extent::FarthestSide, true) => {
                                let r = far_x.max(far_y);
                                (r, r)
                            }
                            (Extent::ClosestCorner, true) => {
                                let r = nearest_x.hypot(nearest_y);
                                (r, r)
                            }
                            (Extent::FarthestCorner, true) => {
                                let r = far_x.hypot(far_y);
                                (r, r)
                            }
                            (Extent::ClosestSide, false) => (nearest_x, nearest_y),
                            (Extent::FarthestSide, false) => (far_x, far_y),
                            // the ellipse through the corner with the aspect ratio that the
                            // ellipse of the corresponding side keyword would have
                            (Extent::ClosestCorner, false) => {
                                let scale = std::f32::consts::SQRT_2;
                                (nearest_x * scale, nearest_y * scale)
                            }
                            (Extent::FarthestCorner, false) => {
                                let scale = std::f32::consts::SQRT_2;
                                (far_x * scale, far_y * scale)
                            }
                        }
                    }
                };
                // radii of zero would divide by zero; the ellipse is then painted as if it
                // were extremely small, so the whole box gets the colour of the last stop
                let (rx, ry) = (rx.max(1e-3), ry.max(1e-3));
                (Geometry::Radial { center: (cx, cy), radius: (rx, ry) }, rx)
            }
            GradientKind::Conic(conic) => {
                let center = position(&conic.position);
                let farthest_x = (center.0 - paint.x).abs().max((paint.x + paint.width - center.0).abs());
                let farthest_y = (center.1 - paint.y).abs().max((paint.y + paint.height - center.1).abs());
                let radius = farthest_x.hypot(farthest_y);
                (Geometry::Conic { center, from: conic.from, radius }, 360.0)
            }
        };

        let stops = resolve_stop_offsets(&self.stops, length, matches!(self.kind, GradientKind::Conic(_)));
        let stops = if self.repeating {
            repeat_stops(stops)
        } else {
            stops
        };
        let stops = clip_stops(stops);
        Some(Resolved { paint, geometry, stops: with_premultiplied_steps(&stops) })
    }
}

/// Assigns an offset to every stop and returns them in order. Offsets are fractions of the
/// gradient line (or of the full turn for a conic gradient).
/// https://www.w3.org/TR/css-images-3/#color-stop-fixup
fn resolve_stop_offsets(
    stops: &[Stop<LengthPercentage, RGBA>],
    length: f32,
    conic: bool,
) -> Vec<(f32, RGBA)> {
    let to_offset = |position: &StopPosition<LengthPercentage>| match position {
        StopPosition::Angle(degrees) => degrees / 360.0,
        StopPosition::Length(lp) if conic => lp.absolute(PixelLength(1.0)).0,
        StopPosition::Length(lp) => lp.absolute(PixelLength(length)).0 / length,
    };

    // a stop with two positions is two stops
    let mut list: Vec<(Option<f32>, RGBA)> = vec![];
    for stop in stops {
        match stop.positions.as_slice() {
            [] => list.push((None, stop.color)),
            positions => {
                for position in positions {
                    list.push((Some(to_offset(position)), stop.color));
                }
            }
        }
    }

    if let Some(first) = list.first_mut() {
        first.0.get_or_insert(0.0);
    }
    if let Some(last) = list.last_mut() {
        last.0.get_or_insert(1.0);
    }
    // an offset smaller than an earlier one is raised to it
    let mut highest = f32::NEG_INFINITY;
    for (offset, _) in list.iter_mut() {
        if let Some(offset) = offset {
            if *offset < highest {
                *offset = highest;
            }
            highest = *offset;
        }
    }
    // runs of stops without a position are spread evenly between the stops around them
    let mut index = 0;
    while index < list.len() {
        if list[index].0.is_some() {
            index += 1;
            continue;
        }
        let run_start = index;
        while list[index].0.is_none() {
            index += 1;
        }
        let before = list[run_start - 1].0.unwrap();
        let after = list[index].0.unwrap();
        let count = (index - run_start + 1) as f32;
        for (n, entry) in list[run_start..index].iter_mut().enumerate() {
            entry.0 = Some(before + (after - before) * (n + 1) as f32 / count);
        }
    }
    list.into_iter().map(|(offset, color)| (offset.unwrap(), color)).collect()
}

/// Repeats the stops over the range `0..=1`.
fn repeat_stops(stops: Vec<(f32, RGBA)>) -> Vec<(f32, RGBA)> {
    let first = stops.first().map_or(0.0, |s| s.0);
    let last = stops.last().map_or(0.0, |s| s.0);
    let period = last - first;
    let first_copy = ((0.0 - last) / period).floor();
    let last_copy = ((1.0 - first) / period).ceil();
    if !(period > 0.0) || !(last_copy - first_copy < MAX_REPEATS) {
        // all stops at the same position, or too many to be told apart: the average colour
        let n = stops.len().max(1) as f32;
        let mean = |channel: fn(&RGBA) -> u8| {
            (stops.iter().map(|s| f32::from(channel(&s.1))).sum::<f32>() / n).round() as u8
        };
        let color = RGBA::new(mean(|c| c.red), mean(|c| c.green), mean(|c| c.blue), mean(|c| c.alpha));
        return vec![(0.0, color)];
    }
    let mut repeated = Vec::with_capacity(stops.len() * (last_copy - first_copy + 1.0) as usize);
    let mut copy = first_copy;
    while copy <= last_copy {
        repeated.extend(stops.iter().map(|&(offset, color)| (offset + copy * period, color)));
        copy += 1.0;
    }
    repeated
}

fn rgba_components(color: RGBA) -> [f32; 4] {
    let channel = |value: u8| f32::from(value) / 255.0;
    [channel(color.red), channel(color.green), channel(color.blue), channel(color.alpha)]
}

/// Interpolates premultiplied colours.
fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let alpha = a[3] + (b[3] - a[3]) * t;
    if alpha <= 0.0 {
        // fully transparent: the color does not matter, keep the one of the nearer stop
        return if t < 0.5 { [a[0], a[1], a[2], 0.0] } else { [b[0], b[1], b[2], 0.0] };
    }
    let mut result = [0.0, 0.0, 0.0, alpha];
    for i in 0..3 {
        let premultiplied = a[i] * a[3] * (1.0 - t) + b[i] * b[3] * t;
        result[i] = premultiplied / alpha;
    }
    result
}

/// Cuts the stops to the range `0..=1`, with stops at the ends if they are in the middle of a
/// transition.
fn clip_stops(stops: Vec<(f32, RGBA)>) -> Vec<ResolvedStop> {
    let stops: Vec<(f32, [f32; 4])> = stops.into_iter().map(|(o, c)| (o, rgba_components(c))).collect();
    let color_at = |at: f32| -> [f32; 4] {
        // the colour of a hard transition at `at` is the one after it
        let mut previous = stops[0];
        if at < previous.0 {
            return previous.1;
        }
        for &next in &stops[1..] {
            if at < next.0 {
                let t = (at - previous.0) / (next.0 - previous.0);
                return mix(previous.1, next.1, t);
            }
            previous = next;
        }
        previous.1
    };

    let mut result = vec![];
    let first = stops[0].0;
    let last = stops[stops.len() - 1].0;
    if first >= 0.0 || stops.len() == 1 {
        // the first stop is inside; the rest follows
    } else {
        result.push(ResolvedStop { offset: 0.0, color: color_at(0.0) });
    }
    for &(offset, color) in &stops {
        if offset >= 0.0 && offset <= 1.0 {
            result.push(ResolvedStop { offset, color });
        }
    }
    if last > 1.0 {
        // the colour just before 1.0 for a stop pair that straddles it
        let color = {
            let mut previous = stops[0];
            let mut color = stops[0].1;
            for &next in &stops[1..] {
                if next.0 > 1.0 && previous.0 <= 1.0 && next.0 > previous.0 {
                    color = mix(previous.1, next.1, (1.0 - previous.0) / (next.0 - previous.0));
                    break;
                }
                previous = next;
            }
            color
        };
        result.push(ResolvedStop { offset: 1.0, color });
    }
    if result.is_empty() {
        // all stops are outside of the range, on the same side
        let color = if last < 0.0 { stops[stops.len() - 1].1 } else { stops[0].1 };
        result.push(ResolvedStop { offset: 0.0, color });
    }
    result
}

/// Adds stops in transitions between different opacities, so that linear interpolation between
/// the stops of straight colours gives (nearly) the colours of premultiplied interpolation.
fn with_premultiplied_steps(stops: &[ResolvedStop]) -> Vec<ResolvedStop> {
    let mut result = Vec::with_capacity(stops.len());
    for (index, &stop) in stops.iter().enumerate() {
        result.push(stop);
        if let Some(&next) = stops.get(index + 1) {
            if (stop.color[3] - next.color[3]).abs() > f32::EPSILON && next.offset > stop.offset {
                for step in 1..ALPHA_STEPS {
                    let t = step as f32 / ALPHA_STEPS as f32;
                    result.push(ResolvedStop {
                        offset: stop.offset + (next.offset - stop.offset) * t,
                        color: mix(stop.color, next.color, t),
                    });
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(css: &str) -> Option<SpecifiedImage> {
        let mut parser = Parser::new(css);
        let image = SpecifiedImage::from_css_property_value(&mut parser).ok()?;
        parser.is_exhausted().then_some(image)
    }

    fn gradient(css: &str) -> Gradient<QuantityOrPercentage, Color> {
        match parse(css) {
            Some(Image::Gradient(gradient)) => gradient,
            other => panic!("{css:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn linear_defaults_to_bottom() {
        let g = gradient("linear-gradient(red, blue)");
        assert_eq!(g.kind, GradientKind::Linear(LinearDirection::Side(Side::Bottom)));
        assert_eq!(g.stops.len(), 2);
        assert!(!g.repeating);
    }

    #[test]
    fn linear_directions() {
        let direction = |css| match gradient(css).kind {
            GradientKind::Linear(d) => d,
            _ => panic!(),
        };
        assert_eq!(direction("linear-gradient(45deg, red, blue)"), LinearDirection::Angle(45.0));
        assert_eq!(direction("linear-gradient(0.25turn, red, blue)"), LinearDirection::Angle(90.0));
        assert_eq!(
            direction("linear-gradient(to right, red, blue)"),
            LinearDirection::Side(Side::Right)
        );
        let corner = LinearDirection::Corner { horizontal: Side::Left, vertical: Side::Top };
        assert_eq!(direction("linear-gradient(to left top, red, blue)"), corner);
        assert_eq!(direction("linear-gradient(to top left, red, blue)"), corner);
        assert!(parse("linear-gradient(to left right, red, blue)").is_none());
    }

    #[test]
    fn stops_have_up_to_two_positions() {
        let g = gradient("linear-gradient(red 10%, yellow 20% 30%, blue)");
        let counts: Vec<_> = g.stops.iter().map(|s| s.positions.len()).collect();
        assert_eq!(counts, [1, 2, 0]);
        assert!(parse("linear-gradient(red 1px 2px 3px, blue)").is_none());
    }

    #[test]
    fn invalid_gradients_are_rejected() {
        assert!(parse("linear-gradient(red)").is_none());
        assert!(parse("linear-gradient()").is_none());
        assert!(parse("linear-gradient(red, 50%, blue)").is_none(), "colour hints are unsupported");
        assert!(parse("linear-gradient(red 10deg, blue)").is_none());
        assert!(parse("foo-gradient(red, blue)").is_none());
        assert!(parse("conic-gradient(red 10px, blue)").is_none());
    }

    #[test]
    fn none_is_an_image() {
        assert_eq!(parse("none"), Some(Image::None));
    }

    #[test]
    fn radial_shapes_and_sizes() {
        let radial = |css| match gradient(css).kind {
            GradientKind::Radial(r) => r,
            _ => panic!(),
        };
        let r = radial("radial-gradient(red, blue)");
        assert!(!r.circle);
        assert_eq!(r.size, RadialSize::Extent(Extent::FarthestCorner));
        let r = radial("radial-gradient(circle closest-side at 10px 20%, red, blue)");
        assert!(r.circle);
        assert_eq!(r.size, RadialSize::Extent(Extent::ClosestSide));
        let r = radial("radial-gradient(40px at top left, red, blue)");
        assert!(r.circle, "a single length is a circle");
        assert!(matches!(r.size, RadialSize::Explicit(..)));
        let r = radial("radial-gradient(40px 20px, red, blue)");
        assert!(!r.circle);
        // percentages are not valid radii for circles
        assert!(parse("radial-gradient(circle 50%, red, blue)").is_none());
        assert!(parse("radial-gradient(circle 10px 20px, red, blue)").is_none());
        assert!(parse("radial-gradient(ellipse 10px, red, blue)").is_none());
    }

    #[test]
    fn positions() {
        let position = |css| match gradient(css).kind {
            GradientKind::Radial(r) => (r.position.x, r.position.y),
            _ => panic!(),
        };
        let pct = |p: f32| QuantityOrPercentage::Percentage(super::super::specified::Percentage(p));
        assert_eq!(position("radial-gradient(at left, red, blue)"), (pct(0.0), pct(0.5)));
        assert_eq!(position("radial-gradient(at top, red, blue)"), (pct(0.5), pct(0.0)));
        assert_eq!(position("radial-gradient(at bottom right, red, blue)"), (pct(1.0), pct(1.0)));
        assert_eq!(position("radial-gradient(at right bottom, red, blue)"), (pct(1.0), pct(1.0)));
        assert_eq!(position("radial-gradient(at center top, red, blue)"), (pct(0.5), pct(0.0)));
        assert!(parse("radial-gradient(at left right, red, blue)").is_none());
        assert!(parse("radial-gradient(at top bottom, red, blue)").is_none());
    }

    #[test]
    fn conic() {
        let g = gradient("conic-gradient(from 90deg at 25% 75%, red 0deg, blue 50%, red)");
        match g.kind {
            GradientKind::Conic(c) => assert_eq!(c.from, 90.0),
            _ => panic!(),
        }
        assert!(matches!(g.stops[0].positions[0], StopPosition::Angle(_)));
        assert!(parse("repeating-conic-gradient(red, blue 30deg)").is_some());
    }

    // computed gradients for the tests of resolving

    fn px(value: f32) -> LengthPercentage {
        LengthPercentage::PixelLength(PixelLength(value))
    }

    fn pct(value: f32) -> LengthPercentage {
        LengthPercentage::Percentage(super::super::specified::Percentage(value))
    }

    const RED: RGBA = RGBA { red: 255, green: 0, blue: 0, alpha: 255 };
    const BLUE: RGBA = RGBA { red: 0, green: 0, blue: 255, alpha: 255 };
    const BOX: BoxSize = BoxSize { x: 10.0, y: 20.0, width: 200.0, height: 100.0 };

    fn stop(color: RGBA, positions: &[LengthPercentage]) -> Stop<LengthPercentage, RGBA> {
        Stop { color, positions: positions.iter().cloned().map(StopPosition::Length).collect() }
    }

    fn linear(direction: LinearDirection, stops: Vec<Stop<LengthPercentage, RGBA>>) -> ResolvedGradient {
        Gradient { kind: GradientKind::Linear(direction), repeating: false, stops }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn to_bottom_goes_through_the_box_vertically() {
        let g = linear(LinearDirection::default(), vec![stop(RED, &[]), stop(BLUE, &[])]);
        let resolved = g.resolve(BOX, BOX).unwrap();
        match resolved.geometry {
            Geometry::Linear { start, end } => {
                assert!(close(start.0, 110.0) && close(start.1, 20.0));
                assert!(close(end.0, 110.0) && close(end.1, 120.0));
            }
            _ => panic!(),
        }
        assert_eq!(resolved.stops.len(), 2);
        assert_eq!(resolved.stops[0].offset, 0.0);
        assert_eq!(resolved.stops[1].offset, 1.0);
        assert_eq!(resolved.stops[1].color, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn angles_use_the_length_of_the_gradient_line() {
        // 90deg: left to right across the full width
        let g = linear(LinearDirection::Angle(90.0), vec![stop(RED, &[]), stop(BLUE, &[])]);
        match g.resolve(BOX, BOX).unwrap().geometry {
            Geometry::Linear { start, end } => {
                assert!(close(start.0, 10.0) && close(end.0, 210.0));
                assert!(close(start.1, 70.0) && close(end.1, 70.0));
            }
            _ => panic!(),
        }
        // 45deg: the line is |w sin| + |h cos| long, so that the corners get the end colours
        let g = linear(LinearDirection::Angle(45.0), vec![stop(RED, &[]), stop(BLUE, &[])]);
        match g.resolve(BOX, BOX).unwrap().geometry {
            Geometry::Linear { start, end } => {
                let length = ((end.0 - start.0).powi(2) + (end.1 - start.1).powi(2)).sqrt();
                assert!(close(length, (200.0 + 100.0) * std::f32::consts::FRAC_1_SQRT_2));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn corner_directions_put_the_50_percent_line_through_the_other_corners() {
        let g = linear(
            LinearDirection::Corner { horizontal: Side::Right, vertical: Side::Top },
            vec![stop(RED, &[]), stop(BLUE, &[])],
        );
        let Geometry::Linear { start, end } = g.resolve(BOX, BOX).unwrap().geometry else { panic!() };
        // the top left and bottom right corners are on the line of offset 1/2, which is
        // perpendicular to the gradient line through the center
        let (dx, dy) = (end.0 - start.0, end.1 - start.1);
        let center = (110.0, 70.0);
        for corner in [(10.0, 20.0), (210.0, 120.0)] {
            let projection = (corner.0 - center.0) * dx + (corner.1 - center.1) * dy;
            assert!(projection.abs() < 1e-2, "{projection}");
        }
        // and the top right corner is at the end
        let projection = ((210.0 - center.0) * dx + (20.0 - center.1) * dy) / (dx * dx + dy * dy);
        assert!(close(projection, 0.5));
    }

    #[test]
    fn stop_positions_are_fixed_up() {
        // the middle stop has no position and is placed halfway; the offset that goes backwards
        // is raised
        let g = linear(
            LinearDirection::default(),
            vec![
                stop(RED, &[pct(0.2)]),
                stop(RED, &[]),
                stop(BLUE, &[pct(0.6)]),
                stop(BLUE, &[pct(0.4)]),
            ],
        );
        let offsets: Vec<f32> = g.resolve(BOX, BOX).unwrap().stops.iter().map(|s| s.offset).collect();
        assert_eq!(offsets.len(), 4);
        assert!(close(offsets[0], 0.2) && close(offsets[1], 0.4));
        assert!(close(offsets[2], 0.6) && close(offsets[3], 0.6));
    }

    #[test]
    fn lengths_are_relative_to_the_gradient_line() {
        let g = linear(LinearDirection::default(), vec![stop(RED, &[px(25.0)]), stop(BLUE, &[px(75.0)])]);
        let offsets: Vec<f32> = g.resolve(BOX, BOX).unwrap().stops.iter().map(|s| s.offset).collect();
        assert!(close(offsets[0], 0.25) && close(offsets[1], 0.75));
    }

    #[test]
    fn two_positions_make_a_hard_stop() {
        let g = linear(
            LinearDirection::default(),
            vec![stop(RED, &[pct(0.0), pct(0.5)]), stop(BLUE, &[pct(0.5), pct(1.0)])],
        );
        let stops = g.resolve(BOX, BOX).unwrap().stops;
        assert_eq!(stops.len(), 4);
        assert_eq!(stops[1].offset, stops[2].offset);
    }

    #[test]
    fn stops_outside_the_range_are_cut_with_the_colour_at_the_edge() {
        let g = linear(LinearDirection::default(), vec![stop(RED, &[pct(-1.0)]), stop(BLUE, &[pct(1.0)])]);
        let stops = g.resolve(BOX, BOX).unwrap().stops;
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].offset, 0.0);
        // halfway between red and blue
        assert!(close(stops[0].color[0], 0.5) && close(stops[0].color[2], 0.5));
        assert_eq!(stops[1].offset, 1.0);
    }

    #[test]
    fn repeating_gradients_fill_the_range() {
        let mut g = linear(LinearDirection::default(), vec![stop(RED, &[pct(0.0)]), stop(BLUE, &[pct(0.25)])]);
        g.repeating = true;
        let stops = g.resolve(BOX, BOX).unwrap().stops;
        // four periods; every period goes red to blue, with a hard step back
        assert!(stops.len() >= 8, "{stops:?}");
        assert_eq!(stops[0].color, [1.0, 0.0, 0.0, 1.0]);
        let steps: Vec<_> = stops
            .windows(2)
            .filter(|w| close(w[0].offset, w[1].offset) && w[0].offset > 0.0 && w[0].offset < 1.0)
            .collect();
        assert_eq!(steps.len(), 3);
        for window in steps {
            assert_eq!(window[0].color, [0.0, 0.0, 1.0, 1.0]);
            assert_eq!(window[1].color, [1.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn repeating_gradient_without_period_is_the_average() {
        let mut g = linear(LinearDirection::default(), vec![stop(RED, &[pct(0.5)]), stop(BLUE, &[pct(0.5)])]);
        g.repeating = true;
        let stops = g.resolve(BOX, BOX).unwrap().stops;
        assert_eq!(stops.len(), 1);
        assert!((stops[0].color[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn transparent_stops_keep_the_other_colour() {
        // premultiplied interpolation: red fades out without going through black
        let transparent = RGBA::new(0, 0, 0, 0);
        let g = linear(LinearDirection::default(), vec![stop(RED, &[]), stop(transparent, &[])]);
        let stops = g.resolve(BOX, BOX).unwrap().stops;
        assert!(stops.len() > 2);
        for stop in &stops[..stops.len() - 1] {
            assert!(close(stop.color[0], 1.0) && close(stop.color[1], 0.0), "{stop:?}");
        }
        assert!(close(stops[stops.len() / 2].color[3], 0.5) || stops.len() % 2 == 0);
        assert_eq!(stops.last().unwrap().color[3], 0.0);
    }

    fn radial(radial: Radial<LengthPercentage>) -> ResolvedGradient {
        Gradient {
            kind: GradientKind::Radial(radial),
            repeating: false,
            stops: vec![stop(RED, &[]), stop(BLUE, &[])],
        }
    }

    fn center() -> Position<LengthPercentage> {
        Position { x: pct(0.5), y: pct(0.5) }
    }

    #[test]
    fn radial_extents() {
        let radius = |circle, extent| {
            let g = radial(Radial { circle, size: RadialSize::Extent(extent), position: center() });
            match g.resolve(BOX, BOX).unwrap().geometry {
                Geometry::Radial { center, radius } => {
                    assert!(close(center.0, 110.0) && close(center.1, 70.0));
                    radius
                }
                _ => panic!(),
            }
        };
        assert_eq!(radius(true, Extent::ClosestSide), (50.0, 50.0));
        assert_eq!(radius(true, Extent::FarthestSide), (100.0, 100.0));
        let (rx, ry) = radius(false, Extent::FarthestCorner);
        // the ellipse that goes through the corner and has the aspect ratio of the box
        assert!(close(rx, 100.0 * std::f32::consts::SQRT_2) && close(ry, 50.0 * std::f32::consts::SQRT_2));
        let (rx, _) = radius(true, Extent::FarthestCorner);
        assert!(close(rx, 100.0f32.hypot(50.0)));
    }

    #[test]
    fn radial_with_a_corner_center() {
        let g = radial(Radial {
            circle: true,
            size: RadialSize::Extent(Extent::ClosestCorner),
            position: Position { x: pct(0.0), y: pct(0.0) },
        });
        let Geometry::Radial { center, radius } = g.resolve(BOX, BOX).unwrap().geometry else { panic!() };
        assert_eq!(center, (10.0, 20.0));
        assert!(radius.0 < 1e-2, "the closest corner is the center itself");
    }

    #[test]
    fn explicit_radii_resolve_percentages_against_the_box() {
        let g = radial(Radial {
            circle: false,
            size: RadialSize::Explicit(pct(0.5), px(30.0)),
            position: center(),
        });
        let Geometry::Radial { radius, .. } = g.resolve(BOX, BOX).unwrap().geometry else { panic!() };
        assert_eq!(radius, (100.0, 30.0));
    }

    #[test]
    fn conic_offsets_are_fractions_of_a_turn() {
        let g = Gradient {
            kind: GradientKind::Conic(Conic { from: 30.0, position: center() }),
            repeating: false,
            stops: vec![
                Stop { color: RED, positions: vec![StopPosition::Angle(90.0)] },
                Stop { color: BLUE, positions: vec![StopPosition::Length(pct(0.75))] },
            ],
        };
        let resolved = g.resolve(BOX, BOX).unwrap();
        let Geometry::Conic { center, from, radius } = resolved.geometry else { panic!() };
        assert_eq!((center, from), ((110.0, 70.0), 30.0));
        assert!(close(radius, 100.0f32.hypot(50.0)));
        assert!(close(resolved.stops[0].offset, 0.25) && close(resolved.stops[1].offset, 0.75));
    }

    #[test]
    fn an_empty_box_has_no_gradient() {
        let g = linear(LinearDirection::default(), vec![stop(RED, &[]), stop(BLUE, &[])]);
        assert!(g.resolve(BoxSize { width: 0.0, ..BOX }, BOX).is_none());
    }

    #[test]
    fn shorthand() {
        let parse = |css: &str| {
            let mut parser = Parser::new(css);
            BackgroundShorthand::from_css_property_value(&mut parser).ok()
        };
        let s = parse("red linear-gradient(blue, green)").unwrap();
        assert_eq!(s.images.len(), 1);
        let s = parse("linear-gradient(blue, green), radial-gradient(red, blue) #123").unwrap();
        assert_eq!(s.images.len(), 2);
        assert!(parse("#123, linear-gradient(blue, green)").is_none(), "only the last layer has a colour");
        assert!(parse("red blue").is_none());
        let s = parse("#abc").unwrap();
        assert_eq!(s.images, vec![Image::None]);
    }
}
