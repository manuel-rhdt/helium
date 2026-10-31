use super::{
    components::{
        ContainingBlock, Decoration, Glyphs, GraphicsOp, IntrinsicSize, MathBox, MathExpression,
        Paragraph, Position, TextOperator, TextRun, TransformMatrix, VectorGraphics, ZIndex,
    },
    inline_size::calculate_padding_and_border,
    text::{font::GlyphIndex, Font},
    FontMap,
};
use math_render::{
    layout_with_style,
    math_box::{Drawable, MathBoxContent, MathBoxMetrics},
    shaper::HarfbuzzShaper,
};
use specs::{Entities, Entity, Join, Read, ReadStorage, System, WriteStorage};

use crate::style::properties::computed::{color_to_rgba, ComputedValues, MathStyle, PixelLength};
use std::collections::BTreeMap;

pub struct MathFormat {}

impl<'world> System<'world> for MathFormat {
    type SystemData = (
        Read<'world, FontMap>,
        Entities<'world>,
        WriteStorage<'world, Position>,
        ReadStorage<'world, MathExpression>,
        ReadStorage<'world, ComputedValues>,
        WriteStorage<'world, IntrinsicSize>,
        WriteStorage<'world, MathBox>,
        WriteStorage<'world, ContainingBlock>,
        WriteStorage<'world, Decoration>,
    );
    fn run(
        &mut self,
        (
            font_map,
            entities,
            mut positions,
            math_expressions,
            computed_values,
            mut intrinsic_sizes,
            mut math_boxes,
            mut parents,
            mut decorations,
        ): Self::SystemData,
    ) {
        for (entity, expr, style) in (&entities, &math_expressions, &computed_values).join() {
            let font = &(*font_map)[&expr.font_name];
            let font_scale = PixelLength(style.font.font_size.0 / (font.upem as f32));

            let scale = |val| PixelLength(val as f32 / font.upem as f32 * style.font.font_size.0);

            let shaper = HarfbuzzShaper::new(font.hb_font.clone());
            let result = layout_with_style(&expr.expression, &shaper, |default, user_data| {
                let entity = entities.entity(user_data as _);
                let style = computed_values.get(entity);
                if let Some(style) = style {
                    if style.math.math_style == MathStyle::Inline {
                        default.inline_style()
                    } else {
                        default.display_style()
                    }
                } else {
                    default
                }
            });

            process_mathbox(
                &result,
                entity,
                scale(result.extents().ascent),
                font_scale,
                &entities,
                &mut positions,
                &mut parents,
                &mut decorations,
                &computed_values,
            );

            let size = IntrinsicSize::SizeWithBaseline {
                width: scale(result.advance_width()),
                height: scale(result.extents().height()),
                baseline: scale(result.extents().ascent),
            };
            intrinsic_sizes.insert(entity, size).unwrap();

            math_boxes
                .insert(
                    entity,
                    MathBox {
                        math_box: result,
                        font_name: expr.font_name.clone(),
                        font_size: style.font.font_size,
                        font_scale,
                    },
                )
                .unwrap();
        }
    }
}

pub fn process_mathbox(
    math_box: &math_render::math_box::MathBox,
    parent: Entity,
    baseline: PixelLength,
    font_scale: PixelLength,
    entities: &Entities,
    positions: &mut WriteStorage<Position>,
    parents: &mut WriteStorage<ContainingBlock>,
    decorations: &mut WriteStorage<Decoration>,
    computed_values: &ReadStorage<ComputedValues>,
) {
    let entity = entities.entity(math_box.user_data() as _);

    let scale = |x| x as f32 * font_scale;

    let extents = math_box.extents();

    let x = scale(math_box.origin.x + extents.left_side_bearing);
    let y = baseline + scale(math_box.origin.y - extents.ascent);
    let width = scale(extents.width);
    let height = scale(extents.height());

    let pos = Position {
        x,
        y,
        width,
        height,
        z: ZIndex(0),
        page: 0,
    };
    positions.insert(entity, pos).unwrap();
    if entity != parent {
        parents
            .insert(entity, ContainingBlock { entity: parent })
            .unwrap();
        let decoration = calculate_padding_and_border(width, computed_values.get(entity).unwrap());
        decorations.insert(entity, decoration).unwrap();
    }

    if let MathBoxContent::Boxes(list) = math_box.content() {
        for child in list {
            process_mathbox(
                child,
                parent,
                baseline,
                font_scale,
                entities,
                positions,
                parents,
                decorations,
                computed_values,
            );
        }
    }
}

pub struct LayoutMathGlyphs {}

