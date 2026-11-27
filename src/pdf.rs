use crate::display_list::{self, DisplayItem, PrintPages, Rect};
use crate::font_usage::FontUsage;
use crate::layout::components::TextOperator;
use crate::style::properties::computed::{IccSpace, PixelLength, PaintColor};
use crate::tagging::TagRecorder;

use lemon_pdf::pagetree::MediaBox;
use lemon_pdf::object::{ByteString, IndirectReference, Name, TextString};
use lemon_pdf::stream::StreamFilter;
use lemon_pdf::{DocumentContext, Page, Pt, Version};

pub(crate) mod color_glyph;
mod encoding;
pub(crate) use encoding::GlyphEncoding;
mod font;
pub(crate) mod gradient;
pub(crate) mod text;

use std::collections::HashMap;
use std::io::{BufWriter, Error, Write};

impl From<PixelLength> for Pt {
    fn from(px: PixelLength) -> Pt {
        Pt(f64::from(px.0) * 0.75)
    }
}

/// Tracks the marked content of a text object while it is written.
///
/// In a tagged PDF all content is either tagged or an artifact (PDF/UA). Content that no tag
/// covers (text hidden from the structure with `aria-hidden`, for instance) is wrapped in an
/// implicit `/Artifact` marked-content sequence.
#[derive(Default)]
struct MarkedContent {
    /// For every open marked-content sequence, whether it was written (tags are dropped when
    /// the PDF is not tagged).
    stack: Vec<bool>,
    implicit_artifact: bool,
}

impl MarkedContent {
    /// Called before content is painted: opens an implicit artifact if no marked content
    /// covers the content.
    fn before_content(&mut self, page: &mut PageContext, tagged: bool) -> Result<(), Error> {
        if tagged && self.stack.is_empty() && !self.implicit_artifact {
            page.push_operand(RawPdf("/Artifact".into()))?;
            page.apply_operator("BMC")?;
            self.implicit_artifact = true;
        }
        Ok(())
    }

    /// Called before an explicit marked-content sequence starts at the top level.
    fn close_implicit_artifact(&mut self, page: &mut PageContext) -> Result<(), Error> {
        if self.stack.is_empty() && self.implicit_artifact {
            page.apply_operator("EMC")?;
            self.implicit_artifact = false;
        }
        Ok(())
    }

    fn end(&mut self, page: &mut PageContext) -> Result<(), Error> {
        if self.stack.pop() == Some(true) {
            page.apply_operator("EMC")?;
        }
        Ok(())
    }

    /// Closes everything that is still open at the end of the text object.
    fn finish(mut self, page: &mut PageContext) -> Result<(), Error> {
        while !self.stack.is_empty() {
            self.end(page)?;
        }
        self.close_implicit_artifact(page)
    }
}

type PageContext<'p, 'c, 'b> = lemon_pdf::content::PageContext<'p, 'c, 'b>;

/// How an image is embedded: raster images as an image XObject, SVG images as a form.
enum Embedded {
    Raster(IndirectReference<lemon_pdf::image::ImageXObject>),
    /// An SVG image as a form, with its size.
    Svg(lemon_pdf::form::FormReference, f32, f32, bool),
}

/// The opacity (`ca` and `CA`) that is set in the graphics state of the content stream being
/// written, so that an `ExtGState` is only written when it changes. It follows `Push` and `Pop`
/// (`q` and `Q`), which restore the opacity.
#[derive(Debug)]
struct Alpha {
    /// `UNKNOWN` after something that changes the opacity without restoring it.
    fill: u16,
    stroke: u16,
    saved: Vec<(u16, u16)>,
}

impl Alpha {
    const UNKNOWN: u16 = u16::MAX;

    fn new() -> Self {
        Alpha { fill: u16::from(u8::MAX), stroke: u16::from(u8::MAX), saved: vec![] }
    }

    /// Forgets the opacity: the next colour or `reset` writes it again.
    fn invalidate(&mut self) {
        self.fill = Self::UNKNOWN;
        self.stroke = Self::UNKNOWN;
    }

    fn push(&mut self) {
        self.saved.push((self.fill, self.stroke));
    }

    fn pop(&mut self) {
        if let Some((fill, stroke)) = self.saved.pop() {
            self.fill = fill;
            self.stroke = stroke;
        }
    }

    fn set_fill(&mut self, context: &mut PageContext<'_, '_, '_>, alpha: u8) -> Result<(), Error> {
        if self.fill != u16::from(alpha) {
            if alpha != u8::MAX {
                pdfa1_forbid("a colour with an alpha channel, or `opacity`")?;
            }
            context.set_fill_alpha(f64::from(alpha) / f64::from(u8::MAX))?;
            self.fill = u16::from(alpha);
        }
        Ok(())
    }

    fn set_stroke(&mut self, context: &mut PageContext<'_, '_, '_>, alpha: u8) -> Result<(), Error> {
        if self.stroke != u16::from(alpha) {
            if alpha != u8::MAX {
                pdfa1_forbid("a colour with an alpha channel, or `opacity`")?;
            }
            context.set_stroke_alpha(f64::from(alpha) / f64::from(u8::MAX))?;
            self.stroke = u16::from(alpha);
        }
        Ok(())
    }

    /// Makes what is painted next opaque, for the items that do not have a colour of their own
    /// (images, colour glyphs) and must not take the transparency of the colour set before.
    fn reset(&mut self, context: &mut PageContext<'_, '_, '_>) -> Result<(), Error> {
        self.set_fill(context, u8::MAX)?;
        self.set_stroke(context, u8::MAX)
    }
}

/// Linear-light Display P3 to CIE XYZ (D65), and CIE XYZ (D65) to linear-light Rec. 2020 (CSS Color 4).
#[rustfmt::skip]
const P3_TO_XYZ: [[f64; 3]; 3] = [
    [0.486_570_948_6, 0.265_667_693_2, 0.198_217_285_2],
    [0.228_974_564_1, 0.691_738_521_8, 0.079_286_914_1],
    [0.0,             0.045_113_381_9, 1.043_944_368_9],
];
#[rustfmt::skip]
const XYZ_TO_REC2020: [[f64; 3]; 3] = [
    [ 1.716_651_187_97, -0.355_670_783_78, -0.253_366_281_37],
    [-0.666_684_351_83,  1.616_481_236_63,  0.015_768_545_81],
    [ 0.017_639_857_45, -0.042_770_613_26,  0.942_103_121_24],
];

