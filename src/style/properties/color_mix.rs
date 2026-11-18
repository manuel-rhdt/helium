//! `color-mix()`: https://www.w3.org/TR/css-color-5/#color-mix
//!
//! The mix is computed when the colour is parsed, and the result is a plain sRGB colour. The
//! colours that are mixed must therefore be known then: `currentcolor` is not supported in a
//! mix (the declaration is dropped), and neither are the RGB spaces with a wider gamut than sRGB
//! (`display-p3` and others) as the interpolation space. Missing components (`none`) are not
//! supported either. A colour outside of the sRGB gamut is clipped per channel after the mix.

use super::color::{
    D50_WHITE, Rgb, apply, color_to_paint, d50_to_d65, lab_to_xyz, oklab_to_srgb, srgb_decode, srgb_encode, to_u8,
    xyz_to_srgb,
};
use cssparser::color::PredefinedColorSpace;
use cssparser::{Parser, Token};
use cssparser_color::{Color, RgbaLegacy, hsl_to_rgb, hwb_to_rgb};

/// The colour spaces that colours can be mixed in.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum Space {
    #[default]
    Srgb,
    SrgbLinear,
    Hsl,
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    XyzD65,
    XyzD50,
}

impl Space {
    fn parse(name: &str) -> Option<Space> {
        Some(match name.to_ascii_lowercase().as_str() {
            "srgb" => Space::Srgb,
            "srgb-linear" => Space::SrgbLinear,
            "hsl" => Space::Hsl,
            "hwb" => Space::Hwb,
            "lab" => Space::Lab,
            "lch" => Space::Lch,
            "oklab" => Space::Oklab,
            "oklch" => Space::Oklch,
            "xyz" | "xyz-d65" => Space::XyzD65,
            "xyz-d50" => Space::XyzD50,
            _ => return None,
        })
    }

    /// The index of the hue among the components, if the space is a polar one.
    fn hue(self) -> Option<usize> {
        match self {
            Space::Hsl | Space::Hwb => Some(0),
            Space::Lch | Space::Oklch => Some(2),
            _ => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum HueMethod {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// The weights of the two colours: the shares that sum up to one, and the factor that the alpha
/// is multiplied by if the percentages summed up to less than 100%.
fn weights(first: Option<f32>, second: Option<f32>) -> Option<(f32, f32, f32)> {
    let (p1, p2) = match (first, second) {
        (None, None) => (0.5, 0.5),
        (Some(p), None) => (p, 1.0 - p),
        (None, Some(p)) => (1.0 - p, p),
        (Some(p1), Some(p2)) => (p1, p2),
    };
    if !(0.0..=1.0).contains(&p1) || !(0.0..=1.0).contains(&p2) {
        return None;
    }
    let sum = p1 + p2;
    if sum <= 0.0 {
        return None;
    }
    Some((p1 / sum, p2 / sum, sum.min(1.0)))
}

/// How colours are interpolated: in a colour space, and for a polar one along an arc of hues.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct Interpolation {
    pub space: Space,
    pub method: HueMethod,
}

/// Parses `in <space> [<method> hue]?`, the colour interpolation method of CSS Color 4.
pub fn parse_interpolation<'i>(input: &mut Parser<'i>) -> Result<Interpolation, ()> {
    input.expect_ident_matching("in").map_err(|_| ())?;
    let space = Space::parse(input.expect_ident().map_err(|_| ())?).ok_or(())?;
    let mut method = HueMethod::Shorter;
    if let Ok(found) = input.try_parse(|input| -> Result<HueMethod, ()> {
        let name = input.expect_ident().map_err(|_| ())?.to_ascii_lowercase();
        let method = match name.as_str() {
            "shorter" => HueMethod::Shorter,
            "longer" => HueMethod::Longer,
            "increasing" => HueMethod::Increasing,
            "decreasing" => HueMethod::Decreasing,
            _ => return Err(()),
        };
        input.expect_ident_matching("hue").map_err(|_| ())?;
        Ok(method)
    }) {
        // (only polar spaces have a hue)
        if space.hue().is_none() {
            return Err(());
        }
        method = found;
    }
    Ok(Interpolation { space, method })
}

/// Parses what follows `color-mix(` up to the closing parenthesis, with `parse_color` for the
/// colours (so that a mix can be one of them).
fn parse_arguments<'i>(
    input: &mut Parser<'i>,
    parse_color: &mut dyn FnMut(&mut Parser<'i>) -> Result<Color, ()>,
) -> Result<Color, ()> {
    let Interpolation { space, method } = parse_interpolation(input)?;
    input.expect_comma().map_err(|_| ())?;
    let mut item = |input: &mut Parser<'i>| -> Result<(Color, Option<f32>), ()> {
        let percentage = |input: &mut Parser<'i>| -> Result<f32, ()> {
            match input.next().map_err(|_| ())? {
                Token::Percentage { unit_value, .. } => Ok(*unit_value),
                _ => Err(()),
            }
        };
        let before = input.try_parse(percentage).ok();
        let color = parse_color(input)?;
        let after = input.try_parse(percentage).ok();
        if before.is_some() && after.is_some() {
            return Err(());
        }
        Ok((color, before.or(after)))
    };
    let (first, first_share) = item(input)?;
    input.expect_comma().map_err(|_| ())?;
    let (second, second_share) = item(input)?;
    if !input.is_exhausted() {
        return Err(());
    }
    let (w1, w2, alpha_scale) = weights(first_share, second_share).ok_or(())?;
    mix(space, method, first, second, w1, w2, alpha_scale)
}

