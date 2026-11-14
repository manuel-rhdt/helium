//! Paints gradients with PDF shadings.
//!
//! Linear gradients are axial shadings and radial gradients radial shadings, drawn with a
//! transformation that turns the unit circle into the ellipse of the gradient. Conic gradients
//! have no counterpart in PDF and are a mesh of Coons patches, one for every sector of a few
//! degrees. A gradient whose colour stops have different opacities is painted through a soft
//! mask that has the opacities as a shading of gray levels.

use std::io::{Error, ErrorKind};

use lemon_pdf::content::PageContext;
use lemon_pdf::form::TransparencyGroup;
use lemon_pdf::graphics_state::{ExtGState, SoftMask};
use lemon_pdf::image::ColorSpace;
use lemon_pdf::mesh::{CoonsMesh, Patch};
use lemon_pdf::shading::{
    AxialShading, Circle, Color, ColorStop, Gradient, Gray, Point, RadialShading, Rgb, Shading,
};
use lemon_pdf::Pt;

use super::RawPdf;
use crate::style::properties::computed::PixelLength;
use crate::style::properties::gradient::{Geometry, Resolved, ResolvedStop};

/// The width of the sectors of a conic gradient, in turns. Each sector is straight and has
/// colours that are linear in the angle, so this is the accuracy of the gradient.
const CONIC_SECTORS: usize = 180;

fn invalid(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::new(ErrorKind::InvalidData, error)
}

/// A shading with the transformation that has to be applied before painting it.
struct Built {
    shading: Shading,
    /// `[a b c d e f]` for the `cm` operator.
    matrix: Option<[f64; 6]>,
}

/// Maps layout coordinates (in pixels, y pointing down) to PDF user space.
#[derive(Copy, Clone)]
struct Space {
    page_height: PixelLength,
}

impl Space {
    fn x(self, x: f32) -> f64 {
        Pt::from(PixelLength(x)).0
    }

    fn y(self, y: f32) -> f64 {
        Pt::from(self.page_height - PixelLength(y)).0
    }

    fn length(self, length: f32) -> f64 {
        Pt::from(PixelLength(length)).0
    }
}

/// The shading of `gradient` with the colours that `to_color` makes of the stops.
fn build<C: Color>(
    context: &mut PageContext<'_, '_, '_>,
    space: Space,
    gradient: &Resolved,
    to_color: impl Fn([f32; 4]) -> C,
) -> Result<Built, Error> {
    if let Geometry::Conic { .. } = gradient.geometry {
        return Ok(Built { shading: conic(context, space, gradient, to_color)?, matrix: None });
    }
    let stops: Vec<ColorStop<C>> = gradient
        .stops
        .iter()
        .map(|stop| ColorStop::new(f64::from(stop.offset), to_color(stop.color)))
        .collect();
    let function = Gradient::new(&stops).map_err(invalid)?.extended();
    match gradient.geometry {
        Geometry::Linear { start, end } => {
            let shading = AxialShading::new(
                Point::new(space.x(start.0), space.y(start.1)),
                Point::new(space.x(end.0), space.y(end.1)),
                &function,
            );
            Ok(Built { shading: shading.into(), matrix: None })
        }
        Geometry::Radial { center, radius } => {
            // the unit circle, scaled to the ellipse
            let circle = |radius| Circle { x: Pt(0.0), y: Pt(0.0), radius: Pt(radius) };
            let shading = RadialShading::new(circle(0.0), circle(1.0), &function);
            Ok(Built {
                shading: shading.into(),
                matrix: Some([
                    space.length(radius.0),
                    0.0,
                    0.0,
                    space.length(radius.1),
                    space.x(center.0),
                    space.y(center.1),
                ]),
            })
        }
        Geometry::Conic { .. } => unreachable!(),
    }
}

/// The colours at the start and the end of the part of the gradient between offsets `a` and
/// `b`, which has no stop in its inside.
fn colors_between(stops: &[ResolvedStop], a: f32, b: f32) -> ([f32; 4], [f32; 4]) {
    let middle = (a + b) / 2.0;
    let first = stops[0];
    let last = stops[stops.len() - 1];
    if middle <= first.offset {
        return (first.color, first.color);
    }
    if middle >= last.offset {
        return (last.color, last.color);
    }
    let index = stops
        .windows(2)
        .position(|pair| pair[0].offset <= middle && middle < pair[1].offset)
        .unwrap_or(0);
    let (from, to) = (stops[index], stops[index + 1]);
    let at = |offset: f32| {
        let t = ((offset - from.offset) / (to.offset - from.offset)).clamp(0.0, 1.0);
        let mut color = [0.0; 4];
        for i in 0..4 {
            color[i] = from.color[i] + (to.color[i] - from.color[i]) * t;
        }
        color
    };
    (at(a), at(b))
}