fn multiply(matrix: &[[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row.iter().zip(vector).map(|(m, v)| m * v).sum())
}

/// The coordinates of a colour (CIE XYZ, D65) in the Rec. 2020 profile with the gamma 2.4: the linear
/// ones to the power of 1/2.4. A colour that Rec. 2020 cannot hold is clipped, the only clipping here.
pub(crate) fn rec2020_gamma24_components(xyz: [f64; 3]) -> [f64; 3] {
    multiply(&XYZ_TO_REC2020, xyz).map(|linear| linear.clamp(0.0, 1.0).powf(1.0 / 2.4))
}

/// The ICC space of the colours that sRGB cannot hold (Rec. 2020 primaries, gamma 2.4), written once for the
/// document.
pub(crate) fn wide_color_space(context: &mut lemon_pdf::content::PageContext<'_, '_, '_>) -> Result<lemon_pdf::color_space::IccBasedColorSpace, Error> {
    Ok(context.pdf_context.icc_color_space("rec2020-gamma24", lemon_pdf::icc::rec2020_gamma24_profile, 3, Some(StreamFilter::Deflate))?)
}

/// Bradford adaptation from D65 to D50 (CSS Color 4, "Sample code for color conversions").
#[rustfmt::skip]
const D65_TO_D50: [[f64; 3]; 3] = [
    [ 1.047_929_820_840_548_8,  0.022_946_793_341_019_088, -0.050_192_229_543_135_57],
    [ 0.029_627_815_688_159_344, 0.990_434_484_573_249,    -0.017_073_825_029_385_14],
    [-0.009_243_058_152_591_178, 0.015_055_144_896_577_895,  0.751_874_289_958_000_8],
];
#[rustfmt::skip]
const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.412_390_799_265_959_5, 0.357_584_339_383_878, 0.180_480_788_401_834_3],
    [0.212_639_005_871_510_36, 0.715_168_678_767_756, 0.072_192_315_360_733_7],
    [0.019_330_818_715_591_85, 0.119_194_779_794_626, 0.950_532_152_249_660_7],
];
#[rustfmt::skip]
const A98_TO_XYZ: [[f64; 3]; 3] = [
    [0.576_669_042_910_131_1, 0.185_558_237_906_546_4, 0.188_228_646_234_245_6],
    [0.297_344_975_250_535_9, 0.627_363_566_255_466_3, 0.075_291_458_493_997_8],
    [0.027_031_361_386_412_36, 0.070_688_852_535_827_6, 0.991_337_536_837_639_2],
];
#[rustfmt::skip]
const REC2020_TO_XYZ: [[f64; 3]; 3] = [
    [0.636_958_048_301_291, 0.144_616_903_586_208_4, 0.168_880_975_164_172_1],
    [0.262_700_212_011_267_0, 0.677_998_071_518_871_0, 0.059_301_716_469_861_9],
    [0.0,                     0.028_072_693_049_087_5, 1.060_985_057_710_791_3],
];
/// The colorants of ProPhoto RGB, which is relative to D50 already.
#[rustfmt::skip]
const PROPHOTO_TO_XYZ_D50: [[f64; 3]; 3] = [
    [0.797_766_6, 0.135_181_3, 0.031_347_73],
    [0.288_074_8, 0.711_835_2, 0.000_089_94],
    [0.0,         0.0,         0.825_104_6],
];

/// The ICC space of a colour of `space`: the profile has its primaries and its curve, as CSS Color 4
/// says them, written once for the document.
fn icc_space(
    context: &mut lemon_pdf::content::PageContext<'_, '_, '_>,
    space: IccSpace,
) -> Result<lemon_pdf::color_space::IccBasedColorSpace, Error> {
    use lemon_pdf::icc::{ParametricCurve, rgb_matrix_profile};
    const LINEAR: ParametricCurve = ParametricCurve { g: 1.0, a: 1.0, b: 0.0, c: 0.0, d: 0.0 };
    let colorants = |to_xyz: &[[f64; 3]; 3], adapt: bool| -> [(f64, f64, f64); 3] {
        [0, 1, 2].map(|column| {
            let xyz = [to_xyz[0][column], to_xyz[1][column], to_xyz[2][column]];
            let [x, y, z] = if adapt { multiply(&D65_TO_D50, xyz) } else { xyz };
            (x, y, z)
        })
    };
    let (key, name, to_xyz, adapt, curve) = match space {
        IccSpace::SrgbLinear => ("srgb-linear", "sRGB linear", SRGB_TO_XYZ, true, LINEAR),
        IccSpace::DisplayP3 => ("display-p3", "Display P3", P3_TO_XYZ, true, ParametricCurve::SRGB),
        IccSpace::DisplayP3Linear => ("display-p3-linear", "Display P3 linear", P3_TO_XYZ, true, LINEAR),
        IccSpace::A98Rgb => ("a98-rgb", "Adobe RGB (1998)", A98_TO_XYZ, true, ParametricCurve { g: 563.0 / 256.0, a: 1.0, b: 0.0, c: 0.0, d: 0.0 }),
        IccSpace::ProphotoRgb => (
            "prophoto-rgb",
            "ProPhoto RGB",
            PROPHOTO_TO_XYZ_D50,
            false,
            ParametricCurve { g: 1.8, a: 1.0, b: 0.0, c: 1.0 / 16.0, d: 16.0 / 512.0 },
        ),
        // pure gamma 2.4, as CSS Color 4 says, and the one that an iPhone decodes every Rec. 2020 profile with
        IccSpace::Rec2020 => return wide_color_space(context),
    };
    let [red, green, blue] = colorants(&to_xyz, adapt);
    Ok(context.pdf_context.icc_color_space(key, || rgb_matrix_profile(name, red, green, blue, curve), 3, Some(StreamFilter::Deflate))?)
}

/// Makes the fills (or the strokes) of the page use `color`, in the space that CSS wrote it in:
///
/// - sRGB (`rgb()`, `hsl()`, a hex code, a name, `color(srgb …)`) is a device colour; every page and form
///   defines `DefaultRGB` as sRGB, so that it is sRGB in every viewer that honours it ([`set_device_defaults`]);
/// - a colour of `color()` in a space with a profile (Display P3, Adobe RGB, ProPhoto RGB, Rec. 2020, and the
///   linear ones) is in an ICC space of that profile, written once for the document;
/// - every other colour (`lab()`, `lch()`, `xyz`, an `oklab()` or `oklch()` that sRGB cannot hold, a colour
///   that is outside of the space it was written in) is CIELAB (D50): the PDF has no Oklab, so those are
///   converted. A device may show a* and b* only within ±128 (an iPhone scales the range), so a colour
///   beyond is in the Rec. 2020 ICC space if that holds it, else its chroma is reduced to fit.
///
/// The alpha is not set.
fn set_color(context: &mut lemon_pdf::content::PageContext<'_, '_, '_>, color: PaintColor, stroke: bool) -> Result<(), Error> {
    let xyz = match color {
        PaintColor::Srgb(color) => {
            let channel = |value: u8| f64::from(value) / f64::from(u8::MAX);
            let (r, g, b) = (channel(color.red), channel(color.green), channel(color.blue));
            return if stroke { context.device_rgb_stroke_color(r, g, b) } else { context.device_rgb_fill_color(r, g, b) };
        }
        // PDF/A-1 does not allow the profiles of version 4 that the ICC spaces are: the colour is CIELAB
        PaintColor::Icc(_) if pdfa1_active() => color.to_xyz().map(f64::from),
        PaintColor::Icc(color) => {
            let space = icc_space(context, color.space)?;
            let space = context.add_color_space(space);
            let components = color.components().map(|value| (f64::from(value) * 100_000.0).round() / 100_000.0);
            return if stroke { context.set_stroke_color_components(&space, &components) } else { context.set_fill_color_components(&space, &components) };
        }
        PaintColor::Xyz(color) => color.components().map(f64::from),
    };
    let [x, y, z] = multiply(&D65_TO_D50, xyz);
    let white = lemon_pdf::color_space::D50_WHITE_POINT;
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x / white[0]), f(y / white[1]), f(z / white[2]));
    let (lightness, a, b) = (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz));
    let round = |value: f64| (value * 10_000.0).round() / 10_000.0;
    let lab = lemon_pdf::color_space::LabColorSpace::d50();
    let limit = lab.range[1];
    let (mut a, mut b) = (a, b);
    if a.abs() > limit || b.abs() > limit {
        // A device may show a* and b* only within ±128 (an iPhone scales the range), so a colour beyond is in the
        // ICC space of Rec. 2020 if that holds it, and else its chroma is reduced (the lightness and the hue stay).
        let linear = multiply(&XYZ_TO_REC2020, xyz);
        if linear.iter().all(|v| (-0.000_5..=1.000_5).contains(v)) {
            let space = wide_color_space(context)?;
            let space = context.add_color_space(space);
            let components = rec2020_gamma24_components(xyz).map(|value| (value * 100_000.0).round() / 100_000.0);
            return if stroke { context.set_stroke_color_components(&space, &components) } else { context.set_fill_color_components(&space, &components) };
        }
        let scale = limit / a.abs().max(b.abs());
        (a, b) = (a * scale, b * scale);
    }
    let name = context.add_lab_color_space(lab);
    let lightness = round(lightness.clamp(0.0, 100.0));
    if stroke { context.lab_stroke_color(&name, lightness, round(a), round(b)) } else { context.lab_fill_color(&name, lightness, round(a), round(b)) }
}

