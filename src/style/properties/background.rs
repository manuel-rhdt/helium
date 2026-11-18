//! `background-size`, `background-position` and `background-repeat`: where the images of a
//! background go and how they are tiled.

use cssparser::{match_ignore_ascii_case, ParseError, Parser};

use super::computed::{ComputedValues, LengthPercentage, PixelLength};
use super::gradient::{parse_length_percentage, parse_side, BoxSize, Position, Resolved, ResolvedGradient, Side, Tiling};
use super::specified::{CommaSeparated, ComputedValue, Percentage, PropertyValue, QuantityOrPercentage};
use crate::style::parse::StyleParseError;

type ParseResult<T> = Result<T, ParseError<StyleParseError>>;

// ---------------------------------------------------------------------------------------------
// background-repeat

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RepeatStyle {
    Repeat,
    Space,
    Round,
    NoRepeat,
}

/// The value of `background-repeat` for one image.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Repeat {
    pub x: RepeatStyle,
    pub y: RepeatStyle,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat { x: RepeatStyle::Repeat, y: RepeatStyle::Repeat }
    }
}

fn parse_repeat_style<'i>(input: &mut Parser<'i>) -> ParseResult<RepeatStyle> {
    let ident = input.expect_ident()?.clone();
    match_ignore_ascii_case! { &ident,
        "repeat" => Ok(RepeatStyle::Repeat),
        "space" => Ok(RepeatStyle::Space),
        "round" => Ok(RepeatStyle::Round),
        "no-repeat" => Ok(RepeatStyle::NoRepeat),
        _ => Err(ParseError::unexpected_token()),
    }
}

impl<'i> PropertyValue<'i> for Repeat {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        if input.try_parse(|i| i.expect_ident_matching("repeat-x")).is_ok() {
            return Ok(Repeat { x: RepeatStyle::Repeat, y: RepeatStyle::NoRepeat });
        }
        if input.try_parse(|i| i.expect_ident_matching("repeat-y")).is_ok() {
            return Ok(Repeat { x: RepeatStyle::NoRepeat, y: RepeatStyle::Repeat });
        }
        let x = parse_repeat_style(input)?;
        let y = input.try_parse(parse_repeat_style).unwrap_or(x);
        Ok(Repeat { x, y })
    }
}

// ---------------------------------------------------------------------------------------------
// background-size

/// The value of `background-size` for one image. `None` is `auto`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Size<L> {
    Cover,
    Contain,
    Explicit(Option<L>, Option<L>),
}

impl<L> Default for Size<L> {
    fn default() -> Self {
        Size::Explicit(None, None)
    }
}

pub type SpecifiedSize = Size<QuantityOrPercentage>;
pub type ComputedSize = Size<LengthPercentage>;

impl<'i> PropertyValue<'i> for SpecifiedSize {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        if input.try_parse(|i| i.expect_ident_matching("cover")).is_ok() {
            return Ok(Size::Cover);
        }
        if input.try_parse(|i| i.expect_ident_matching("contain")).is_ok() {
            return Ok(Size::Contain);
        }
        fn component<'i>(input: &mut Parser<'i>) -> ParseResult<Option<QuantityOrPercentage>> {
            if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
                return Ok(None);
            }
            // (a negative size is invalid)
            let size = parse_length_percentage(input)?;
            let negative = match size {
                QuantityOrPercentage::Quantity(quantity) => quantity.value < 0.0,
                QuantityOrPercentage::Percentage(percentage) => percentage.0 < 0.0,
            };
            if negative {
                return Err(input.new_error_for_next_token());
            }
            Ok(Some(size))
        }
        let width = component(input)?;
        let height = input.try_parse(component).unwrap_or(None);
        Ok(Size::Explicit(width, height))
    }
}

