//! The `transform` and `transform-origin` properties (2D transforms only).
//! https://www.w3.org/TR/css-transforms-1/

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};

use super::computed::PixelLength;
use crate::style::parse::StyleParseError;
use super::specified::{Percentage, PropertyValue, QuantityOrPercentage};

type ParseResult<T> = Result<T, ParseError<StyleParseError>>;

/// A transform function. Angles are in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformFunction {
    Matrix([f32; 6]),
    Translate(QuantityOrPercentage, QuantityOrPercentage),
    Scale(f32, f32),
    Rotate(f32),
    Skew(f32, f32),
}

/// A list of transform functions; empty for `none`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Transform(pub Vec<TransformFunction>);

fn zero() -> QuantityOrPercentage {
    QuantityOrPercentage::Percentage(Percentage(0.0))
}

fn angle<'i>(input: &mut Parser<'i>) -> ParseResult<f32> {
    let token = input.next()?.clone();
    match token {
        Token::Dimension { value, ref unit, .. } => match_ignore_ascii_case! { unit,
            "deg" => Ok(value),
            "grad" => Ok(value * 0.9),
            "rad" => Ok(value.to_degrees()),
            "turn" => Ok(value * 360.0),
            _ => Err(ParseError::unexpected_token()),
        },
        Token::Number { value, .. } if value == 0.0 => Ok(0.0),
        _ => Err(ParseError::unexpected_token()),
    }
}

fn length<'i>(input: &mut Parser<'i>) -> ParseResult<QuantityOrPercentage> {
    QuantityOrPercentage::from_css_property_value(input)
}

fn function<'i>(input: &mut Parser<'i>) -> ParseResult<TransformFunction> {
    let name = input.expect_function()?.clone();
    input.parse_nested_block(|input| {
        let result = match_ignore_ascii_case! { &name,
            "translate" => {
                let x = length(input)?;
                let y = if input.try_parse(|i| i.expect_comma()).is_ok() { length(input)? } else { zero() };
                TransformFunction::Translate(x, y)
            },
            "translatex" => TransformFunction::Translate(length(input)?, zero()),
            "translatey" => TransformFunction::Translate(zero(), length(input)?),
            "scale" => {
                let x = input.expect_number()?;
                let y = if input.try_parse(|i| i.expect_comma()).is_ok() { input.expect_number()? } else { x };
                TransformFunction::Scale(x, y)
            },
            "scalex" => TransformFunction::Scale(input.expect_number()?, 1.0),
            "scaley" => TransformFunction::Scale(1.0, input.expect_number()?),
            "rotate" => TransformFunction::Rotate(angle(input)?),
            "skew" => {
                let x = angle(input)?;
                let y = if input.try_parse(|i| i.expect_comma()).is_ok() { angle(input)? } else { 0.0 };
                TransformFunction::Skew(x, y)
            },
            "skewx" => TransformFunction::Skew(angle(input)?, 0.0),
            "skewy" => TransformFunction::Skew(0.0, angle(input)?),
            "matrix" => {
                let mut m = [0.0; 6];
                for (i, value) in m.iter_mut().enumerate() {
                    if i > 0 {
                        input.expect_comma()?;
                    }
                    *value = input.expect_number()?;
                }
                TransformFunction::Matrix(m)
            },
            _ => return Err(ParseError::unexpected_token()),
        };
        input.expect_exhausted()?;
        Ok(result)
    })
}

impl<'i> PropertyValue<'i> for Transform {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        input.parse_entirely(|input| {
            if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
                return Ok(Transform(vec![]));
            }
            let mut functions = vec![function(input)?];
            while !input.is_exhausted() {
                functions.push(function(input)?);
            }
            Ok(Transform(functions))
        })
    }
}

/// The origin of the transformation: a point in the border box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformOrigin {
    pub x: QuantityOrPercentage,
    pub y: QuantityOrPercentage,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        TransformOrigin {
            x: QuantityOrPercentage::Percentage(Percentage(0.5)),
            y: QuantityOrPercentage::Percentage(Percentage(0.5)),
        }
    }
}

#[derive(Clone, Copy)]
enum Component {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    Value(QuantityOrPercentage),
}

fn component<'i>(input: &mut Parser<'i>) -> ParseResult<Component> {
    if let Ok(ident) = input.try_parse(|i| i.expect_ident().cloned()) {
        return match_ignore_ascii_case! { &ident,
            "left" => Ok(Component::Left),
            "right" => Ok(Component::Right),
            "top" => Ok(Component::Top),
            "bottom" => Ok(Component::Bottom),
            "center" => Ok(Component::Center),
            _ => Err(ParseError::unexpected_token()),
        };
    }
    Ok(Component::Value(length(input)?))
}

impl<'i> PropertyValue<'i> for TransformOrigin {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> ParseResult<Self> {
        input.parse_entirely(|input| {
            let first = component(input)?;
            let second = input.try_parse(component).ok();
            let pct = |p: f32| QuantityOrPercentage::Percentage(Percentage(p));
            let horizontal = |c: Component| match c {
                Component::Left => Some(pct(0.0)),
                Component::Right => Some(pct(1.0)),
                Component::Center => Some(pct(0.5)),
                Component::Value(v) => Some(v),
                _ => None,
            };
            let vertical = |c: Component| match c {
                Component::Top => Some(pct(0.0)),
                Component::Bottom => Some(pct(1.0)),
                Component::Center => Some(pct(0.5)),
                Component::Value(v) => Some(v),
                _ => None,
            };
            let invalid = || ParseError::unexpected_token();
            let (x, y) = match (first, second) {
                // `top left`: the keywords may come in either order
                (Component::Top | Component::Bottom, Some(second)) => {
                    (horizontal(second).ok_or_else(invalid)?, vertical(first).ok_or_else(invalid)?)
                }
                (Component::Top | Component::Bottom, None) => (pct(0.5), vertical(first).ok_or_else(invalid)?),
                (first, Some(second)) => {
                    (horizontal(first).ok_or_else(invalid)?, vertical(second).ok_or_else(invalid)?)
                }
                (first, None) => (horizontal(first).ok_or_else(invalid)?, pct(0.5)),
            };
            Ok(TransformOrigin { x, y })
        })
    }
}

