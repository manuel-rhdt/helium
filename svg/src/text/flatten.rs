// Copyright 2022 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::mem;
use std::sync::Arc;

use tiny_skia_path::{NonZeroRect, Transform};

use crate::text::cache::{FontCache, with_optical_sizing};
use crate::*;

fn resolve_rendering_mode(text: &Text) -> ShapeRendering {
    match text.rendering_mode {
        TextRendering::OptimizeSpeed => ShapeRendering::CrispEdges,
        TextRendering::OptimizeLegibility => ShapeRendering::GeometricPrecision,
        TextRendering::GeometricPrecision => ShapeRendering::GeometricPrecision,
    }
}

fn push_outline_paths(
    span: &layout::Span,
    builder: &mut tiny_skia_path::PathBuilder,
    new_children: &mut Vec<Node>,
    rendering_mode: ShapeRendering,
    abs_transform: Transform,
) {
    let builder = mem::replace(builder, tiny_skia_path::PathBuilder::new());

    if let Some(data) = builder.finish() {
        let path = Path::new(
            Box::default(),
            span.visible,
            span.fill.clone(),
            span.stroke.clone(),
            span.paint_order,
            rendering_mode,
            Arc::new(data),
            abs_transform,
        );
        new_children.push(Node::Path(Box::new(path)));
    }
}

pub(crate) fn flatten(text: &mut Text, fonts: &mut FontCache) -> Option<(Group, NonZeroRect)> {
    let mut new_children = vec![];

    let abs_transform = text.abs_transform;
    let rendering_mode = resolve_rendering_mode(text);

    for span in &text.layouted {
        if let Some(path) = span.overline.as_ref() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            new_children.push(Node::Path(path));
        }

        if let Some(path) = span.underline.as_ref() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            new_children.push(Node::Path(path));
        }

        // Instead of always processing each glyph separately, we always collect
        // as many outline glyphs as possible by pushing them into the span_builder
        // and only if we encounter a different glyph, or we reach the very end of the
        // span to we push the actual outline paths into new_children. This way, we don't need
        // to create a new path for every glyph if we have many consecutive glyphs
        // with just outlines (which is the most common case).
        let mut span_builder = tiny_skia_path::PathBuilder::new();

        let variations = with_optical_sizing(
            &span.variations,
            span.font_optical_sizing,
            span.font_size.get(),
        );

        for glyph in &span.positioned_glyphs {
            {
                let outline = fonts.outline(glyph.font, glyph.id, variations.clone());

                if let Some(outline) = outline.and_then(|p| p.transform(glyph.outline_transform()))
                {
                    span_builder.push_path(&outline);
                }
            }
        }

        push_outline_paths(
            span,
            &mut span_builder,
            &mut new_children,
            rendering_mode,
            abs_transform,
        );

        if let Some(path) = span.line_through.as_ref() {
            let mut path = path.clone();
            path.rendering_mode = rendering_mode;
            new_children.push(Node::Path(path));
        }
    }

    let mut group = Group {
        id: text.id.clone(),
        ..Group::empty()
    };

    for child in new_children {
        group.children.push(child);
    }

    let stroke_bbox = group.stroke_bounding_box()?.to_non_zero_rect()?;
    Some((group, stroke_bbox))
}