impl ComputedValue<CommaSeparated<SpecifiedSize>> for Vec<ComputedSize> {
    fn from_specified(specified: &CommaSeparated<SpecifiedSize>, computed: &ComputedValues) -> Self {
        let length = |q: &QuantityOrPercentage| LengthPercentage::from_specified(q, computed);
        specified
            .elements()
            .iter()
            .map(|size| match size {
                Size::Cover => Size::Cover,
                Size::Contain => Size::Contain,
                Size::Explicit(w, h) => Size::Explicit(w.as_ref().map(length), h.as_ref().map(length)),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// background-position

/// The offset of an image from one edge: a distance from the start (left or top) of the
/// positioning area, or from its end (right or bottom).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Edge {
    pub from_end: bool,
    pub offset: QuantityOrPercentage,
}

impl Edge {
    fn start(offset: QuantityOrPercentage) -> Self {
        Edge { from_end: false, offset }
    }

    fn fraction(fraction: f32) -> Self {
        Edge::start(QuantityOrPercentage::Percentage(Percentage(fraction)))
    }
}

/// The value of `background-position` for one image.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SpecifiedPosition {
    pub x: Edge,
    pub y: Edge,
}

impl Default for SpecifiedPosition {
    fn default() -> Self {
        SpecifiedPosition { x: Edge::fraction(0.0), y: Edge::fraction(0.0) }
    }
}

#[derive(Copy, Clone)]
enum Item {
    Center,
    Side(Side),
    Value(QuantityOrPercentage),
}

fn parse_item<'i>(input: &mut Parser<'i>) -> ParseResult<Item> {
    if let Ok(value) = input.try_parse(parse_length_percentage) {
        return Ok(Item::Value(value));
    }
    if input.try_parse(|i| i.expect_ident_matching("center")).is_ok() {
        return Ok(Item::Center);
    }
    parse_side(input).map(Item::Side)
}

fn is_horizontal(side: Side) -> bool {
    matches!(side, Side::Left | Side::Right)
}

/// A side with its optional offset as an `Edge`, and the axis (true for horizontal).
fn edge(side: Side, offset: Option<QuantityOrPercentage>) -> (bool, Edge) {
    let end = matches!(side, Side::Right | Side::Bottom);
    let edge = match offset {
        Some(offset) => Edge { from_end: end, offset },
        None => Edge::fraction(if end { 1.0 } else { 0.0 }),
    };
    (is_horizontal(side), edge)
}

impl<'i> PropertyValue<'i> for SpecifiedPosition {
    type Error = StyleParseError;

    /// One to four values (https://www.w3.org/TR/css-backgrounds-3/#typedef-bg-position).
    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        let mut items = vec![];
        while items.len() < 4 {
            match input.try_parse(parse_item) {
                Ok(item) => items.push(item),
                Err(_) => break,
            }
        }
        let center = Edge::fraction(0.5);
        let fail = |input: &mut Parser<'i>| Err(input.new_error_for_next_token());
        let plain = |item: Item| match item {
            Item::Value(value) => Edge::start(value),
            Item::Center => Edge::fraction(0.5),
            Item::Side(side) => edge(side, None).1,
        };
        // the axis an item belongs to: `Some(true)` is horizontal
        let axis = |item: Item| match item {
            Item::Side(side) => Some(is_horizontal(side)),
            _ => None,
        };
        match items.len() {
            0 => fail(input),
            1 => Ok(match axis(items[0]) {
                Some(false) => SpecifiedPosition { x: center, y: plain(items[0]) },
                _ => SpecifiedPosition { x: plain(items[0]), y: center },
            }),
            2 => {
                let (a, b) = (items[0], items[1]);
                let both_keywords = !matches!(a, Item::Value(_)) && !matches!(b, Item::Value(_));
                if axis(a) != Some(false) && axis(b) != Some(true) {
                    Ok(SpecifiedPosition { x: plain(a), y: plain(b) })
                } else if both_keywords && axis(a) != Some(true) && axis(b) != Some(false) {
                    Ok(SpecifiedPosition { x: plain(b), y: plain(a) })
                } else {
                    fail(input)
                }
            }
            _ => {
                // keyword, optional offset, keyword, optional offset (`center` has no offset)
                let mut groups: Vec<(Option<bool>, Edge)> = vec![];
                let mut rest = items.as_slice();
                while let Some((&first, tail)) = rest.split_first() {
                    match first {
                        Item::Center => {
                            groups.push((None, center));
                            rest = tail;
                        }
                        Item::Side(side) => match tail.split_first() {
                            Some((&Item::Value(offset), tail)) => {
                                let (horizontal, edge) = edge(side, Some(offset));
                                groups.push((Some(horizontal), edge));
                                rest = tail;
                            }
                            _ => {
                                let (horizontal, edge) = edge(side, None);
                                groups.push((Some(horizontal), edge));
                                rest = tail;
                            }
                        },
                        Item::Value(_) => return fail(input),
                    }
                }
                match groups.as_slice() {
                    [(a, first), (b, second)] => match (a, b) {
                        (Some(true), Some(false)) | (Some(true), None) | (None, Some(false)) => {
                            Ok(SpecifiedPosition { x: *first, y: *second })
                        }
                        (Some(false), Some(true)) | (Some(false), None) | (None, Some(true)) => {
                            Ok(SpecifiedPosition { x: *second, y: *first })
                        }
                        _ => fail(input),
                    },
                    _ => fail(input),
                }
            }
        }
    }
}

