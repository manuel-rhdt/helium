use super::{
    components::{
        Glyphs, GraphicsOp, MathBox, MathExpression, Paragraph, Position, TextOperator,
        TransformMatrix, VectorGraphics,
    },
    text::{font::GlyphIndex, Font},
    FontMap,
};
use math_render::{
    layout,
    math_box::{Drawable, MathBoxContent, MathBoxMetrics},
    shaper::HarfbuzzShaper,
};
use specs::{Join, Read, ReadStorage, System, WriteStorage};

use crate::style::properties::computed::{ComputedValues, PixelLength};

pub struct MathFormat {}

impl<'world> System<'world> for MathFormat {
    type SystemData = (
        Read<'world, FontMap>,
        ReadStorage<'world, MathExpression>,
        ReadStorage<'world, ComputedValues>,
        WriteStorage<'world, Position>,
        WriteStorage<'world, MathBox>,
    );
    fn run(
        &mut self,
        (font_map, math_expressions, computed_values, mut positions, mut math_boxes): Self::SystemData,
    ) {
        for (expr, style, pos, math_box) in (
            &math_expressions,
            &computed_values,
            positions.entries(),
            math_boxes.entries(),
        )
            .join()
        {
            let font = &(*font_map)[&expr.font_name];

            let scale = |val| val as f32 / font.upem() as f32 * style.font.font_size;

            let shaper = HarfbuzzShaper::new(font.hb_font.clone());
            let result = layout(&expr.expression, &shaper);

            let pos = pos.or_insert(Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(0.0),
                height: PixelLength(0.0),
                z: Default::default(),
                page: 0,
            });

            pos.width = scale(result.extents().width);
            pos.height = scale(result.extents().height());

            math_box.replace(MathBox {
                math_box: result,
                font_name: expr.font_name.clone(),
                font_size: style.font.font_size,
            });
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
    ) {
        let scale = |val| val as f32 / font.upem() as f32 * font_size;
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
                Drawable::Glyph(glyph) => {
                    let (x_glyph_offset, y_glyph_offset) = (glyph.offset.x, glyph.offset.y);
                    let translate = TransformMatrix::translation(
                        x + scale(x_glyph_offset),
                        y + scale(y_glyph_offset),
                    );
                    para.ops.push(TextOperator::SetTransform(translate));
                    let glyph_index = GlyphIndex(glyph.glyph_code);
                    let _ = font.advance(glyph_index);
                    let op = TextOperator::Glyphs(Glyphs(vec![glyph_index]));
                    para.ops.push(op);
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
        ReadStorage<'world, MathBox>,
        ReadStorage<'world, Position>,
        WriteStorage<'world, Paragraph>,
        WriteStorage<'world, VectorGraphics>,
        Read<'world, FontMap>,
    );
    fn run(
        &mut self,
        (math_boxes, positions, mut paragraphs, mut vector_graphics, font_map): Self::SystemData,
    ) {
        for (math_box, pos, paragraph, vector_graphic) in (
            &math_boxes,
            &positions,
            paragraphs.entries(),
            vector_graphics.entries(),
        )
            .join()
        {
            let mut para = Paragraph::default();
            let mut vg = VectorGraphics::default();
            let font = &font_map[&math_box.font_name];
            para.ops.push(TextOperator::SetFont(
                math_box.font_name.clone(),
                math_box.font_size,
            ));
            let scale = |val| val as f32 / font.upem() as f32 * math_box.font_size;
            self.generate_elements(
                dbg!(&math_box.math_box),
                &mut para,
                &mut vg,
                font,
                pos.x,
                pos.y + scale(math_box.math_box.extents().ascent),
                math_box.font_size,
            );
            paragraph.replace(para);
            vector_graphic.replace(vg);
        }
    }
}