/// Parses `color-mix(...)` if `input` is one, with `parse_color` for the colours in it.
pub fn parse<'i>(
    input: &mut Parser<'i>,
    parse_color: &mut dyn FnMut(&mut Parser<'i>) -> Result<Color, ()>,
) -> Result<Color, ()> {
    input.expect_function_matching("color-mix").map_err(|_| ())?;
    input.parse_nested_block(|input| parse_arguments(input, parse_color).map_err(|()| cssparser::ParseError::<()>::custom(())))
        .map_err(|_| ())
}

fn cbrt(x: f32) -> f32 {
    x.cbrt()
}

/// The components of an sRGB colour (encoded, 0 to 1) in a colour space. The hue is in degrees; a
/// hue that is powerless (a grey) is `None`.
fn to_space(space: Space, [r, g, b]: Rgb) -> ([f32; 3], Option<f32>) {
    let linear = [srgb_decode(r), srgb_decode(g), srgb_decode(b)];
    const EPSILON: f32 = 1e-4;
    let polar = |a: f32, b: f32, chroma_floor: f32| {
        let chroma = a.hypot(b);
        let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
        (chroma, (chroma > chroma_floor).then_some(hue))
    };
    match space {
        Space::Srgb => ([r, g, b], None),
        Space::SrgbLinear => (linear, None),
        Space::Hsl | Space::Hwb => {
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            let hue = if delta < EPSILON {
                None
            } else {
                let h = if max == r {
                    ((g - b) / delta).rem_euclid(6.0)
                } else if max == g {
                    (b - r) / delta + 2.0
                } else {
                    (r - g) / delta + 4.0
                };
                Some(h * 60.0)
            };
            if space == Space::Hsl {
                let lightness = (max + min) / 2.0;
                let saturation = if delta < EPSILON { 0.0 } else { delta / (1.0 - (2.0 * lightness - 1.0).abs()) };
                ([hue.unwrap_or(0.0), saturation, lightness], hue)
            } else {
                // (a colour with no whiteness and no blackness would have a hue)
                ([hue.unwrap_or(0.0), min, 1.0 - max], hue)
            }
        }
        Space::XyzD65 | Space::XyzD50 | Space::Lab | Space::Lch => {
            let xyz65 = apply(&SRGB_TO_XYZ, linear);
            if space == Space::XyzD65 {
                return (xyz65, None);
            }
            let xyz50 = apply(&D65_TO_D50, xyz65);
            if space == Space::XyzD50 {
                return (xyz50, None);
            }
            const KAPPA: f32 = 24389.0 / 27.0;
            const EPS: f32 = 216.0 / 24389.0;
            let f = |t: f32| if t > EPS { cbrt(t) } else { (KAPPA * t + 16.0) / 116.0 };
            let (fx, fy, fz) = (f(xyz50[0] / D50_WHITE[0]), f(xyz50[1] / D50_WHITE[1]), f(xyz50[2] / D50_WHITE[2]));
            let (l, a, b) = (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz));
            if space == Space::Lab {
                ([l, a, b], None)
            } else {
                let (chroma, hue) = polar(a, b, 0.02);
                ([l, chroma, hue.unwrap_or(0.0)], hue)
            }
        }
        Space::Oklab | Space::Oklch => {
            let l = cbrt(0.4122214708 * linear[0] + 0.5363325363 * linear[1] + 0.0514459929 * linear[2]);
            let m = cbrt(0.2119034982 * linear[0] + 0.6806995451 * linear[1] + 0.1073969566 * linear[2]);
            let s = cbrt(0.0883024619 * linear[0] + 0.2817188376 * linear[1] + 0.6299787005 * linear[2]);
            let lightness = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
            let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
            let b = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
            if space == Space::Oklab {
                ([lightness, a, b], None)
            } else {
                let (chroma, hue) = polar(a, b, 2e-4);
                ([lightness, chroma, hue.unwrap_or(0.0)], hue)
            }
        }
    }
}

