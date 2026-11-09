//! The sRGB colour that a CSS colour is painted with.
//!
//! Every colour space is converted to linear-light sRGB with the matrices of CSS Color 4 (§ 12,
//! "Sample code for color conversions") and gamma encoded once at the end, so the result is what
//! a browser would paint. Colours outside the sRGB gamut are clipped per channel.

use cssparser::color::PredefinedColorSpace;
use cssparser_color::{Color, hsl_to_rgb, hwb_to_rgb};

use super::computed::RGBA;

/// Linear-light sRGB components.
type Rgb = [f32; 3];

// CIE XYZ (D65) → linear-light sRGB. https://drafts.csswg.org/css-color-4/#color-conversion-code
#[rustfmt::skip]
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [ 3.24096994, -1.53738318, -0.49861076],
    [-0.96924364,  1.87596750,  0.04155506],
    [ 0.05563008, -0.20397696,  1.05697151],
];

#[rustfmt::skip]
const P3_TO_XYZ: [[f32; 3]; 3] = [
    [0.48657095, 0.26566769, 0.19821729],
    [0.22897456, 0.69173852, 0.07928691],
    [0.00000000, 0.04511338, 1.04394437],
];

#[rustfmt::skip]
const A98_TO_XYZ: [[f32; 3]; 3] = [
    [0.57666904, 0.18555824, 0.18822865],
    [0.29734498, 0.62736357, 0.07529146],
    [0.02703136, 0.07068885, 0.99133754],
];

#[rustfmt::skip]
const PROPHOTO_TO_XYZ_D50: [[f32; 3]; 3] = [
    [0.79776664, 0.13518130, 0.03134773],
    [0.28807483, 0.71183523, 0.00008994],
    [0.00000000, 0.00000000, 0.82510460],
];

#[rustfmt::skip]
const REC2020_TO_XYZ: [[f32; 3]; 3] = [
    [0.63695805, 0.14461690, 0.16888098],
    [0.26270021, 0.67799807, 0.05930172],
    [0.00000000, 0.02807269, 1.06098506],
];

// The D50 white point of CIELAB and ProPhoto RGB.
const D50_WHITE: Rgb = [0.9642956764295677, 1.0, 0.8251046025104602];

// Bradford chromatic adaptation from the D50 white of CIELAB and ProPhoto to D65.
#[rustfmt::skip]
const D50_TO_D65: [[f32; 3]; 3] = [
    [ 0.95547342, -0.02309845,  0.06325924],
    [-0.02836971,  1.00999540,  0.02104144],
    [ 0.01231401, -0.02050765,  1.33036593],
];

fn apply(matrix: &[[f32; 3]; 3], v: Rgb) -> Rgb {
    [
        matrix[0][0] * v[0] + matrix[0][1] * v[1] + matrix[0][2] * v[2],
        matrix[1][0] * v[0] + matrix[1][1] * v[1] + matrix[1][2] * v[2],
        matrix[2][0] * v[0] + matrix[2][1] * v[1] + matrix[2][2] * v[2],
    ]
}

fn xyz_to_srgb(xyz: Rgb) -> Rgb {
    apply(&XYZ_TO_SRGB, xyz)
}

fn d50_to_d65(xyz: Rgb) -> Rgb {
    apply(&D50_TO_D65, xyz)
}