impl LayoutMathGlyphs {
    fn generate_elements(
        &self,
        math_box: &math_render::math_box::MathBox,
        para: &mut Paragraph,
        graphics: &mut VectorGraphics,
        font: &Font,
        x: PixelLength,
        y: PixelLength,
        font_size: PixelLength,
        text_runs: &ReadStorage<'_, TextRun>,
        entities: &Entities<'_>,
        to_unicode_map: &mut BTreeMap<GlyphIndex, String>,
    ) {
        let scale = |val| val as f32 / font.upem as f32 * font_size;
        let content = math_box.content();

        match content {
            MathBoxContent::Boxes(list) => {
                for item in list {
                    let x_offset = scale(item.origin.x);
                    let y_offset = scale(item.origin.y);
                    self.generate_elements(
                        item,
                        para,
                        graphics,
                        font,
                        x + x_offset,
                        y + y_offset,
                        font_size,
                        text_runs,
                        entities,
                        to_unicode_map,
                    );
                }
                // let pt = math_box.origin;
                // if pt.x == 0 && pt.y == 0 {
                //     for item in list.as_slice() {
                //         generate_svg(node, item, func);
                //     }
                //     return;
                // }
                // let mut group =
                //     Group::new().set("transform", format!("translate({:?}, {:?})", pt.x, pt.y));
                // for item in list.as_slice() {
                //     generate_svg(&mut group, item, func);
                // }
                // node.append(group);
            }
            MathBoxContent::Drawable(drawable) => match drawable {
                Drawable::Glyphs {
                    glyphs,
                    scale: scale_factor,
                } => {
                    let mut previous_cluster = 0xFFFF;
                    let mut cluster_starting_glyph = 0xFFFF;

                    let text = text_runs
                        .get(entities.entity(math_box.user_data() as _))
                        .map(|TextRun { text, .. }| text);

                    let base_transform = TransformMatrix::translation(x, y)
                        .inv_apply(TransformMatrix::uniform_scale(scale_factor.as_scale_mult()));
                    for (advance, glyph) in glyphs.iter().scan(0, |advance, glyph| {
                        let previous_advance = scale(*advance);
                        *advance += glyph.advance_width();
                        Some((previous_advance, glyph))
                    }) {
                        let (x_glyph_offset, y_glyph_offset) = (glyph.offset.x, glyph.offset.y);
                        let translate = TransformMatrix::translation(
                            advance + scale(x_glyph_offset),
                            scale(y_glyph_offset),
                        );
                        let transform = base_transform.inv_apply(translate);
                        para.ops.push(TextOperator::SetTransform(transform));
                        let glyph_index = GlyphIndex(glyph.glyph_code);
                        let _ = font.advance(glyph_index);
                        let op = TextOperator::Glyphs(Glyphs(vec![glyph_index]));
                        para.ops.push(op);

                        if let Some(text) = text {
                            if glyph.cluster != previous_cluster {
                                // a new cluster began
                                if previous_cluster < glyph.cluster {
                                    let text =
                                        &text[previous_cluster as usize..glyph.cluster as usize];
                                    to_unicode_map.insert(
                                        GlyphIndex(cluster_starting_glyph),
                                        text.to_owned(),
                                    );
                                }
                                cluster_starting_glyph = glyph.glyph_code;
                                previous_cluster = glyph.cluster;
                            }
                        }
                    }

                    if let Some(text) = text {
                        if let Some(text) = text.get(previous_cluster as usize..) {
                            to_unicode_map
                                .insert(GlyphIndex(cluster_starting_glyph), text.to_owned());
                        }
                    }
                }
                Drawable::Line { vector, thickness } => {
                    let to_x = x + scale(vector.x);
                    let to_y = y + scale(vector.y);
                    graphics.ops.push(GraphicsOp::Line {
                        from_x: x,
                        from_y: y,
                        to_x,
                        to_y,
                        thickness: scale(*thickness as i32),
                    })
                }
            },
            _ => {} // _ => func(node, math_box),
        }
    }
}

impl<'world> System<'world> for LayoutMathGlyphs {
    type SystemData = (
        Entities<'world>,
        ReadStorage<'world, MathBox>,
        ReadStorage<'world, Position>,
        WriteStorage<'world, Paragraph>,
        WriteStorage<'world, VectorGraphics>,
        ReadStorage<'world, TextRun>,
        ReadStorage<'world, ComputedValues>,
        Read<'world, FontMap>,
    );
    fn run(
        &mut self,
        (entities, math_boxes, positions, mut paragraphs, mut vector_graphics, text_runs, computed_values, font_map): Self::SystemData,
    ) {
        for (entity, math_box, pos) in (&entities, &math_boxes, &positions).join() {
            let mut para = Paragraph::new(PixelLength(0.0), PixelLength(0.0));
            let mut vg = VectorGraphics::default();
            let font = &font_map[&math_box.font_name];
            para.ops.push(TextOperator::SetFont(
                math_box.font_name.clone(),
                math_box.font_size,
            ));
            if let Some(style) = computed_values.get(entity) {
                para.ops.push(TextOperator::SetColor(color_to_rgba(style.font.color)));
            }
            let scale = |val| val as f32 / font.upem as f32 * math_box.font_size;
            let mut to_unicode_map = BTreeMap::new();
            self.generate_elements(
                &math_box.math_box,
                &mut para,
                &mut vg,
                font,
                pos.x,
                pos.y + scale(math_box.math_box.extents().ascent),
                math_box.font_size,
                &text_runs,
                &entities,
                &mut to_unicode_map,
            );
            font.to_unicode.lock().append(&mut to_unicode_map);
            paragraphs.insert(entity, para).unwrap();
            vector_graphics.insert(entity, vg).unwrap();
        }
    }
}