impl ComputedValue<CommaSeparated<SpecifiedPosition>> for Vec<Position<LengthPercentage>> {
    fn from_specified(specified: &CommaSeparated<SpecifiedPosition>, computed: &ComputedValues) -> Self {
        let compute = |edge: &Edge| {
            let offset = LengthPercentage::from_specified(&edge.offset, computed);
            if !edge.from_end {
                return offset;
            }
            // `right 10px` is 100% - 10px
            match offset {
                LengthPercentage::PixelLength(length) => LengthPercentage::Both {
                    length: PixelLength(-length.0),
                    percentage: Percentage(1.0),
                },
                LengthPercentage::Percentage(Percentage(p)) => LengthPercentage::Percentage(Percentage(1.0 - p)),
                LengthPercentage::Both { length, percentage: Percentage(p) } => LengthPercentage::Both {
                    length: PixelLength(-length.0),
                    percentage: Percentage(1.0 - p),
                },
            }
        };
        specified
            .elements()
            .iter()
            .map(|position| Position { x: compute(&position.x), y: compute(&position.y) })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// the shorthand

/// What the shorthand `background` sets for one image besides the image itself.
#[derive(Debug, Copy, Clone, Default)]
pub struct LayerStyle {
    pub position: SpecifiedPosition,
    pub size: SpecifiedSize,
    pub repeat: Repeat,
    /// The `<box>` keywords of the layer: the origin, and the clip, which is the origin if only
    /// one is given.
    pub origin: Option<super::specified::BackgroundBox>,
    pub clip: Option<super::specified::BackgroundBox>,
}

/// Parses the part of a layer of the shorthand that is the position (and size), when the
/// input starts with one.
pub fn parse_position_and_size<'i>(
    input: &mut Parser<'i>,
) -> ParseResult<(SpecifiedPosition, Option<SpecifiedSize>)> {
    let position = SpecifiedPosition::from_css_property_value(input)?;
    let size = if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        Some(SpecifiedSize::from_css_property_value(input)?)
    } else {
        None
    };
    Ok((position, size))
}

// ---------------------------------------------------------------------------------------------
// layout of the tiles

/// The tiles of one axis: where the first one starts, the distance between tiles, whether
/// there are more than one, and the size of a tile.
struct AxisTiles {
    origin: f32,
    size: f32,
    step: f32,
    repeats: bool,
}