/// The sRGB transfer function of https://drafts.csswg.org/css-color-4/#color-conversion-code.
fn srgb_encode(linear: f32) -> f32 {
    let sign = linear.signum();
    let abs = linear.abs();
    if abs > 0.0031308 {
        sign * (1.055 * abs.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * linear
    }
}

fn srgb_decode(encoded: f32) -> f32 {
    let sign = encoded.signum();
    let abs = encoded.abs();
    if abs <= 0.04045 {
        encoded / 12.92
    } else {
        sign * ((abs + 0.055) / 1.055).powf(2.4)
    }
}

/// The `a98-rgb` transfer function: a pure 563/256 power curve.
fn a98_decode(encoded: f32) -> f32 {
    encoded.abs().powf(563.0 / 256.0) * encoded.signum()
}

/// The ProPhoto RGB transfer function: a gamma curve with a linear segment near black.
fn prophoto_decode(encoded: f32) -> f32 {
    const ET: f32 = 1.0 / 512.0;
    if encoded.abs() <= 16.0 * ET {
        encoded / 16.0
    } else {
        encoded.signum() * encoded.abs().powf(1.8)
    }
}

/// The `rec2020` transfer function.
fn rec2020_decode(encoded: f32) -> f32 {
    const ALPHA: f32 = 1.099_296_8;
    const BETA: f32 = 0.018_053_969;
    let abs = encoded.abs();
    if abs < BETA * 4.5 {
        encoded / 4.5
    } else {
        encoded.signum() * ((abs + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
    }
}

/// CIELAB (D50) to XYZ, https://drafts.csswg.org/css-color-4/#lab-to-xyz.
fn lab_to_xyz(lightness: f32, a: f32, b: f32) -> Rgb {
    const KAPPA: f32 = 24389.0 / 27.0;
    const EPSILON: f32 = 216.0 / 24389.0;

    let f1 = (lightness + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;

    let cube = |f: f32| if f * f * f > EPSILON { f * f * f } else { (116.0 * f - 16.0) / KAPPA };
    let y = if lightness > KAPPA * EPSILON { f1 * f1 * f1 } else { lightness / KAPPA };

    // the formula yields XYZ relative to the D50 white point
    [
        cube(f0) * D50_WHITE[0],
        y * D50_WHITE[1],
        cube(f2) * D50_WHITE[2],
    ]
}

/// Oklab to linear-light sRGB, https://bottosson.github.io/posts/oklab/, as in CSS Color 4.
fn oklab_to_srgb(lightness: f32, a: f32, b: f32) -> Rgb {
    let l = lightness + 0.3963377774 * a + 0.2158037573 * b;
    let m = lightness - 0.1055613458 * a - 0.0638541728 * b;
    let s = lightness - 0.0894841775 * a - 1.2914855480 * b;

    let (l, m, s) = (l * l * l, m * m * m, s * s * s);

    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
}

fn to_u8(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn from_linear(linear: Rgb, alpha: Option<f32>) -> RGBA {
    let encode = |v: f32| to_u8(srgb_encode(v));
    RGBA::new(
        encode(linear[0]),
        encode(linear[1]),
        encode(linear[2]),
        to_u8(alpha.unwrap_or(1.0)),
    )
}

/// The sRGB colour a CSS colour is painted with.  Colors that have no sRGB representation
/// (`currentcolor` outside a cascade) resolve to transparent.
pub fn color_to_rgba(color: Color) -> RGBA {
    match color {
        Color::CurrentColor => RGBA::transparent(),
        // the legacy syntax is sRGB already, so the channels pass through unchanged
        Color::Rgba(c) => RGBA::new(c.red, c.green, c.blue, to_u8(c.alpha)),
        Color::Hsl(c) => {
            let rgb = hsl_to_rgb(
                (c.hue.unwrap_or(0.0) / 360.0).rem_euclid(1.0),
                c.saturation.unwrap_or(0.0),
                c.lightness.unwrap_or(0.0),
            );
            RGBA::new(to_u8(rgb.0), to_u8(rgb.1), to_u8(rgb.2), to_u8(c.alpha.unwrap_or(1.0)))
        }
        Color::Hwb(c) => {
            let rgb = hwb_to_rgb(
                (c.hue.unwrap_or(0.0) / 360.0).rem_euclid(1.0),
                c.whiteness.unwrap_or(0.0),
                c.blackness.unwrap_or(0.0),
            );
            RGBA::new(to_u8(rgb.0), to_u8(rgb.1), to_u8(rgb.2), to_u8(c.alpha.unwrap_or(1.0)))
        }
        Color::Lab(c) => from_linear(
            xyz_to_srgb(d50_to_d65(lab_to_xyz(
                c.lightness.unwrap_or(0.0),
                c.a.unwrap_or(0.0),
                c.b.unwrap_or(0.0),
            ))),
            c.alpha,
        ),
        Color::Lch(c) => {
            let (a, b) = polar_to_rectangular(c.chroma.unwrap_or(0.0), c.hue.unwrap_or(0.0));
            from_linear(
                xyz_to_srgb(d50_to_d65(lab_to_xyz(
                    c.lightness.unwrap_or(0.0),
                    a,
                    b,
                ))),
                c.alpha,
            )
        }
        Color::Oklab(c) => from_linear(
            oklab_to_srgb(
                c.lightness.unwrap_or(0.0),
                c.a.unwrap_or(0.0),
                c.b.unwrap_or(0.0),
            ),
            c.alpha,
        ),
        Color::Oklch(c) => {
            let (a, b) = polar_to_rectangular(c.chroma.unwrap_or(0.0), c.hue.unwrap_or(0.0));
            from_linear(
                oklab_to_srgb(c.lightness.unwrap_or(0.0), a, b),
                c.alpha,
            )
        }
        Color::ColorFunction(c) => {
            use PredefinedColorSpace::*;
            let components = [
                c.c1.unwrap_or(0.0),
                c.c2.unwrap_or(0.0),
                c.c3.unwrap_or(0.0),
            ];
            let linear = match c.color_space {
                // `srgb` is gamma encoded, `srgb-linear` is not
                Srgb => components.map(srgb_decode),
                SrgbLinear => components,
                DisplayP3 => xyz_to_srgb(apply(&P3_TO_XYZ, components.map(srgb_decode))),
                DisplayP3Linear => xyz_to_srgb(apply(&P3_TO_XYZ, components)),
                A98Rgb => xyz_to_srgb(apply(&A98_TO_XYZ, components.map(a98_decode))),
                ProphotoRgb => xyz_to_srgb(d50_to_d65(apply(
                    &PROPHOTO_TO_XYZ_D50,
                    components.map(prophoto_decode),
                ))),
                Rec2020 => xyz_to_srgb(apply(&REC2020_TO_XYZ, components.map(rec2020_decode))),
                XyzD50 => xyz_to_srgb(d50_to_d65(components)),
                XyzD65 => xyz_to_srgb(components),
            };
            from_linear(linear, c.alpha)
        }
    }
}

/// Chroma and hue (in degrees) to the `a` and `b` axes.
fn polar_to_rectangular(chroma: f32, hue: f32) -> (f32, f32) {
    let hue = hue.to_radians();
    (chroma * hue.cos(), chroma * hue.sin())
}

#[cfg(test)]
mod test {
    use super::*;

    fn rgba(color: Color) -> [u8; 4] {
        let c = color_to_rgba(color);
        [c.red, c.green, c.blue, c.alpha]
    }

    fn close(actual: [u8; 4], expected: [u8; 4]) -> bool {
        actual
            .iter()
            .zip(expected.iter())
            .all(|(a, b)| (*a as i32 - *b as i32).abs() <= 2)
    }

    #[test]
    fn oklab_reaches_the_srgb_corners() {
        assert!(close(
            rgba(Color::Oklab(cssparser_color::Oklab::new(Some(1.0), Some(0.0), Some(0.0), None))),
            [255, 255, 255, 255]
        ));
        assert!(close(
            rgba(Color::Oklab(cssparser_color::Oklab::new(Some(0.0), Some(0.0), Some(0.0), None))),
            [0, 0, 0, 255]
        ));
    }

    #[test]
    fn oklch_matches_known_colours() {
        // oklch(62.8% 0.2577 29.23) is sRGB red
        let red = Color::Oklch(cssparser_color::Oklch::new(
            Some(0.628),
            Some(0.2577),
            Some(29.23),
            None,
        ));
        assert!(close(rgba(red), [255, 0, 0, 255]), "{:?}", rgba(red));

        // a neutral colour stays neutral
        let gray = Color::Oklch(cssparser_color::Oklch::new(Some(0.7), Some(0.0), Some(120.0), None));
        let [r, g, b, _] = rgba(gray);
        assert!((r as i32 - g as i32).abs() <= 1 && (g as i32 - b as i32).abs() <= 1);
    }

    #[test]
    fn lab_matches_known_colours() {
        // sRGB red in CIELAB (D50)
        let red = Color::Lab(cssparser_color::Lab::new(
            Some(54.288),
            Some(80.798),
            Some(69.879),
            None,
        ));
        assert!(close(rgba(red), [255, 0, 0, 255]), "{:?}", rgba(red));

        assert!(close(
            rgba(Color::Lab(cssparser_color::Lab::new(Some(0.0), Some(0.0), Some(0.0), None))),
            [0, 0, 0, 255]
        ));
        assert!(close(
            rgba(Color::Lab(cssparser_color::Lab::new(Some(100.0), Some(0.0), Some(0.0), None))),
            [255, 255, 255, 255]
        ));
    }

    #[test]
    fn lch_matches_known_colours() {
        // the same red in CIE LCH: hue 40.855°, chroma 106.824
        let red = Color::Lch(cssparser_color::Lch::new(
            Some(54.288),
            Some(106.824),
            Some(40.855),
            None,
        ));
        assert!(close(rgba(red), [255, 0, 0, 255]), "{:?}", rgba(red));
    }

    #[test]
    fn color_function_spaces_agree_for_srgb_red() {
        use cssparser_color::ColorFunction;

        for space in [
            PredefinedColorSpace::Srgb,
            PredefinedColorSpace::SrgbLinear,
        ] {
            let c = Color::ColorFunction(ColorFunction::new(space, Some(1.0), Some(0.0), Some(0.0), None));
            assert!(close(rgba(c), [255, 0, 0, 255]), "mismatch in {space:?}");
        }

        // XYZ D65 coordinates of sRGB red
        let xyz = Color::ColorFunction(ColorFunction::new(
            PredefinedColorSpace::XyzD65,
            Some(0.4124),
            Some(0.2126),
            Some(0.0193),
            None,
        ));
        assert!(close(rgba(xyz), [255, 0, 0, 255]), "{:?}", rgba(xyz));

        // display-p3 red is outside the sRGB gamut and clips to red
        let p3 = Color::ColorFunction(ColorFunction::new(
            PredefinedColorSpace::DisplayP3,
            Some(1.0),
            Some(0.0),
            Some(0.0),
            None,
        ));
        let [r, g, b, _] = rgba(p3);
        assert_eq!(r, 255);
        assert!(g < 30 && b < 30, "{:?}", rgba(p3));
    }

    #[test]
    fn every_rgb_space_maps_white_to_white_and_grey_to_grey() {
        use cssparser_color::ColorFunction;
        use PredefinedColorSpace::*;

        let spaces = [
            Srgb,
            SrgbLinear,
            DisplayP3,
            DisplayP3Linear,
            A98Rgb,
            ProphotoRgb,
            Rec2020,
        ];
        for space in spaces {
            let component = |v: f32| Color::ColorFunction(ColorFunction::new(space, Some(v), Some(v), Some(v), None));
            let white = rgba(component(1.0));
            assert!(
                close(white, [255, 255, 255, 255]),
                "{space:?} white is {white:?}"
            );

            let [r, g, b, _] = rgba(component(0.5));
            assert!(
                (r as i32 - b as i32).abs() <= 2 && (g as i32 - b as i32).abs() <= 2,
                "{space:?} is not neutral: {r}, {g}, {b}"
            );
        }
    }

    #[test]
    fn alpha_survives_every_space() {
        let c = Color::Oklch(cssparser_color::Oklch::new(
            Some(0.628),
            Some(0.2577),
            Some(29.23),
            Some(0.5),
        ));
        assert_eq!(rgba(c)[3], 128);
    }
}