/// Defines what a device colour means in the resources of the page or form `context` (`DefaultRGB`):
/// sRGB. A form has resources of its own, which the page's do not reach, so every form and pattern
/// has to call this before it paints.
pub(crate) fn set_device_defaults(context: &mut lemon_pdf::content::PageContext<'_, '_, '_>) -> Result<(), Error> {
    let srgb = context.pdf_context.icc_color_space("srgb", lemon_pdf::icc::srgb_profile, 3, Some(StreamFilter::Deflate))?;
    context.set_default_color_space(lemon_pdf::color_space::DefaultColorSpace::Rgb, srgb);
    Ok(())
}

/// Makes the fills of the page use `color`, see [`set_color`].
pub(crate) fn set_fill_color(context: &mut lemon_pdf::content::PageContext<'_, '_, '_>, color: PaintColor) -> Result<(), Error> {
    set_color(context, color, false)
}

/// Makes the strokes of the page use `color`, see [`set_color`].
pub(crate) fn set_stroke_color(context: &mut lemon_pdf::content::PageContext<'_, '_, '_>, color: PaintColor) -> Result<(), Error> {
    set_color(context, color, true)
}

/// Paints the items of a display list. The coordinates of the items are in pixels from the top
/// left corner of a page that is `page_height` high.
#[allow(clippy::too_many_arguments)]
fn paint_items(
    page_context: &mut PageContext<'_, '_, '_>,
    display_list: &mut display_list::DisplayList,
    page_height: PixelLength,
    filter: Option<StreamFilter>,
    font_map: &HashMap<String, font::EmbeddedFont>,
    usage: &FontUsage,
    images: &HashMap<*const crate::image::LoadedImage, Embedded>,
    color_glyphs: &std::cell::RefCell<color_glyph::ColorGlyphs>,
    tagged: bool,
    recorder: &mut Option<TagRecorder>,
    page_index: usize,
) -> Result<(), Error> {
    let items: Vec<&DisplayItem> = display_list.items().collect();
    paint_slice(
        page_context,
        &items,
        page_height,
        filter,
        font_map,
        usage,
        images,
        color_glyphs,
        tagged,
        recorder,
        page_index,
    )
}