fn axis_tiles(style: RepeatStyle, mut size: f32, position: &LengthPercentage, start: f32, length: f32) -> AxisTiles {
    if style == RepeatStyle::Round {
        size = length / (length / size).round().max(1.0);
    }
    let at_position = start + position.absolute(PixelLength(length - size)).0;
    match style {
        RepeatStyle::Repeat | RepeatStyle::Round => AxisTiles { origin: at_position, size, step: size, repeats: true },
        RepeatStyle::NoRepeat => AxisTiles { origin: at_position, size, step: size, repeats: false },
        RepeatStyle::Space => {
            let count = (length / size).floor();
            if count >= 2.0 {
                let step = (length - count * size) / (count - 1.0) + size;
                AxisTiles { origin: start, size, step, repeats: true }
            } else {
                AxisTiles { origin: at_position, size, step: size, repeats: false }
            }
        }
    }
}

/// The range of the tiles of an axis within `paint_start..paint_end`.
fn axis_region(tiles: &AxisTiles, paint_start: f32, paint_end: f32) -> (f32, f32) {
    if tiles.repeats {
        (paint_start, paint_end)
    } else {
        (tiles.origin.max(paint_start), (tiles.origin + tiles.size).min(paint_end))
    }
}

/// The most tiles that an image is repeated into; more would be a page full of one-pixel tiles.
const MAX_TILES: usize = 10_000;

/// The rectangles that an image of the size `natural` is drawn into as a layer of a background:
/// the size of a tile follows `background-size`, and the tiles are those that
/// `background-position` and `background-repeat` put into the positioning area `area` (the
/// padding box) and that touch the painted area `paint` (the border box).
///
/// https://www.w3.org/TR/css-backgrounds-3/#the-background-size
pub fn image_tiles(
    natural: (f32, f32),
    size: &ComputedSize,
    position: &Position<LengthPercentage>,
    repeat: Repeat,
    area: BoxSize,
    paint: BoxSize,
) -> Vec<BoxSize> {
    let (image_width, image_height) = natural;
    if !(image_width > 0.0 && image_height > 0.0 && area.width > 0.0 && area.height > 0.0) {
        return vec![];
    }
    let (width, height) = match size {
        Size::Cover => {
            let scale = (area.width / image_width).max(area.height / image_height);
            (image_width * scale, image_height * scale)
        }
        Size::Contain => {
            let scale = (area.width / image_width).min(area.height / image_height);
            (image_width * scale, image_height * scale)
        }
        Size::Explicit(width, height) => {
            let width = width.map(|w| w.absolute(PixelLength(area.width)).0);
            let height = height.map(|h| h.absolute(PixelLength(area.height)).0);
            match (width, height) {
                (Some(width), Some(height)) => (width, height),
                // `auto` keeps the proportions of the image
                (Some(width), None) => (width, width * image_height / image_width),
                (None, Some(height)) => (height * image_width / image_height, height),
                (None, None) => (image_width, image_height),
            }
        }
    };
    if !(width > 0.0 && height > 0.0) {
        return vec![];
    }
    let mut xs = axis_tiles(repeat.x, width, &position.x, area.x, area.width);
    let mut ys = axis_tiles(repeat.y, height, &position.y, area.y, area.height);
    // With `round` in one axis only, an `auto` size in the other axis follows the rounded size,
    // so that the image keeps its proportions.
    // https://www.w3.org/TR/css-backgrounds-3/#the-background-size
    if let Size::Explicit(auto_width, auto_height) = size {
        if repeat.x == RepeatStyle::Round && repeat.y != RepeatStyle::Round && auto_height.is_none() {
            ys = axis_tiles(repeat.y, xs.size * image_height / image_width, &position.y, area.y, area.height);
        } else if repeat.y == RepeatStyle::Round && repeat.x != RepeatStyle::Round && auto_width.is_none() {
            xs = axis_tiles(repeat.x, ys.size * image_width / image_height, &position.x, area.x, area.width);
        }
    }
    // the starts of the tiles of an axis that touch `from..to`
    let starts = |tiles: &AxisTiles, from: f32, to: f32| -> Vec<f32> {
        if !tiles.repeats {
            return (tiles.origin < to && tiles.origin + tiles.size > from).then_some(tiles.origin).into_iter().collect();
        }
        let mut starts = vec![];
        let mut k = ((from - tiles.origin) / tiles.step).floor();
        loop {
            let start = tiles.origin + k * tiles.step;
            if start >= to || starts.len() > MAX_TILES {
                break;
            }
            if start + tiles.size > from {
                starts.push(start);
            }
            k += 1.0;
        }
        starts
    };
    let columns = starts(&xs, paint.x, paint.x + paint.width);
    let rows = starts(&ys, paint.y, paint.y + paint.height);
    if columns.len() * rows.len() > MAX_TILES {
        log::warn!("A background image is repeated {} times: not painted", columns.len() * rows.len());
        return vec![];
    }
    rows.iter()
        .flat_map(|&y| columns.iter().map(move |&x| BoxSize { x, y, width: xs.size, height: ys.size }))
        .collect()
}