/// The sRGB colour (encoded) of the components of a colour space.
fn from_space(space: Space, [x, y, z]: [f32; 3]) -> Rgb {
    let encode = |linear: Rgb| [srgb_encode(linear[0]), srgb_encode(linear[1]), srgb_encode(linear[2])];
    let from_xyz65 = |xyz: Rgb| encode(xyz_to_srgb(xyz));
    match space {
        Space::Srgb => [x, y, z],
        Space::SrgbLinear => encode([x, y, z]),
        Space::Hsl => {
            let (r, g, b) = hsl_to_rgb(x.rem_euclid(360.0) / 360.0, y.clamp(0.0, 1.0), z.clamp(0.0, 1.0));
            [r, g, b]
        }
        Space::Hwb => {
            let (r, g, b) = hwb_to_rgb(x.rem_euclid(360.0) / 360.0, y.max(0.0), z.max(0.0));
            [r, g, b]
        }
        Space::XyzD65 => from_xyz65([x, y, z]),
        Space::XyzD50 => from_xyz65(d50_to_d65([x, y, z])),
        Space::Lab => from_xyz65(d50_to_d65(lab_to_xyz(x, y, z))),
        Space::Lch => {
            let hue = z.to_radians();
            from_xyz65(d50_to_d65(lab_to_xyz(x, y * hue.cos(), y * hue.sin())))
        }
        Space::Oklab => encode(oklab_to_srgb(x, y, z)),
        Space::Oklch => {
            let hue = z.to_radians();
            encode(oklab_to_srgb(x, y * hue.cos(), y * hue.sin()))
        }
    }
}

#[rustfmt::skip]
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.41239080, 0.35758434, 0.18048079],
    [0.21263901, 0.71516868, 0.07219232],
    [0.01933082, 0.11919478, 0.95053215],
];

// Bradford adaptation from the D65 white to the D50 white.
#[rustfmt::skip]
const D65_TO_D50: [[f32; 3]; 3] = [
    [ 1.04792979,  0.02294687, -0.05019227],
    [ 0.02962781,  0.99043442, -0.01707382],
    [-0.00924304,  0.01505519,  0.75187428],
];

/// The hue between two hues, as the two numbers to interpolate between.
fn arc(method: HueMethod, mut first: f32, mut second: f32) -> (f32, f32) {
    first = first.rem_euclid(360.0);
    second = second.rem_euclid(360.0);
    // (the same hue, which goes through the conversion to a colour and back, is the same up to rounding)
    let delta = second - first;
    let delta = if delta.abs() < 1e-3 { 0.0 } else { delta };
    match method {
        HueMethod::Shorter => {
            if delta > 180.0 {
                first += 360.0;
            } else if delta < -180.0 {
                second += 360.0;
            }
        }
        HueMethod::Longer => {
            if 0.0 < delta && delta < 180.0 {
                first += 360.0;
            } else if -180.0 < delta && delta <= 0.0 {
                second += 360.0;
            }
        }
        HueMethod::Increasing => {
            if second < first {
                second += 360.0;
            }
        }
        HueMethod::Decreasing => {
            if first < second {
                first += 360.0;
            }
        }
    }
    (first, second)
}