/// Paints items. The ones between a `BeginGroup` and its `EndGroup` are painted into a form that
/// is a transparency group, which is drawn with the opacity of the group.
#[allow(clippy::too_many_arguments)]
fn paint_slice(
    page_context: &mut PageContext<'_, '_, '_>,
    items: &[&DisplayItem],
    page_height: PixelLength,
    filter: Option<StreamFilter>,
    font_map: &HashMap<String, font::EmbeddedFont>,
    usage: &FontUsage,
    images: &HashMap<*const crate::image::LoadedImage, Embedded>,
    color_glyphs: &std::cell::RefCell<color_glyph::ColorGlyphs>,
    tagged: bool,
    recorder: &mut Option<TagRecorder>,
    page_index: usize,
) -> Result<(), Error> {
    let mut alpha = Alpha::new();
    let mut remaining = items.iter().copied();
    while let Some(item) = remaining.next() {
        match item {
            &DisplayItem::BeginGroup { opacity, bbox } => {
                // the items of the group, up to the `EndGroup` that belongs to it
                let mut inner = vec![];
                let mut depth = 1;
                for next in remaining.by_ref() {
                    match next {
                        DisplayItem::BeginGroup { .. } => depth += 1,
                        DisplayItem::EndGroup => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(next);
                }
                // an invisible group paints nothing
                if opacity <= 0.0 || inner.is_empty() {
                    continue;
                }
                let area = [
                    Pt::from(bbox.x),
                    Pt::from(page_height - bbox.y - bbox.height),
                    Pt::from(bbox.x + bbox.width),
                    Pt::from(page_height - bbox.y),
                ];
                pdfa1_forbid("`opacity` on an element")?;
                let form = page_context.pdf_context.write_form(
                    area,
                    Some(lemon_pdf::form::TransparencyGroup::new(
                        lemon_pdf::image::ColorSpace::DeviceRGB,
                    )),
                    filter,
                    |form| {
                        set_device_defaults(form)?;
                        // Content of a form is not part of the structure of the page: it is
                        // not tagged.
                        paint_slice(
                            form,
                            &inner,
                            page_height,
                            filter,
                            font_map,
                            usage,
                            images,
                            color_glyphs,
                            false,
                            &mut None,
                            page_index,
                        )
                    },
                )?;
                let key = page_context.add_xobject(form);
                alpha.push();
                page_context.save_graphics_state()?;
                page_context.push_operand(RawPdf("/Artifact".into()))?;
                page_context.apply_operator("BMC")?;
                // the opacity is of the group as a whole; the group itself starts out opaque
                let opacity = f64::from(opacity);
                page_context.apply_ext_gstate(
                    lemon_pdf::graphics_state::ExtGState::new()
                        .with_fill_alpha(opacity)
                        .with_stroke_alpha(opacity),
                )?;
                page_context.draw_xobject(&key)?;
                page_context.apply_operator("EMC")?;
                page_context.restore_graphics_state()?;
                alpha.pop();
            }
            // consumed by the `BeginGroup` that it belongs to
            DisplayItem::EndGroup => {}
            _ => paint_item(
                page_context,
                item,
                &mut alpha,
                page_height,
                filter,
                font_map,
                usage,
                images,
                color_glyphs,
                tagged,
                recorder,
                page_index,
            )?,
        }
    }
    Ok(())
}

/// Paints an item that is not a group.
#[allow(clippy::too_many_arguments)]
fn paint_item(
    page_context: &mut PageContext<'_, '_, '_>,
    item: &DisplayItem,
    alpha: &mut Alpha,
    page_height: PixelLength,
    filter: Option<StreamFilter>,
    font_map: &HashMap<String, font::EmbeddedFont>,
    usage: &FontUsage,
    images: &HashMap<*const crate::image::LoadedImage, Embedded>,
    color_glyphs: &std::cell::RefCell<color_glyph::ColorGlyphs>,
    tagged: bool,
    recorder: &mut Option<TagRecorder>,
    page_index: usize,
) -> Result<(), Error> {
    {
        match item {
            // handled by `paint_slice`
            DisplayItem::BeginGroup { .. } | DisplayItem::EndGroup => {}
            DisplayItem::Push => {
                alpha.push();
                page_context.save_graphics_state()?
            }
            DisplayItem::Pop => {
                alpha.pop();
                page_context.restore_graphics_state()?
            }
        &DisplayItem::Transform(matrix) => {
            // the matrix is in pixels with y down, from the top left corner of the page: bring
            // it to the points and the y axis of the PDF
            use crate::style::properties::transform::multiply;
            let h = page_height.0;
            let to_pdf = [0.75, 0.0, 0.0, -0.75, 0.0, 0.75 * h];
            let from_pdf = [1.0 / 0.75, 0.0, 0.0, -1.0 / 0.75, 0.0, h];
            let m = multiply(multiply(to_pdf, matrix), from_pdf);
            page_context.concatenate_matrix(
                f64::from(m[0]),
                f64::from(m[1]),
                f64::from(m[2]),
                f64::from(m[3]),
                Pt(f64::from(m[4])),
                Pt(f64::from(m[5])),
            )?;
        }
            DisplayItem::Translate(x, y) => {
                let x = *x;
                let y = -*y;
                page_context.concatenate_matrix(1.0, 0.0, 0.0, 1.0, x.into(), y.into())?;
            }
            &DisplayItem::Color(color) => {
                set_fill_color(page_context, color)?;
                alpha.set_fill(page_context, color.alpha())?;
            }
            &DisplayItem::StrokeColor(color) => {
                set_stroke_color(page_context, color)?;
                alpha.set_stroke(page_context, color.alpha())?;
            }
            &DisplayItem::LineString {
                ref points,
                line_width,
            } => {
                let mut point_iter =
                    points.iter().map(|&crate::display_list::Point { x, y }| {
                        let x = Pt::from(x);
                        let y = Pt::from(page_height - y);
                        (x, y)
                    });

                if let Some((x, y)) = point_iter.next() {
                    page_context.push_operand(RawPdf("/Artifact".into()))?;
                    page_context.apply_operator("BMC")?;
                    page_context.move_to(x, y)?
                } else {
                    return Ok(());
                }

                for (x, y) in point_iter {
                    page_context.line_to(x, y)?
                }

                page_context.line_width(line_width.into())?;
                page_context.stroke_path()?;
                page_context.apply_operator("EMC")?;
            }
            DisplayItem::FillPath { path, even_odd } => {
                page_context.push_operand(RawPdf("/Artifact".into()))?;
                page_context.apply_operator("BMC")?;
                emit_path(page_context, path, page_height)?;
                let rule = if *even_odd {
                    lemon_pdf::graphics_state::FillRule::EvenOdd
                } else {
                    lemon_pdf::graphics_state::FillRule::NonZero
                };
                page_context.fill_path_with_rule(rule)?;
                page_context.apply_operator("EMC")?;
            }
            DisplayItem::ColorGlyph(item) => {
                alpha.reset(page_context)?;
                page_context.push_operand(RawPdf("/Artifact".into()))?;
                page_context.apply_operator("BMC")?;
                color_glyphs.borrow_mut().paint_at(
                    page_context,
                    &item.font,
                    item.glyph,
                    item.color,
                    (Pt::from(item.origin.x), Pt::from(page_height - item.origin.y)),
                    Pt::from(item.size),
                    filter,
                )?;
                page_context.apply_operator("EMC")?;
            }
            DisplayItem::Gradient(resolved) => {
                alpha.reset(page_context)?;
                gradient::paint(page_context, resolved, page_height, filter)?;
                // a gradient without a clip leaves its opacity set
                alpha.invalidate();
            }
            DisplayItem::Image { rect, image } => {
                let Some(embedded) = images.get(&std::sync::Arc::as_ptr(image)) else {
                    return Ok(());
                };
                alpha.reset(page_context)?;
                page_context.push_operand(RawPdf("/Artifact".into()))?;
                page_context.apply_operator("BMC")?;
                match *embedded {
                    Embedded::Raster(reference) => {
                        let key = page_context.add_xobject(reference);
                        page_context.draw_image(
                            &key,
                            Pt::from(rect.x),
                            Pt::from(page_height - rect.y - rect.height),
                            Pt::from(rect.width),
                            Pt::from(rect.height),
                        )?;
                    }
                    Embedded::Svg(form, width, height, unscaled) => {
                        // like `object-fit: contain`: the SVG keeps its proportions
                        // and is centered in the box (an element without a `viewBox` keeps its
                        // scale and the corner, and what is outside of the box is clipped)
                        let (box_w, box_h) = (rect.width.0, rect.height.0);
                        let scale = if unscaled { 1.0 } else { (box_w / width).min(box_h / height) };
                        let (free_w, free_h) =
                            if unscaled { (0.0, 0.0) } else { (box_w - width * scale, box_h - height * scale) };
                        // layout is in pixels, the PDF in points
                        let x = Pt::from(rect.x + PixelLength(free_w / 2.0));
                        let y = Pt::from(page_height - rect.y - PixelLength(free_h / 2.0));
                        let scale = f64::from(Pt::from(PixelLength(scale)).0);
                        let key = page_context.add_xobject(form);
                        page_context.save_graphics_state()?;
                        page_context.rect(
                            Pt::from(rect.x),
                            Pt::from(page_height - rect.y - rect.height),
                            Pt::from(rect.width),
                            Pt::from(rect.height),
                        )?;
                        page_context.clip(lemon_pdf::graphics_state::FillRule::NonZero)?;
                        page_context.end_path()?;
                        // SVG has y pointing down
                        page_context.concatenate_matrix(
                            scale,
                            0.0,
                            0.0,
                            -scale,
                            x,
                            y,
                        )?;
                        page_context.draw_xobject(&key)?;
                        page_context.restore_graphics_state()?;
                    }
                }
                page_context.apply_operator("EMC")?;
            }
            DisplayItem::BlendGroup(group) => {
                alpha.reset(page_context)?;
                gradient::paint_blend_group(page_context, group, page_height, filter)?
            }
            DisplayItem::ClipPath { path, even_odd } => {
                emit_path(page_context, path, page_height)?;
                let rule = if *even_odd {
                    lemon_pdf::graphics_state::FillRule::EvenOdd
                } else {
                    lemon_pdf::graphics_state::FillRule::NonZero
                };
                page_context.clip(rule)?;
                page_context.end_path()?;
            }
            &DisplayItem::FilledRect(Rect {
                x,
                y,
                width,
                height,
            }) => {
                let x = Pt::from(x);
                let y = Pt::from(page_height - y - height);
                let width = Pt::from(width);
                let height = Pt::from(height);

                page_context.push_operand(RawPdf("/Artifact".into()))?;
                page_context.apply_operator("BMC")?;
                page_context.rect(x, y, width, height)?;
                page_context.fill_path()?;
                page_context.apply_operator("EMC")?;
            }
            DisplayItem::Paragraph(par) => {
                let x = par.start_pos.0;
                let y = page_height - par.start_pos.1;

                page_context.begin_text()?;
                page_context.set_position(x.into(), y.into())?;
                let mut marked = MarkedContent::default();

                // the text that the fonts cannot describe is given where it is drawn
                let marked_ops = text::with_actual_text(&par.ops, usage);
                let ops: &[TextOperator] = marked_ops.as_deref().unwrap_or(&par.ops);
                // the encoding of the font that is selected: the text starts by selecting one
                let mut encoding = encoding::GlyphEncoding::Identity;
                for op in ops {
                    match op {
                        TextOperator::SetFont(font_name, size) => {
                            // A font that draws no glyph anywhere is not embedded (e.g.
                            // the font of text that is drawn entirely with fallback
                            // fonts). Selecting it would change nothing.
                            if let Some(font) = font_map.get(font_name) {
                                let key = page_context.add_font(font.reference);
                                page_context.set_font(&key, (*size).into())?;
                                encoding = font.encoding;
                            }
                        }
                        &TextOperator::SetColor(color) => {
                            set_fill_color(page_context, color)?;
                            alpha.set_fill(page_context, color.alpha())?;
                        }
                        &TextOperator::KernedGlyphs(range) => {
                            marked.before_content(page_context, tagged)?;
                            page_context.push_operand(text::KernedArray::new(range, &par.ops.glyphs, encoding))?;
                            page_context.apply_operator("TJ")?;
                        }
                        &TextOperator::NewLine { x_offset, leading } => {
                            page_context
                                .set_position(x_offset.into(), (-leading).into())?;
                        }
                        &TextOperator::SetWordSpacing(spacing) => {
                            page_context.push_operand(Pt::from(spacing))?;
                            page_context.apply_operator("Tw")?;
                        }
                        TextOperator::BeginActualText(text) => {
                            marked.before_content(page_context, tagged)?;
                            marked.stack.push(true);
                            page_context.push_operand(RawPdf("/Span".into()))?;
                            let mut hex = String::from("FEFF");
                            for unit in text.encode_utf16() {
                                hex.push_str(&format!("{unit:04X}"));
                            }
                            page_context
                                .push_operand(RawPdf(format!("<< /ActualText <{hex}> >>")))?;
                            page_context.apply_operator("BDC")?;
                        }
                        TextOperator::BeginArtifact => {
                            // page numbers, running headers and footers
                            marked.close_implicit_artifact(page_context)?;
                            marked.stack.push(true);
                            page_context.push_operand(RawPdf("/Artifact".into()))?;
                            page_context
                                .push_operand(RawPdf("<< /Type /Pagination >>".into()))?;
                            page_context.apply_operator("BDC")?;
                        }
                        TextOperator::BeginTag(tag) => match recorder.as_mut() {
                            Some(recorder) => {
                                marked.close_implicit_artifact(page_context)?;
                                marked.stack.push(true);
                                let (role, mcid) = recorder.mark(page_index, *tag);
                                page_context.push_operand(RawPdf(format!("/{role}")))?;
                                page_context.push_operand(RawPdf(format!(
                                    "<< /MCID {mcid} >>"
                                )))?;
                                page_context.apply_operator("BDC")?;
                            }
                            None => marked.stack.push(false),
                        },
                        TextOperator::EndMarkedContent => {
                            marked.end(page_context)?;
                        }
                        // painted by itself, at its position (see `DisplayItem::ColorGlyph`)
                        TextOperator::ColorGlyph { .. } => {}
                        &TextOperator::SetRise(rise) => {
                            page_context.push_operand(Pt::from(rise))?;
                            page_context.apply_operator("Ts")?;
                        }
                        &TextOperator::Embed { x, y, .. } => {
                            // resolved by the display list; only the position is left
                            page_context.push_operand(1.0f32)?;
                            page_context.push_operand(0.0f32)?;
                            page_context.push_operand(0.0f32)?;
                            page_context.push_operand(1.0f32)?;
                            page_context.push_operand(Pt::from(x).0 as f32)?;
                            page_context.push_operand(Pt::from(page_height - y).0 as f32)?;
                            page_context.apply_operator("Tm")?;
                        }
                        &TextOperator::SetTransform(matrix) => {
                            let matrix: [f32; 6] = [
                                matrix.a,
                                matrix.b,
                                matrix.c,
                                matrix.d,
                                Pt::from(matrix.e).0 as f32,
                                Pt::from(page_height - matrix.f).0 as f32,
                            ];
                            for elem in &matrix {
                                page_context.push_operand(*elem)?;
                            }
                            page_context.apply_operator("Tm")?;
                        }
                    }
                }

                marked.finish(page_context)?;
                page_context.end_text()?;
            }
        }
    }


    Ok(())
}

/// Gives the image the ICC profile of its file, if it has one that a PDF can use for the image (its colour
/// space and number of components are those of the image); otherwise the image keeps its device
/// colours, which are sRGB (`DefaultRGB`), what CSS says for an image without a profile. The profile
/// is written once, however many images have it.
fn with_icc_profile(context: &mut DocumentContext<'_>, raster: &crate::image::RasterImage, image: lemon_pdf::image::Image) -> lemon_pdf::image::Image {
    let Some(profile) = raster.icc_profile() else {
        return image;
    };
    let components = image.color_space().components();
    let info = match lemon_pdf::icc::inspect_profile(&profile) {
        Ok(info) if info.components == components => info,
        Ok(info) => {
            log::warn!("the ICC profile of an image has {} components and the image {}: it is ignored", info.components, components);
            return image;
        }
        Err(why) => {
            log::warn!("the ICC profile of an image is ignored: {why}");
            return image;
        }
    };
    if info.major_version >= 3 {
        let _ = pdfa1_forbid("an image with an ICC profile of version 4 (the profile has to be version 2)");
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile.hash(&mut hasher);
    let key = format!("image-profile-{:016x}-{}", hasher.finish(), profile.len());
    match context.icc_color_space(&key, || profile, i64::from(components), Some(StreamFilter::Deflate)) {
        Ok(space) => image.with_icc_profile(space),
        Err(error) => {
            log::warn!("the ICC profile of an image cannot be written: {error}");
            image
        }
    }
}

/// Writes a JPEG or PNG image into the PDF with the colour space of its file (see [`with_icc_profile`]).
/// Both `<img>` and the `<image>` elements of SVG images embed their rasters with it, so that they
/// have the same colour handling.
pub(crate) fn write_raster(
    context: &mut DocumentContext<'_>,
    raster: &crate::image::RasterImage,
) -> Result<lemon_pdf::object::IndirectReference<lemon_pdf::image::ImageXObject>, String> {
    let pdf_image = raster.to_pdf_image()?;
    if pdf_image.has_alpha() {
        let _ = pdfa1_forbid("an image with an alpha channel (it is a soft mask)");
    }
    let pdf_image = with_icc_profile(context, raster, pdf_image);
    context.write_image(pdf_image).map_err(|error| error.to_string())
}

/// Embeds the images of the items of a display list that are not embedded yet.
fn embed_images(
    context: &mut DocumentContext<'_>,
    display_list: &mut display_list::DisplayList,
    images: &mut HashMap<*const crate::image::LoadedImage, Embedded>,
    svg_writer: &mut crate::svg::SvgWriter,
) {
    for item in display_list.items() {
        let DisplayItem::Image { image, .. } = item else {
            continue;
        };
        let key = std::sync::Arc::as_ptr(image);
        if images.contains_key(&key) {
            continue;
        }
        let embedded = match &**image {
            crate::image::LoadedImage::Raster(raster) => write_raster(context, raster).map(Embedded::Raster),
            crate::image::LoadedImage::Svg(svg) => svg_writer
                .write(context, &svg.tree, svg.unscaled)
                .map(|form| Embedded::Svg(form, svg.tree.size().width(), svg.tree.size().height(), svg.unscaled))
                .map_err(|error| error.to_string()),
        };
        match embedded {
            Ok(embedded) => {
                images.insert(key, embedded);
            }
            Err(error) => log::warn!("cannot embed an image: {}", error),
        }
    }
}

/// The level of PDF/A (ISO 19005) that a file is written for, like `2b` or `3u`.
///
/// Parts 2 and 3 are the ones that fit the PDF 1.7 that is written (part 4 is for PDF 2.0). Part 1
/// forbids transparency, object streams and version 4 of ICC profiles, so a file of level `1b` is
/// written without the first two, and a document that needs any of the things that it forbids is an
/// error (see [`pdfa1_forbid`]); only the level `b` of part 1 is supported. Level `b` is about the looks
/// of the pages, `u` adds that the text can be extracted, `a` that the file is tagged. Umbruch always
/// writes the maps from the glyphs to Unicode, so `u` costs nothing; `a` needs the structure tree,
/// so it does not go with `--no-tags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfALevel {
    /// 1, 2 or 3.
    pub part: u8,
    /// `A`, `B` or `U`.
    pub conformance: char,
}

impl PdfALevel {
    /// Whether the level needs a tagged PDF.
    pub fn needs_tags(&self) -> bool {
        self.conformance == 'A'
    }
}

impl std::str::FromStr for PdfALevel {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim().to_ascii_uppercase();
        let mut chars = text.chars();
        match (chars.next(), chars.next(), chars.next()) {
            (Some('1'), Some('B'), None) => Ok(PdfALevel { part: 1, conformance: 'B' }),
            (Some('1'), Some('A' | 'U'), None) => Err(format!(
                "`{text}` is not supported: of PDF/A-1 only the level 1b is (the level 1a needs a tagged PDF of version 1.4)"
            )),
            (Some(part @ ('2' | '3')), Some(conformance @ ('A' | 'B' | 'U')), None) => {
                Ok(PdfALevel { part: part as u8 - b'0', conformance })
            }
            _ => Err(format!("`{text}` is not a level of PDF/A that is supported: 1b, 2a, 2b, 2u, 3a, 3b or 3u")),
        }
    }
}

