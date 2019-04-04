use super::block::BlockContainer;
use super::entity::{Position, Text, UnscannedText};
use super::text::{layout_text, font::match_font};
use crate::style::properties::computed::{ComputedValues, LineHeight, PixelLength as Px};

use specs::{
    world::{EntitiesRes, Index},
    Component, Entity, VecStorage,
};
use specs::{Entities, Join, ReadStorage, System, WriteStorage};

#[derive(Debug, Clone)]
pub struct InlineFormattingContext {
    pub elements: Vec<Index>,
}

impl InlineFormattingContext {
    pub fn element_entities<'a>(
        &'a self,
        entities_res: &'a EntitiesRes,
    ) -> impl 'a + Iterator<Item = Entity> {
        self.elements.iter().filter_map(move |&index| {
            let entity = entities_res.entity(index);
            if entities_res.is_alive(entity) {
                Some(entity)
            } else {
                None
            }
        })
    }
}

impl Component for InlineFormattingContext {
    type Storage = VecStorage<Self>;
}

pub struct InlineLayout;

impl<'a> System<'a> for InlineLayout {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, ComputedValues>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, InlineFormattingContext>,
        WriteStorage<'a, BlockContainer>,
        WriteStorage<'a, Text>,
        WriteStorage<'a, UnscannedText>,
    );

    fn run(
        &mut self,
        (
            entities,
            style,
            mut positions,
            inline_formatting_context,
            mut block_formatting_context,
            mut text,
            mut unscanned_text,
        ): Self::SystemData,
    ) {
        for (entity, style, inline_formatting_context) in
            (&entities, &style, &inline_formatting_context).join()
        {
            let &Position { x, width, .. } = positions.get(entity).unwrap();

            let mut lines = vec![];

            for child in inline_formatting_context.element_entities(&entities) {
                let mut content = unscanned_text.get_mut(child).unwrap().clone();
                let font = match_font(&style).unwrap();
                let layout_lines =
                    layout_text(&mut content.characters, &font, style.font.font_size, width);

                for line_content in layout_lines {
                    let font_height = super::text::font_height(style.font.font_size);
                    let line_height = match style.font.line_height {
                        LineHeight::Normal => font_height * 1.2,
                        LineHeight::LengthPercentage(lp) => {
                            lp.absolute(style.font.font_size, style.font.font_size)
                        }
                    };
                    let line_gap = line_height - font_height;

                    let line = entities
                        .build_entity()
                        .with(
                            Position {
                                x,
                                y: Px(0.0),
                                height: line_height,
                                width,
                            },
                            &mut positions,
                        )
                        .with(
                            Text {
                                glyphs: line_content.to_owned(),
                                font,
                                font_size: style.font.font_size,
                                baseline: super::text::font_ascender(style.font.font_size)
                                    + line_gap / 2.0,
                            },
                            &mut text,
                        )
                        .build();

                    lines.push(line);
                }
            }

            block_formatting_context
                .insert(entity, BlockContainer { elements: lines })
                .unwrap();
        }
    }
}