/// The mix of two colours in `space`, premultiplied with the alpha (which is what CSS Color 5
/// asks for), with the weights `w1` and `w2`.
fn mix(space: Space, method: HueMethod, first: Color, second: Color, w1: f32, w2: f32, alpha_scale: f32) -> Result<Color, ()> {
    if matches!(first, Color::CurrentColor) || matches!(second, Color::CurrentColor) {
        return Err(());
    }
    // The colours are mixed in the space of the mix, from the channels they have: those of a colour
    // outside of sRGB are outside of 0 to 1 and nothing is clipped.
    let components = |c: Color| super::computed::PaintColor::to_extended_srgb(color_to_paint(c));
    let [r, g, b, alpha] = blend(space, method, components(first), components(second), w1, w2);
    let alpha = (alpha * alpha_scale).clamp(0.0, 1.0);
    if [r, g, b].iter().all(|channel| (-0.001..=1.001).contains(channel)) {
        Ok(Color::Rgba(RgbaLegacy::new(to_u8(r), to_u8(g), to_u8(b), alpha)))
    } else {
        // a colour that sRGB cannot hold: its channels as a `color(srgb ...)`, which keeps them
        Ok(Color::ColorFunction(cssparser_color::ColorFunction::new(
            PredefinedColorSpace::Srgb,
            Some(r),
            Some(g),
            Some(b),
            Some(alpha),
        )))
    }
}

/// The kinds of components that carry over between colour spaces when one is missing (CSS Color 4, 12.2:
/// the analogous components).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Kind {
    Red,
    Green,
    Blue,
    X,
    Y,
    Z,
    Lightness,
    A,
    B,
    /// chroma and saturation
    Colorfulness,
    Hue,
    Whiteness,
    Blackness,
}

impl Space {
    fn kinds(self) -> [Kind; 3] {
        use Kind::*;
        match self {
            Space::Srgb | Space::SrgbLinear => [Red, Green, Blue],
            Space::XyzD65 | Space::XyzD50 => [X, Y, Z],
            Space::Hsl => [Hue, Colorfulness, Lightness],
            Space::Hwb => [Hue, Whiteness, Blackness],
            Space::Lab | Space::Oklab => [Lightness, A, B],
            Space::Lch | Space::Oklch => [Lightness, Colorfulness, Hue],
        }
    }
}

/// Which of the components of `space` are missing (`none`) in `color`, because the colour has a component
/// that is analogous to them and that one is missing. A missing component that is not analogous to
/// one of the space is 0 there, and so it is not missing.
pub fn missing_components(space: Space, color: &Color) -> [bool; 3] {
    use Kind::*;
    let (kinds, missing): ([Kind; 3], [bool; 3]) = match color {
        Color::Hsl(c) => ([Hue, Colorfulness, Lightness], [c.hue.is_none(), c.saturation.is_none(), c.lightness.is_none()]),
        Color::Hwb(c) => ([Hue, Whiteness, Blackness], [c.hue.is_none(), c.whiteness.is_none(), c.blackness.is_none()]),
        Color::Lab(c) => ([Lightness, A, B], [c.lightness.is_none(), c.a.is_none(), c.b.is_none()]),
        Color::Oklab(c) => ([Lightness, A, B], [c.lightness.is_none(), c.a.is_none(), c.b.is_none()]),
        Color::Lch(c) => ([Lightness, Colorfulness, Hue], [c.lightness.is_none(), c.chroma.is_none(), c.hue.is_none()]),
        Color::Oklch(c) => ([Lightness, Colorfulness, Hue], [c.lightness.is_none(), c.chroma.is_none(), c.hue.is_none()]),
        Color::ColorFunction(c) => {
            let kinds = match c.color_space {
                PredefinedColorSpace::XyzD50 | PredefinedColorSpace::XyzD65 => [X, Y, Z],
                _ => [Red, Green, Blue],
            };
            (kinds, [c.c1.is_none(), c.c2.is_none(), c.c3.is_none()])
        }
        _ => return [false; 3],
    };
    let wanted = space.kinds();
    let mut result = [false; 3];
    for (index, kind) in wanted.iter().enumerate() {
        result[index] = kinds.iter().zip(missing).any(|(k, m)| m && k == kind);
    }
    result
}