thread_local! {
    /// While a file of PDF/A-1 is written: the things that the document needs and that PDF/A-1 forbids,
    /// in the order they were found.
    static PDFA1_VIOLATIONS: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Whether the file that is written is one of PDF/A-1.
pub(crate) fn pdfa1_active() -> bool {
    PDFA1_VIOLATIONS.with(|violations| violations.borrow().is_some())
}

/// Called where the writer is about to use something that PDF/A-1 forbids (transparency: `what` says which
/// kind, for the message): in a file of PDF/A-1 it notes `what` and returns an error, so that the
/// writer does not write it; the error is also reported when the caller makes light of it (an
/// image that cannot be embedded is only a warning otherwise), because [`write_pdf`] fails with
/// everything that was noted. In every other file it does nothing.
pub(crate) fn pdfa1_forbid(what: &str) -> Result<(), Error> {
    PDFA1_VIOLATIONS.with(|violations| match violations.borrow_mut().as_mut() {
        Some(list) => {
            if !list.iter().any(|known| known == what) {
                list.push(what.to_owned());
            }
            Err(Error::new(std::io::ErrorKind::InvalidInput, format!("{what} is not allowed in PDF/A-1")))
        }
        None => Ok(()),
    })
}

/// The output intent of a PDF/A file is RGB, and PDF/A does not allow device CMYK colours without
/// one that is CMYK, so a CMYK image is an error. (An image has to be converted to RGB.)
fn check_pdfa_images(print_pages: &mut PrintPages) -> Result<(), Error> {
    let lists = print_pages
        .pages
        .iter_mut()
        .map(|page| &mut page.display_list)
        .chain(print_pages.islands.iter_mut().map(|island| &mut island.display_list));
    for list in lists {
        for item in list.items() {
            if let DisplayItem::Image { image, .. } = item {
                let cmyk = match &**image {
                    crate::image::LoadedImage::Raster(raster) => raster.is_cmyk(),
                    crate::image::LoadedImage::Svg(svg) => crate::svg::has_cmyk_image(&svg.tree),
                };
                if cmyk {
                    return Err(Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "the document has a CMYK image, which PDF/A does not allow (the colours are sRGB): convert it to RGB",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Writes the pages as a PDF to `output`. If `tagged`, it is a tagged PDF whose structure tree is the
/// `structure` of the pages. With a `pdfa` level the file is written to conform to it.
///
/// With PDF/A-1 the whole file is held back until it is known to conform, so that nothing is written
/// for a document that does not.
pub fn write_pdf<W: Write>(
    mut output: W,
    print_pages: PrintPages,
    no_filter: bool,
    crop: bool,
    tagged: bool,
    pdfa: Option<PdfALevel>,
) -> Result<(), Error> {
    if !pdfa.is_some_and(|level| level.part == 1) {
        return write_pdf_to(output, print_pages, no_filter, crop, tagged, pdfa);
    }
    // PDF/A-1 forbids transparency, which the writer meets at many places: every one of them notes
    // what it met, and the file is not kept if there was any
    PDFA1_VIOLATIONS.with(|violations| *violations.borrow_mut() = Some(Vec::new()));
    let mut buffer = Vec::new();
    let result = write_pdf_to(&mut buffer, print_pages, no_filter, crop, tagged, pdfa);
    let violations = PDFA1_VIOLATIONS.with(|violations| violations.borrow_mut().take()).unwrap_or_default();
    if !violations.is_empty() {
        return Err(Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("the document cannot be written as PDF/A-1, which does not allow: {}", violations.join("; ")),
        ));
    }
    result?;
    output.write_all(&buffer)?;
    output.flush()
}

fn write_pdf_to<W: Write>(
    output: W,
    mut print_pages: PrintPages,
    no_filter: bool,
    crop: bool,
    tagged: bool,
    pdfa: Option<PdfALevel>,
) -> Result<(), Error> {
    if pdfa.is_some() {
        // before the file is made, so that there is none if there is an error
        check_pdfa_images(&mut print_pages)?;
    }
    let mut context = DocumentContext::with_writer(BufWriter::new(output), Version::Pdf1_7)?;
    if pdfa1_active() {
        // PDF/A-1 (ISO 19005-1, 6.1.4) does not have cross-reference streams or object streams
        context.object_streams = false;
    }

    let structure = std::mem::take(&mut print_pages.structure);
    let outline = std::mem::take(&mut print_pages.outline);
    let named_destinations = std::mem::take(&mut print_pages.named_destinations);
    let page_labels = std::mem::take(&mut print_pages.page_labels);
    // what `write_outline` needs of the pages, which are consumed below
    let page_sizes: Vec<(crate::display_list::Rect, crate::style::properties::computed::PixelLength)> = print_pages
        .pages
        .iter()
        .map(|page| (page.content_rect.clone(), page.height))
        .collect();
    let mut recorder = tagged.then(|| TagRecorder::new(&structure, print_pages.pages.len()));
    if structure.title.is_some() {
        context.set_display_doc_title(true);
    }
    // the document information dictionary and the XMP packet say the same
    let metadata = &structure.metadata;
    let date = metadata.date.or_else(crate::metadata::Timestamp::from_environment);
    let producer = format!("Umbruch {}", env!("CARGO_PKG_VERSION"));
    {
        let info = &mut context.document_info;
        let mut set = |key: &str, value: String| {
            info.insert(key.to_owned(), TextString(value));
        };
        if let Some(title) = &structure.title {
            set("Title", title.clone());
        }
        if !metadata.authors.is_empty() {
            set("Author", metadata.authors.join(", "));
        }
        if let Some(description) = &metadata.description {
            set("Subject", description.clone());
        }
        if let Some(keywords) = &metadata.keywords {
            set("Keywords", keywords.clone());
        }
        if let Some(creator) = &metadata.creator {
            set("Creator", creator.clone());
        }
        set("Producer", producer);
        if let Some(date) = date {
            set("CreationDate", date.pdf());
            set("ModDate", date.pdf());
        }
    }
    if let Some(lang) = &structure.lang {
        context.set_lang(lang);
    }
    if tagged {
        // PDF/UA (ISO 14289-1 7.2, 7.1) needs both
        if structure.lang.is_none() {
            log::warn!("The document has no `lang`: the PDF does not conform to PDF/UA.");
        }
        if structure.title.is_none() {
            log::warn!("The document has no `<title>`: the PDF does not conform to PDF/UA.");
        }
    }
    context.set_metadata(&xmp_metadata(&structure, date, tagged, pdfa))?;
    if pdfa.is_some() {
        // device colours (everything that is painted) are sRGB, which the output intent says
        use lemon_pdf::output_intent::{OutputIntentDesc, OutputIntentSubtype};
        context.add_output_intent(
            OutputIntentDesc {
                subtype: OutputIntentSubtype::PdfA,
                output_condition_identifier: "sRGB IEC61966-2.1".to_owned(),
                info: Some("sRGB IEC61966-2.1".to_owned()),
                profile: lemon_pdf::icc::srgb_profile(),
                components: 3,
            },
            if no_filter { None } else { Some(StreamFilter::Deflate) },
        )?;
    }
    // All pages get their references up front so that links can point to later pages.
    let page_references: Vec<IndirectReference<Page>> = (0..print_pages.pages.len())
        .map(|_| context.create_reference())
        .collect();

    // The fonts are embedded with the glyphs that the document draws. The glyphs of the text of
    // SVG images are shown with the embedded fonts of the document too, so the fonts of those
    // are added to the font map first.
    let (usage, svg_font_names) = crate::font_usage::collect(&mut print_pages);

    // The colour glyphs of the page and of the SVG images, painted by one code path. The fonts are
    // known once the fonts of the SVG text have been added to the font map.
    let color_glyphs =
        std::rc::Rc::new(std::cell::RefCell::new(color_glyph::ColorGlyphs::new(&print_pages.font_map)));

    let mut font_map = HashMap::new();
    for (font_name, glyphs) in usage.fonts() {
        let Some(font) = print_pages.font_map.get(font_name) else { continue };
        log::info!("Embedding font {} into PDF file.", font_name);
        let font = font::embed_into_pdf(font, glyphs, &mut context, !no_filter)?;
        font_map.insert(font_name.to_owned(), font);
    }

    // Every image is embedded once, however often it is painted.
    let mut images: HashMap<*const crate::image::LoadedImage, Embedded> = HashMap::new();
    let page_filter = if no_filter { None } else { Some(StreamFilter::Deflate) };
    let mut svg_writer = crate::svg::SvgWriter::new(page_filter);
    svg_writer.set_color_glyphs(color_glyphs.clone());
    svg_writer.set_fonts(
        svg_font_names,
        font_map
            .iter()
            .filter_map(|(name, font)| {
                Some((name.clone(), (font.reference, print_pages.font_map.get(name)?.upem, font.encoding)))
            })
            .collect(),
    );
    // The content of the `foreignObject` elements of SVG images (HTML and MathML) was laid out
    // like small pages and becomes forms. An island can contain SVG images with islands of their
    // own, which come after it, so the last one is written first.
    for index in (0..print_pages.islands.len()).rev() {
        let mut island = std::mem::take(&mut print_pages.islands[index]);
        embed_images(&mut context, &mut island.display_list, &mut images, &mut svg_writer);
        let (width, height) = (island.width, island.height);
        let form = context.write_form(
            [Pt(0.0), Pt(0.0), Pt::from(width), Pt::from(height)],
            None,
            page_filter,
            |form| {
                set_device_defaults(form)?;
                paint_items(
                    form,
                    &mut island.display_list,
                    height,
                    page_filter,
                    &font_map,
                    &usage,
                    &images,
                    &color_glyphs,
                    false,
                    &mut None,
                    0,
                )
            },
        )?;
        svg_writer.add_island(index as u64, form);
    }
    for page in &mut print_pages.pages {
        embed_images(&mut context, &mut page.display_list, &mut images, &mut svg_writer);
    }

    log::info!("Write {} pages.", print_pages.pages.len());

    for (page_index, mut print_page) in print_pages.pages.into_iter().enumerate() {
        let mut page = Page::new();
        let page_height = print_page.height;
        if tagged {
            // the key of the page in the parent tree; annotations follow the order of the
            // structure tree
            page.struct_parents = Some(page_index as i64);
            page.tabs = Some(Name::from("S"));
        }

        if crop {
            page.set_media_box(MediaBox::new(
                print_page.content_rect.x.into(),
                (page_height - print_page.content_rect.y).into(),
                print_page.content_rect.xmax().into(),
                (page_height - print_page.content_rect.ymax()).into(),
            ));
        } else {
            page.set_media_box(MediaBox::new(
                Pt(0.0),
                Pt(0.0),
                print_page.width.into(),
                print_page.height.into(),
            ));
        }

        let filter = if no_filter {
            None
        } else {
            Some(StreamFilter::Deflate)
        };

        page.add_content(&mut context, filter, |page_context| {
            // device colours (and images and shadings, which have only those) are sRGB, in every
            // viewer, instead of whatever the viewer takes the device to be
            set_device_defaults(page_context)?;
            paint_items(
                page_context,
                &mut print_page.display_list,
                page_height,
                filter,
                &font_map,
                &usage,
                &images,
                &color_glyphs,
                tagged,
                &mut recorder,
                page_index,
            )?;

            Ok(())
        })?;

        // Write the annotations of this page (currently only hyperlinks). The media box of a
        // cropped page starts at the content rect, so link coordinates must be offset by it.
        for hyperlink in print_pages
            .hyperlinks
            .iter()
            .filter(|link| link.page == page_index)
        {
            let (offset_x, offset_y) = if crop {
                (print_page.content_rect.x, print_page.content_rect.y)
            } else {
                (
                    crate::style::properties::computed::PixelLength(0.0),
                    crate::style::properties::computed::PixelLength(0.0),
                )
            };
            let xmin = hyperlink.rect.x - offset_x;
            let ymin = hyperlink.rect.y - offset_y;
            let xmax = hyperlink.rect.xmax() - offset_x;
            let ymax = hyperlink.rect.ymax() - offset_y;

            use lemon_pdf::annotation::{GoToAction, LinkAction, UriAction};
            let action = if let Some(destination) = &hyperlink.destination {
                // a cross-reference: jump to the position of the target element
                let x = Pt::from(destination.x - offset_x);
                // the destination y is the top of the target element; keep it in the same
                // coordinate space as the link rect (bottom-left origin)
                let y = Pt::from(page_height - (destination.y - offset_y));
                LinkAction::GoTo(GoToAction {
                    destination: lemon_pdf::annotation::Destination {
                        page: page_references[destination.page],
                        point: (x, y),
                    },
                })
            } else {
                LinkAction::Uri(UriAction {
                    uri: ByteString::from(hyperlink.href.as_str()),
                })
            };

            // the annotation is the content of the `Link` element of the structure tree, next to
            // the text of the link
            let link_node = hyperlink.node;
            let struct_parent = match (recorder.as_mut(), link_node) {
                (Some(recorder), Some(_)) => Some(recorder.annotation_key()),
                _ => None,
            };
            let contents = link_node
                .and_then(|node| structure.nodes[node as usize].link_text.clone())
                .or_else(|| Some(hyperlink.href.clone()))
                .map(TextString);

            let mut link = lemon_pdf::annotation::LinkAnnotation::new(
                [
                    Pt::from(xmin),
                    Pt::from(page_height - ymin - (ymax - ymin)),
                    Pt::from(xmax),
                    Pt::from(page_height - ymin),
                ],
                action,
            );
            link.contents = contents;
            link.struct_parent = struct_parent;
            let annotation = lemon_pdf::annotation::Annotation::Link(link);
            let annotation = page.add_annotation(&mut context, annotation)?;
            if let (Some(recorder), Some(node), Some(key)) =
                (recorder.as_mut(), link_node, struct_parent)
            {
                recorder.add_annotation(node, page_index, annotation, key);
            }
        }

        context.add_reserved_page(page_references[page_index], page)?;
    }
    {
        use crate::layout::margin_boxes::LabelStyle;
        use lemon_pdf::page_labels::{PageLabel, PageLabelStyle};
        context.set_page_labels(page_labels.iter().map(|range| {
            let style = range.style.map(|style| match style {
                LabelStyle::Decimal => PageLabelStyle::Decimal,
                LabelStyle::LowerRoman => PageLabelStyle::LowerRoman,
                LabelStyle::UpperRoman => PageLabelStyle::UpperRoman,
                LabelStyle::LowerLetters => PageLabelStyle::LowerLetters,
                LabelStyle::UpperLetters => PageLabelStyle::UpperLetters,
            });
            let mut label = PageLabel { style, prefix: None, start: style.map(|_| range.start) };
            if !range.prefix.is_empty() {
                label = label.with_prefix(range.prefix.clone());
            }
            (range.first_page, label)
        }));
    }
    context.set_named_destinations(named_destinations.iter().filter_map(|(id, destination)| {
        Some((id.as_bytes().to_vec(), pdf_destination(destination, &page_references, &page_sizes, crop)?))
    }));
    write_outline(&mut context, &outline, &page_references, &page_sizes, crop)?;
    if !outline.is_empty() {
        // viewers open the panel of the bookmarks
        context.set_page_mode(lemon_pdf::PageMode::UseOutlines);
    }
    if let Some(recorder) = recorder {
        recorder.finish(&mut context, &page_references)?;
    }
    context.finish()?;

    Ok(())
}

/// The destination in the PDF of a position of the layout: the page and the point on it (which
/// is measured from the bottom left corner of the page, and from the content of a cropped page).
fn pdf_destination(
    destination: &crate::layout::components::GoToDestination,
    page_references: &[IndirectReference<Page>],
    page_sizes: &[(crate::display_list::Rect, crate::style::properties::computed::PixelLength)],
    crop: bool,
) -> Option<lemon_pdf::annotation::Destination> {
    let (&page, (content_rect, page_height)) =
        (page_references.get(destination.page)?, page_sizes.get(destination.page)?);
    let (offset_x, offset_y) = if crop { (content_rect.x, content_rect.y) } else { Default::default() };
    Some(lemon_pdf::annotation::Destination {
        page,
        point: (Pt::from(destination.x - offset_x), Pt::from(*page_height - (destination.y - offset_y))),
    })
}

/// Writes the outline (bookmarks) of the PDF from the headings of the document: a heading of
/// level n becomes a child of the closest preceding heading of a lower level. Levels may be
/// skipped (an `h3` right after an `h1` is a child of the `h1`).
fn write_outline(
    context: &mut DocumentContext<'_>,
    entries: &[crate::layout::components::OutlineEntry],
    page_references: &[IndirectReference<Page>],
    page_sizes: &[(crate::display_list::Rect, crate::style::properties::computed::PixelLength)],
    crop: bool,
) -> Result<(), Error> {
    use lemon_pdf::outline::OutlineItem;

    let mut roots: Vec<OutlineItem> = Vec::new();
    let mut open: Vec<(u8, OutlineItem)> = Vec::new();
    fn close_to(roots: &mut Vec<OutlineItem>, open: &mut Vec<(u8, OutlineItem)>, level: u8) {
        while open.last().is_some_and(|(open_level, _)| *open_level >= level) {
            let (_, item) = open.pop().unwrap();
            match open.last_mut() {
                Some((_, parent)) => parent.children.push(item),
                None => roots.push(item),
            }
        }
    }
    for entry in entries {
        let Some(destination) = pdf_destination(&entry.destination, page_references, page_sizes, crop) else {
            continue;
        };
        let mut item = OutlineItem::new(entry.title.clone(), destination);
        item.open = !entry.closed;
        close_to(&mut roots, &mut open, entry.level);
        open.push((entry.level, item));
    }
    close_to(&mut roots, &mut open, 0);
    context.set_outlines(&roots)?;
    Ok(())
}

/// An operand that is written to the content stream as it is.
#[derive(Debug)]
struct RawPdf(String);

impl lemon_pdf::object::PdfFormat for RawPdf {
    fn write(&self, output: &mut lemon_pdf::object::Formatter) -> std::io::Result<()> {
        use std::io::Write;
        write!(output, "{}", self.0)
    }
}

/// The XMP packet of the document metadata (ISO 32000-1 14.3.2). A tagged document claims
/// conformance to PDF/UA-1 (`pdfuaid:part`, ISO 14289-1 5); PDF/UA also wants the title in
/// `dc:title` (7.1).
/// Constructs the path (without painting it) on the page.
pub(crate) fn emit_path(
    page_context: &mut lemon_pdf::content::PageContext<'_, '_, '_>,
    path: &[display_list::PathSegment],
    page_height: PixelLength,
) -> Result<(), Error> {
    use display_list::PathSegment;
    let pt = |p: display_list::Point| (Pt::from(p.x), Pt::from(page_height - p.y));
    for segment in path {
        match *segment {
            PathSegment::MoveTo(p) => {
                let (x, y) = pt(p);
                page_context.move_to(x, y)?
            }
            PathSegment::LineTo(p) => {
                let (x, y) = pt(p);
                page_context.line_to(x, y)?
            }
            PathSegment::CurveTo(c1, c2, end) => {
                let (x1, y1) = pt(c1);
                let (x2, y2) = pt(c2);
                let (x3, y3) = pt(end);
                page_context.curve_to(x1, y1, x2, y2, x3, y3)?
            }
            PathSegment::Close => page_context.close_path()?,
        }
    }
    Ok(())
}

/// The declaration of the schema of `pdfuaid:part` for the XMP of a PDF/A file (ISO 19005-2, 6.6.2.3).
const PDFUA_EXTENSION_SCHEMA: &str = "   <pdfaExtension:schemas>
    <rdf:Bag>
     <rdf:li rdf:parseType=\"Resource\">
      <pdfaSchema:schema>PDF/UA identification schema</pdfaSchema:schema>
      <pdfaSchema:namespaceURI>http://www.aiim.org/pdfua/ns/id/</pdfaSchema:namespaceURI>
      <pdfaSchema:prefix>pdfuaid</pdfaSchema:prefix>
      <pdfaSchema:property>
       <rdf:Seq>
        <rdf:li rdf:parseType=\"Resource\">
         <pdfaProperty:name>part</pdfaProperty:name>
         <pdfaProperty:valueType>Integer</pdfaProperty:valueType>
         <pdfaProperty:category>internal</pdfaProperty:category>
         <pdfaProperty:description>Indicates, which part of ISO 14289 standard is followed</pdfaProperty:description>
        </rdf:li>
       </rdf:Seq>
      </pdfaSchema:property>
     </rdf:li>
    </rdf:Bag>
   </pdfaExtension:schemas>
";

/// The namespaces of the XMP packet of a PDF/A file, on top of the others.
const PDFA_NAMESPACES: &str = "
    xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"
    xmlns:pdfaExtension=\"http://www.aiim.org/pdfa/ns/extension/\"
    xmlns:pdfaSchema=\"http://www.aiim.org/pdfa/ns/schema#\"
    xmlns:pdfaProperty=\"http://www.aiim.org/pdfa/ns/property#\"";

fn xmp_metadata(
    structure: &crate::tagging::StructureTree,
    date: Option<crate::metadata::Timestamp>,
    tagged: bool,
    pdfa: Option<PdfALevel>,
) -> String {
    let (title, lang, metadata) = (structure.title.as_deref(), structure.lang.as_deref(), &structure.metadata);
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }
    let mut properties = String::new();
    if let Some(title) = title {
        properties += &format!(
            "   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n",
            escape(title)
        );
    }
    if let Some(lang) = lang {
        properties += &format!("   <dc:language><rdf:Bag><rdf:li>{}</rdf:li></rdf:Bag></dc:language>\n", escape(lang));
    }
    if !metadata.authors.is_empty() {
        let authors: String = metadata.authors.iter().map(|author| format!("<rdf:li>{}</rdf:li>", escape(author))).collect();
        properties += &format!("   <dc:creator><rdf:Seq>{authors}</rdf:Seq></dc:creator>\n");
    }
    if let Some(description) = &metadata.description {
        properties += &format!(
            "   <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n",
            escape(description)
        );
    }
    if let Some(keywords) = &metadata.keywords {
        properties += &format!("   <pdf:Keywords>{}</pdf:Keywords>\n", escape(keywords));
    }
    if let Some(creator) = &metadata.creator {
        properties += &format!("   <xmp:CreatorTool>{}</xmp:CreatorTool>\n", escape(creator));
    }
    if let Some(date) = date {
        properties += &format!("   <xmp:CreateDate>{0}</xmp:CreateDate>\n   <xmp:ModifyDate>{0}</xmp:ModifyDate>\n", date.xmp());
    }
    properties += &format!(
        "   <pdf:Producer>Umbruch {}</pdf:Producer>\n",
        env!("CARGO_PKG_VERSION")
    );
    if tagged {
        properties += "   <pdfuaid:part>1</pdfuaid:part>\n";
    }
    if let Some(level) = pdfa {
        // ISO 19005-2 and -3, 6.6.4: the identification of the file
        properties += &format!(
            "   <pdfaid:part>{}</pdfaid:part>\n   <pdfaid:conformance>{}</pdfaid:conformance>\n",
            level.part, level.conformance
        );
        if tagged {
            // the properties of a schema that is not one of the predefined ones must be declared
            properties += PDFUA_EXTENSION_SCHEMA;
        }
    }
    let pdfa_namespaces = if pdfa.is_some() { PDFA_NAMESPACES } else { "" };
    format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
  <rdf:Description rdf:about=\"\"
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"
    xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\"{pdfa_namespaces}>
{properties}  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end=\"w\"?>"
    )
}