/// A 2D matrix `[a, b, c, d, e, f]`: `(x, y)` becomes `(a x + c y + e, b x + d y + f)`.
pub type Matrix = [f32; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The matrix `first` and then `second`... in the order of CSS: `m = first * second` applies
/// `second` to a point first.
pub fn multiply(first: Matrix, second: Matrix) -> Matrix {
    let [a1, b1, c1, d1, e1, f1] = first;
    let [a2, b2, c2, d2, e2, f2] = second;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    ]
}

impl Transform {
    /// The matrix of the transformation of a border box of the given position and size, in
    /// pixels, or `None` if there is no transformation. Percentages of `translate` refer to the
    /// size of the box.
    pub fn matrix(
        &self,
        origin: &TransformOrigin,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: super::computed::FontSizes,
    ) -> Option<Matrix> {
        if self.0.is_empty() {
            return None;
        }
        let resolve = |value: QuantityOrPercentage, reference: f32| match value {
            QuantityOrPercentage::Quantity(q) => PixelLength::from_val_unit(q.value, q.unit, font_size).0,
            QuantityOrPercentage::Percentage(p) => p.0 * reference,
        };
        let mut m = IDENTITY;
        for function in &self.0 {
            let f = match *function {
                TransformFunction::Matrix(matrix) => matrix,
                TransformFunction::Translate(tx, ty) => {
                    [1.0, 0.0, 0.0, 1.0, resolve(tx, width), resolve(ty, height)]
                }
                TransformFunction::Scale(sx, sy) => [sx, 0.0, 0.0, sy, 0.0, 0.0],
                TransformFunction::Rotate(degrees) => {
                    let (sin, cos) = degrees.to_radians().sin_cos();
                    [cos, sin, -sin, cos, 0.0, 0.0]
                }
                TransformFunction::Skew(ax, ay) => {
                    [1.0, ay.to_radians().tan(), ax.to_radians().tan(), 1.0, 0.0, 0.0]
                }
            };
            m = multiply(m, f);
        }
        if m.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let (ox, oy) = (x + resolve(origin.x, width), y + resolve(origin.y, height));
        let to_origin = [1.0, 0.0, 0.0, 1.0, ox, oy];
        let from_origin = [1.0, 0.0, 0.0, 1.0, -ox, -oy];
        Some(multiply(multiply(to_origin, m), from_origin))
    }
}

#[cfg(test)]
mod tests {
    use crate::style::properties::computed::FontSizes;
    use super::*;

    fn parse<'i, T: PropertyValue<'i, Error = StyleParseError>>(css: &'i str) -> Option<T> {
        let mut parser = Parser::new(css);
        T::from_css_property_value(&mut parser).ok()
    }

    fn close(a: Matrix, b: Matrix) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn functions_are_parsed() {
        let t: Transform = parse("translate(10px, 5px) rotate(90deg) scale(2)").unwrap();
        assert_eq!(t.0.len(), 3);
        assert_eq!(parse::<Transform>("none").unwrap(), Transform(vec![]));
        assert!(parse::<Transform>("rotate(10)").is_none());
        assert!(parse::<Transform>("wobble(1)").is_none());
        let t: Transform = parse("matrix(1,0,0,1,5,6) skewX(10deg) scaleY(.5)").unwrap();
        assert_eq!(t.0.len(), 3);
    }

    #[test]
    fn the_origin_is_the_center_by_default() {
        let t: Transform = parse("scale(2)").unwrap();
        // a 100 x 50 box at (10, 20) is scaled around (60, 45)
        let m = t.matrix(&TransformOrigin::default(), 10.0, 20.0, 100.0, 50.0, FontSizes { font: PixelLength(16.0), root: PixelLength(16.0) }).unwrap();
        assert!(close(m, [2.0, 0.0, 0.0, 2.0, -60.0, -45.0]));
    }

    #[test]
    fn origins_are_parsed() {
        let o: TransformOrigin = parse("left top").unwrap();
        assert_eq!(o.x, QuantityOrPercentage::Percentage(Percentage(0.0)));
        let o: TransformOrigin = parse("bottom right").unwrap();
        assert_eq!(o.x, QuantityOrPercentage::Percentage(Percentage(1.0)));
        assert_eq!(o.y, QuantityOrPercentage::Percentage(Percentage(1.0)));
        assert!(parse::<TransformOrigin>("left right").is_none());
    }

    #[test]
    fn functions_apply_right_to_left() {
        // translate(10px) then rotate: the point is rotated first, then moved
        let t: Transform = parse("translate(10px, 0) rotate(90deg)").unwrap();
        let origin = TransformOrigin { x: QuantityOrPercentage::Percentage(Percentage(0.0)), y: QuantityOrPercentage::Percentage(Percentage(0.0)) };
        let m = t.matrix(&origin, 0.0, 0.0, 0.0, 0.0, FontSizes { font: PixelLength(16.0), root: PixelLength(16.0) }).unwrap();
        // (1, 0) -> (0, 1) -> (10, 1)
        let [a, b, c, d, e, f] = m;
        assert!(((a * 1.0 + c * 0.0 + e) - 10.0).abs() < 1e-4);
        assert!(((b * 1.0 + d * 0.0 + f) - 1.0).abs() < 1e-4);
    }
}