/// The stops of a gradient (with the index of the stop that each comes from) with the components that are missing in some of them: each stop that has
/// missing components becomes two stops at the same place, one with the values of the stop before for
/// the missing components and one with the values of the stop after (CSS Color 4, 12.3: the transitions on
/// both sides are interpolated as if the component had the value of the neighbour). Where the neighbour has
/// none either, the value is 0. The colours are sRGB with alpha, the result too.
pub fn fill_missing(how: Interpolation, stops: &[([f32; 4], [bool; 3])]) -> Vec<(usize, [f32; 4])> {
    let components = |color: [f32; 4]| to_space(how.space, [color[0], color[1], color[2]]).0;
    let hue = how.space.hue();
    let rebuilt = |values: [f32; 3], alpha: f32| {
        let [r, g, b] = from_space(how.space, values);
        [r, g, b, alpha]
    };
    let mut result = Vec::with_capacity(stops.len());
    for (index, &(color, missing)) in stops.iter().enumerate() {
        if !missing.iter().any(|&m| m) {
            result.push((index, color));
            continue;
        }
        let own = components(color);
        let from = |neighbour: Option<&([f32; 4], [bool; 3])>| {
            let mut values = own;
            for component in 0..3 {
                if missing[component] {
                    values[component] = match neighbour {
                        Some((other, other_missing)) if !other_missing[component] => {
                            let (values, hue_of) = to_space(how.space, [other[0], other[1], other[2]]);
                            hue.filter(|&h| h == component).and_then(|_| hue_of).unwrap_or(values[component])
                        }
                        // the stop is the first or the last one, or the neighbour misses it too
                        Some(_) => 0.0,
                        None => own[component],
                    };
                }
            }
            rebuilt(values, color[3])
        };
        let before = index.checked_sub(1).map(|i| &stops[i]);
        let after = stops.get(index + 1);
        // (the first stop has no transition before it, nor does the last one after it)
        result.push((index, if before.is_some() { from(before) } else { from(after) }));
        result.push((index, if after.is_some() { from(after) } else { from(before) }));
    }
    result
}

/// The point at `t` (0 to 1) on the way from `first` to `second` (sRGB, 0 to 1 and beyond for colours that sRGB cannot hold, with alpha as the
/// fourth component) when the colours are interpolated as `how` says.
pub fn interpolate(how: Interpolation, first: [f32; 4], second: [f32; 4], t: f32) -> [f32; 4] {
    blend(how.space, how.method, first, second, 1.0 - t, t)
}