/// The mesh of a conic gradient.
fn conic<C: Color>(
    context: &mut PageContext<'_, '_, '_>,
    space: Space,
    gradient: &Resolved,
    to_color: impl Fn([f32; 4]) -> C,
) -> Result<Shading, Error> {
    let Geometry::Conic { center, from, radius } = gradient.geometry else {
        unreachable!("only conic gradients are meshes")
    };

    // The boundaries of the sectors: regularly spaced and at every stop, so that the colour is
    // linear inside of a sector.
    let mut offsets: Vec<f32> =
        (0..=CONIC_SECTORS).map(|i| i as f32 / CONIC_SECTORS as f32).collect();
    offsets.extend(gradient.stops.iter().map(|stop| stop.offset).filter(|o| (0.0..=1.0).contains(o)));
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap());
    offsets.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    let rim = |offset: f32| {
        let angle = (from + 360.0 * offset).to_radians();
        (center.0 + radius * angle.sin(), center.1 - radius * angle.cos())
    };
    let point = |(x, y): (f32, f32)| Point::new(space.x(x), space.y(y));
    // the point at `t` of the way from `a` to `b`
    let between = |t: f32, a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);

    let mut mesh: Option<CoonsMesh<C>> = None;
    for pair in offsets.windows(2) {
        let (color_a, color_b) = colors_between(&gradient.stops, pair[0], pair[1]);
        let (rim_a, rim_b) = (rim(pair[0]), rim(pair[1]));
        // One sector is a patch with two straight sides along radii, the arc as its third side
        // and the center as a side of zero length. The two corners on a radius have the same
        // colour, so the colour does not change along the radius.
        let points = [
            point(center),
            point(between(1.0 / 3.0, center, rim_a)),
            point(between(2.0 / 3.0, center, rim_a)),
            point(rim_a),
            point(between(1.0 / 3.0, rim_a, rim_b)),
            point(between(2.0 / 3.0, rim_a, rim_b)),
            point(rim_b),
            point(between(2.0 / 3.0, center, rim_b)),
            point(between(1.0 / 3.0, center, rim_b)),
            point(center),
            point(center),
            point(center),
        ];
        let patch = Patch {
            points,
            colors: [to_color(color_a), to_color(color_a), to_color(color_b), to_color(color_b)],
        };
        mesh = Some(match mesh {
            None => CoonsMesh::new(patch),
            Some(mesh) => mesh.patch(patch),
        });
    }
    let mesh = mesh.expect("there are at least two offsets");
    context.pdf_context.write_mesh_shading(&mesh).map_err(|error| Error::new(ErrorKind::InvalidData, error.to_string()))
}

/// Paints a shading over the current clip. `artifact` marks it as an artifact of the page, which
/// content in a form that is only a mask does not need.
fn paint_built(context: &mut PageContext<'_, '_, '_>, built: Built, artifact: bool) -> Result<(), Error> {
    if artifact {
        context.push_operand(RawPdf("/Artifact".into()))?;
        context.apply_operator("BMC")?;
    }
    context.save_graphics_state()?;
    if let Some([a, b, c, d, e, f]) = built.matrix {
        context.concatenate_matrix(a, b, c, d, Pt(e), Pt(f))?;
    }
    let key = context.add_shading(built.shading);
    context.paint_shading(&key)?;
    context.restore_graphics_state()?;
    if artifact {
        context.apply_operator("EMC")?;
    }
    Ok(())
}

/// Paints `gradient` over the current clip.
pub fn paint(
    context: &mut PageContext<'_, '_, '_>,
    gradient: &Resolved,
    page_height: PixelLength,
) -> Result<(), Error> {
    let space = Space { page_height };
    let alphas = gradient.stops.iter().map(|stop| stop.color[3]);
    let min_alpha = alphas.clone().fold(f32::INFINITY, f32::min);
    let max_alpha = alphas.fold(f32::NEG_INFINITY, f32::max);

    let color = build(context, space, gradient, |c| Rgb(f64::from(c[0]), f64::from(c[1]), f64::from(c[2])))?;
    if max_alpha - min_alpha < 1e-4 {
        // one opacity for the whole gradient
        if max_alpha < 1.0 {
            context.set_fill_alpha(f64::from(max_alpha))?;
        }
    } else {
        // The opacities are a second gradient, of gray levels, which is the soft mask. It
        // covers the box that the gradient is painted into.
        let paint = gradient.paint;
        let bbox = [
            Pt(space.x(paint.x)),
            Pt(space.y(paint.y + paint.height)),
            Pt(space.x(paint.x + paint.width)),
            Pt(space.y(paint.y)),
        ];
        let mask = context.pdf_context.write_form(
            bbox,
            Some(TransparencyGroup::new(ColorSpace::DeviceGray)),
            None,
            |form| {
                let alpha = build(form, space, gradient, |c| Gray(f64::from(c[3])))?;
                paint_built(form, alpha, false)
            },
        )?;
        context.apply_ext_gstate(ExtGState::new().with_soft_mask(SoftMask::luminosity(mask)))?;
    }
    paint_built(context, color, true)
}