/// Resolves a gradient that is an image of a background: it is laid out in tiles in the
/// positioning area `area` (the padding box) and painted into `paint` (the border box).
pub fn resolve_layer(
    gradient: &ResolvedGradient,
    size: &ComputedSize,
    position: &Position<LengthPercentage>,
    repeat: Repeat,
    area: BoxSize,
    paint: BoxSize,
) -> Option<Resolved> {
    // A gradient has no size of its own, so `auto`, `cover` and `contain` are the size of the area.
    let (width, height) = match size {
        Size::Explicit(w, h) => (
            w.map_or(area.width, |w| w.absolute(PixelLength(area.width)).0),
            h.map_or(area.height, |h| h.absolute(PixelLength(area.height)).0),
        ),
        Size::Cover | Size::Contain => (area.width, area.height),
    };
    if !(width > 0.0 && height > 0.0 && area.width > 0.0 && area.height > 0.0) {
        return None;
    }
    let xs = axis_tiles(repeat.x, width, &position.x, area.x, area.width);
    let ys = axis_tiles(repeat.y, height, &position.y, area.y, area.height);

    let (x0, x1) = axis_region(&xs, paint.x, paint.x + paint.width);
    let (y0, y1) = axis_region(&ys, paint.y, paint.y + paint.height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let region = BoxSize { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
    let tile = BoxSize { x: xs.origin, y: ys.origin, width: xs.size, height: ys.size };

    let eps = 1e-3;
    let covers = tile.x <= paint.x + eps
        && tile.y <= paint.y + eps
        && tile.x + tile.width >= paint.x + paint.width - eps
        && tile.y + tile.height >= paint.y + paint.height - eps;
    if covers {
        return gradient.resolve(tile, paint);
    }
    if !xs.repeats && !ys.repeats {
        let mut resolved = gradient.resolve(tile, region)?;
        resolved.clip = true;
        return Some(resolved);
    }
    let local = BoxSize { x: 0.0, y: 0.0, width: tile.width, height: tile.height };
    let mut resolved = gradient.resolve(local, local)?;
    resolved.tiling = Some(Tiling {
        origin: (tile.x, tile.y),
        size: (tile.width, tile.height),
        step: (xs.step, ys.step),
        region,
    });
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::properties::gradient::{Gradient, GradientKind, LinearDirection, Stop};
    use crate::style::properties::computed::PaintColor;

    fn parse<T: for<'i> PropertyValue<'i, Error = StyleParseError>>(css: &str) -> Option<T> {
        let mut parser = Parser::new(css);
        let value = T::from_css_property_value(&mut parser).ok()?;
        parser.is_exhausted().then_some(value)
    }

    fn pos(css: &str) -> Option<SpecifiedPosition> {
        parse(css)
    }

    fn pct(p: f32) -> Edge {
        Edge::fraction(p)
    }

    fn px(v: f32) -> QuantityOrPercentage {
        use crate::style::properties::specified::{Quantity, Unit};
        QuantityOrPercentage::Quantity(Quantity { value: v, unit: Unit::Px })
    }

    #[test]
    fn positions_with_one_and_two_values() {
        assert_eq!(pos("left").unwrap(), SpecifiedPosition { x: pct(0.0), y: pct(0.5) });
        assert_eq!(pos("bottom").unwrap(), SpecifiedPosition { x: pct(0.5), y: pct(1.0) });
        assert_eq!(pos("center").unwrap(), SpecifiedPosition { x: pct(0.5), y: pct(0.5) });
        assert_eq!(pos("20px").unwrap(), SpecifiedPosition { x: Edge::start(px(20.0)), y: pct(0.5) });
        assert_eq!(pos("50px 0").unwrap().y.offset, px(0.0));
        assert_eq!(pos("top right").unwrap(), SpecifiedPosition { x: pct(1.0), y: pct(0.0) });
        assert_eq!(pos("right 30%").unwrap().x, pct(1.0));
        assert!(pos("left right").is_none());
        assert!(pos("top 10px").is_none());
    }

    #[test]
    fn three_values_can_have_center() {
        let value = pos("left 50px center").unwrap();
        assert_eq!(value, SpecifiedPosition { x: Edge { from_end: false, offset: px(50.0) }, y: pct(0.5) });
        let value = pos("center bottom 20px").unwrap();
        assert_eq!(value, SpecifiedPosition { x: pct(0.5), y: Edge { from_end: true, offset: px(20.0) } });
        let value = pos("top 10px center").unwrap();
        assert_eq!(value, SpecifiedPosition { x: pct(0.5), y: Edge { from_end: false, offset: px(10.0) } });
        assert!(pos("left 10px right").is_none());
    }

    #[test]
    fn positions_with_edge_offsets() {
        let p = pos("right 10px bottom 20px").unwrap();
        assert_eq!(p.x, Edge { from_end: true, offset: px(10.0) });
        assert_eq!(p.y, Edge { from_end: true, offset: px(20.0) });
        let p = pos("bottom 20px left").unwrap();
        assert_eq!(p.x, pct(0.0));
        assert_eq!(p.y, Edge { from_end: true, offset: px(20.0) });
        assert!(pos("left 10px right 5px").is_none());
        assert!(pos("center 10px top").is_none());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse::<SpecifiedSize>("cover"), Some(Size::Cover));
        assert_eq!(parse::<SpecifiedSize>("auto"), Some(Size::Explicit(None, None)));
        assert_eq!(parse::<SpecifiedSize>("20px auto"), Some(Size::Explicit(Some(px(20.0)), None)));
        assert!(parse::<SpecifiedSize>("cover cover").is_none());
    }

    #[test]
    fn repeats() {
        use RepeatStyle::{NoRepeat, Round, Space};
        assert_eq!(parse::<Repeat>("repeat-x"), Some(Repeat { x: RepeatStyle::Repeat, y: NoRepeat }));
        assert_eq!(parse::<Repeat>("space round"), Some(Repeat { x: Space, y: Round }));
        assert_eq!(parse::<Repeat>("no-repeat"), Some(Repeat { x: NoRepeat, y: NoRepeat }));
        assert!(parse::<Repeat>("repeat-x repeat").is_none());
    }

    fn gradient() -> ResolvedGradient {
        Gradient {
            kind: GradientKind::Linear(LinearDirection::default()),
            repeating: false,
            interpolation: Default::default(),
            stops: vec![
                Stop { color: PaintColor::srgb(255, 0, 0, 255), positions: vec![], missing: [false; 3] },
                Stop { color: PaintColor::srgb(0, 0, 255, 255), positions: vec![], missing: [false; 3] },
            ],
        }
    }

    fn lp(v: f32) -> LengthPercentage {
        LengthPercentage::PixelLength(PixelLength(v))
    }

    fn at(x: LengthPercentage, y: LengthPercentage) -> Position<LengthPercentage> {
        Position { x, y }
    }

    const AREA: BoxSize = BoxSize { x: 10.0, y: 20.0, width: 100.0, height: 60.0 };

    fn layer(size: ComputedSize, position: Position<LengthPercentage>, repeat: Repeat) -> Option<Resolved> {
        resolve_layer(&gradient(), &size, &position, repeat, AREA, AREA)
    }

    fn image_tiles_of(
        natural: (f32, f32),
        size: ComputedSize,
        position: Position<LengthPercentage>,
        repeat: Repeat,
    ) -> Vec<BoxSize> {
        image_tiles(natural, &size, &position, repeat, AREA, AREA)
    }

    const NO_REPEAT: Repeat = Repeat { x: RepeatStyle::NoRepeat, y: RepeatStyle::NoRepeat };

    fn tile(x: f32, y: f32, width: f32, height: f32) -> BoxSize {
        BoxSize { x, y, width, height }
    }

    #[test]
    fn an_image_has_its_own_size_unless_told_otherwise() {
        let tiles = image_tiles_of((40.0, 20.0), Size::default(), at(lp(0.0), lp(0.0)), NO_REPEAT);
        assert_eq!(tiles, vec![tile(10.0, 20.0, 40.0, 20.0)]);
    }

    #[test]
    fn auto_keeps_the_proportions_of_the_image() {
        let width = Size::Explicit(Some(lp(50.0)), None);
        assert_eq!(
            image_tiles_of((100.0, 40.0), width, at(lp(0.0), lp(0.0)), NO_REPEAT),
            vec![tile(10.0, 20.0, 50.0, 20.0)]
        );
        let height = Size::Explicit(None, Some(lp(30.0)));
        assert_eq!(
            image_tiles_of((100.0, 40.0), height, at(lp(0.0), lp(0.0)), NO_REPEAT),
            vec![tile(10.0, 20.0, 75.0, 30.0)]
        );
    }

    #[test]
    fn cover_fills_the_area_and_contain_fits_into_it() {
        // 100 x 60 area, image 50 x 50: cover scales by 2 (to 100 x 100), contain by 1.2 (to 60 x 60)
        let cover = image_tiles_of((50.0, 50.0), Size::Cover, at(lp(0.0), lp(0.0)), NO_REPEAT);
        assert_eq!(cover, vec![tile(10.0, 20.0, 100.0, 100.0)]);
        let contain = image_tiles_of((50.0, 50.0), Size::Contain, at(lp(0.0), lp(0.0)), NO_REPEAT);
        // (the scale 1.2 is not exact in a float)
        assert_eq!(contain.len(), 1);
        let (got, want) = (contain[0], tile(10.0, 20.0, 60.0, 60.0));
        assert!(
            (got.x - want.x).abs() < 1e-3
                && (got.y - want.y).abs() < 1e-3
                && (got.width - want.width).abs() < 1e-3
                && (got.height - want.height).abs() < 1e-3,
            "{got:?}"
        );
    }

    #[test]
    fn a_repeated_image_fills_the_painted_area_with_tiles() {
        let size = Size::Explicit(Some(lp(30.0)), Some(lp(20.0)));
        let tiles = image_tiles_of((10.0, 10.0), size, at(lp(0.0), lp(0.0)), Repeat::default());
        // 100 x 60 area from (10, 20): the tiles start at the position and continue in both directions
        assert_eq!(tiles.len(), 4 * 3);
        assert!(tiles.iter().all(|t| t.width == 30.0 && t.height == 20.0));
        assert!(tiles.contains(&tile(10.0, 20.0, 30.0, 20.0)));
        assert!(tiles.contains(&tile(100.0, 60.0, 30.0, 20.0)));
    }

    #[test]
    fn a_tile_of_a_centred_image_is_in_the_middle() {
        let half = LengthPercentage::Percentage(super::Percentage(0.5));
        let tiles = image_tiles_of((40.0, 20.0), Size::default(), at(half, half), NO_REPEAT);
        // (100-40)/2 and (60-20)/2 from the start of the area
        assert_eq!(tiles, vec![tile(40.0, 40.0, 40.0, 20.0)]);
    }

    #[test]
    fn nothing_is_drawn_for_an_image_without_a_size() {
        assert!(image_tiles_of((0.0, 10.0), Size::default(), at(lp(0.0), lp(0.0)), NO_REPEAT).is_empty());
    }

    #[test]
    fn default_layer_covers_the_area_without_tiling() {
        let r = layer(Size::default(), at(lp(0.0), lp(0.0)), Repeat::default()).unwrap();
        assert!(r.tiling.is_none() && !r.clip);
    }

    #[test]
    fn tiles_repeat_from_the_position() {
        let size = Size::Explicit(Some(lp(30.0)), Some(lp(20.0)));
        let r = layer(size, at(lp(5.0), lp(0.0)), Repeat::default()).unwrap();
        let t = r.tiling.unwrap();
        assert_eq!(t.origin, (15.0, 20.0));
        assert_eq!(t.size, (30.0, 20.0));
        assert_eq!(t.step, (30.0, 20.0));
        assert_eq!(t.region, AREA);
        // the gradient is relative to the tile
        assert_eq!(r.paint, BoxSize { x: 0.0, y: 0.0, width: 30.0, height: 20.0 });
    }

    #[test]
    fn percentage_positions_use_the_free_space() {
        let size = Size::Explicit(Some(lp(40.0)), Some(lp(20.0)));
        let half = LengthPercentage::Percentage(super::Percentage(0.5));
        let r = layer(size, at(half, half), Repeat { x: RepeatStyle::NoRepeat, y: RepeatStyle::NoRepeat }).unwrap();
        // one tile, centred: (100-40)/2 and (60-20)/2 from the start of the area
        assert!(r.clip);
        assert_eq!(r.paint, BoxSize { x: 40.0, y: 40.0, width: 40.0, height: 20.0 });
    }

    #[test]
    fn no_repeat_axis_limits_the_region() {
        let size = Size::Explicit(Some(lp(30.0)), Some(lp(20.0)));
        let repeat = Repeat { x: RepeatStyle::Repeat, y: RepeatStyle::NoRepeat };
        let t = layer(size, at(lp(0.0), lp(10.0)), repeat).unwrap().tiling.unwrap();
        assert_eq!(t.region, BoxSize { x: 10.0, y: 30.0, width: 100.0, height: 20.0 });
    }

    #[test]
    fn space_distributes_the_gap() {
        let size = Size::Explicit(Some(lp(30.0)), Some(lp(60.0)));
        let repeat = Repeat { x: RepeatStyle::Space, y: RepeatStyle::Repeat };
        let t = layer(size, at(lp(0.0), lp(0.0)), repeat).unwrap().tiling.unwrap();
        // three tiles of 30 in 100: two gaps of 5
        assert_eq!(t.origin.0, 10.0);
        assert!((t.step.0 - 35.0).abs() < 1e-4);
    }

    #[test]
    fn round_resizes_the_tile() {
        let size = Size::Explicit(Some(lp(30.0)), Some(lp(60.0)));
        let repeat = Repeat { x: RepeatStyle::Round, y: RepeatStyle::Repeat };
        let t = layer(size, at(lp(0.0), lp(0.0)), repeat).unwrap().tiling.unwrap();
        // 100 / 30 rounds to 3 tiles
        assert!((t.size.0 - 100.0 / 3.0).abs() < 1e-4);
    }

    #[test]
    fn empty_tiles_paint_nothing() {
        assert!(layer(Size::Explicit(Some(lp(0.0)), None), at(lp(0.0), lp(0.0)), Repeat::default()).is_none());
    }
}