/// The weighted mean of two colours (sRGB with alpha) in a colour space, with premultiplied alpha.
fn blend(space: Space, method: HueMethod, first: [f32; 4], second: [f32; 4], w1: f32, w2: f32) -> [f32; 4] {
    let (a1, a2) = (first[3], second[3]);
    let (mut v1, mut h1) = to_space(space, [first[0], first[1], first[2]]);
    let (mut v2, mut h2) = to_space(space, [second[0], second[1], second[2]]);
    let hue_index = space.hue();
    // a hue that is powerless is the one of the other colour
    if hue_index.is_some() {
        match (h1, h2) {
            (None, Some(h)) => h1 = Some(h),
            (Some(h), None) => h2 = Some(h),
            _ => {}
        }
    }
    let alpha = a1 * w1 + a2 * w2;
    let mut out = [0.0_f32; 3];
    for index in 0..3 {
        if Some(index) == hue_index {
            let (first, second) = arc(method, h1.unwrap_or(v1[index]), h2.unwrap_or(v2[index]));
            out[index] = first * w1 + second * w2;
        } else {
            v1[index] *= a1;
            v2[index] *= a2;
            let mixed = v1[index] * w1 + v2[index] * w2;
            out[index] = if alpha > 0.0 { mixed / alpha } else { mixed };
        }
    }
    let [r, g, b] = from_space(space, out);
    [r, g, b, alpha.clamp(0.0, 1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed(css: &str) -> Option<[u8; 4]> {
        let mut parser = Parser::new(css);
        fn color<'i>(input: &mut Parser<'i>) -> Result<Color, ()> {
            let state = input.state();
            match Color::parse(input) {
                Ok(color) => Ok(color),
                Err(_) => {
                    input.reset(&state);
                    parse(input, &mut color)
                }
            }
        }
        let color = parse(&mut parser, &mut color).ok()?;
        let rgba = color_to_paint(color).clip_to_srgb();
        Some([rgba.red, rgba.green, rgba.blue, rgba.alpha])
    }

    fn close(found: Option<[u8; 4]>, expected: [u8; 4]) -> bool {
        found.is_some_and(|found| found.iter().zip(expected).all(|(&a, b)| (i32::from(a) - i32::from(b)).abs() <= 2))
    }

    #[test]
    fn an_even_mix_in_srgb_is_the_mean() {
        assert!(close(mixed("color-mix(in srgb, red, blue)"), [128, 0, 128, 255]));
        assert!(close(mixed("color-mix(in srgb, white 25%, black)"), [64, 64, 64, 255]));
        assert!(close(mixed("color-mix(in srgb, white, black 75%)"), [64, 64, 64, 255]));
    }

    #[test]
    fn percentages_that_sum_to_less_than_100_make_the_colour_transparent() {
        assert!(close(mixed("color-mix(in srgb, red 20%, blue 40%)"), [85, 0, 170, 153]));
        // above 100% they are scaled
        assert!(close(mixed("color-mix(in srgb, red 100%, blue 100%)"), [128, 0, 128, 255]));
    }

    #[test]
    fn the_alpha_is_premultiplied() {
        // the transparent colour has no weight in the colour
        assert!(close(mixed("color-mix(in srgb, red, transparent)"), [255, 0, 0, 128]));
    }

    #[test]
    fn known_results_in_other_spaces() {
        // https://www.w3.org/TR/css-color-5/#color-mix examples (rounded)
        assert!(close(mixed("color-mix(in oklab, white, black)"), [99, 99, 99, 255]));
        assert!(close(mixed("color-mix(in srgb-linear, white, black)"), [188, 188, 188, 255]));
        assert!(close(mixed("color-mix(in lab, white, black)"), [119, 119, 119, 255]));
        // hue interpolation takes the shorter arc: red and yellow give orange
        assert!(close(mixed("color-mix(in hsl, red, yellow)"), [255, 128, 0, 255]));
        // the longer one goes through green and blue
        let longer = mixed("color-mix(in hsl longer hue, red, yellow)").unwrap();
        assert!(longer[2] > longer[0], "{longer:?}");
    }

    #[test]
    fn a_mix_can_be_nested() {
        assert!(close(
            mixed("color-mix(in srgb, color-mix(in srgb, red, blue), white)"),
            [191, 128, 191, 255]
        ));
    }

    #[test]
    fn what_is_not_supported_is_an_error() {
        assert!(mixed("color-mix(in srgb, currentcolor, white)").is_none());
        assert!(mixed("color-mix(in display-p3, red, blue)").is_none());
        assert!(mixed("color-mix(in srgb longer hue, red, blue)").is_none());
        assert!(mixed("color-mix(red, blue)").is_none());
        assert!(mixed("color-mix(in srgb, red 0%, blue 0%)").is_none());
    }

    #[test]
    fn a_missing_component_takes_the_value_of_each_neighbour() {
        let srgb = Interpolation::default();
        let color = |css: &str| Color::parse(&mut Parser::new(css)).unwrap();
        let stop = |css: &str| {
            let c = color(css);
            (super::super::computed::PaintColor::to_extended_srgb(color_to_paint(c)), missing_components(srgb.space, &c))
        };
        let filled = fill_missing(srgb, &[stop("color(srgb 0.5 0 0)"), stop("color(srgb none 0.5 0.5)"), stop("color(srgb 1 1 1)")]);
        let reds: Vec<f32> = filled.iter().map(|(_, c)| c[0]).collect();
        // the first red is the one of the stop before and the second the one of the stop after
        assert_eq!(filled.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 1, 2]);
        assert!((reds[1] - 0.5).abs() < 1e-2 && (reds[2] - 1.0).abs() < 1e-2, "{reds:?}");
        // a missing component of a colour that has no analogous one in the space is 0, not missing
        assert_eq!(missing_components(Space::Srgb, &color("hsl(60 none 50%)")), [false; 3]);
        assert_eq!(missing_components(Space::Oklch, &color("lch(50 none 30)")), [false, true, false]);
        assert_eq!(missing_components(Space::Oklch, &color("oklab(none 0.1 0.1)")), [true, false, false]);
    }
}
