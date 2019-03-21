use super::block::BlockFormattingContext;
use super::entity::{Position, Text};
use super::text::layout_text;
use crate::style::properties::computed::ComputedValues;

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
        WriteStorage<'a, BlockFormattingContext>,
        WriteStorage<'a, Text>,
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
        ): Self::SystemData,
    ) {
        for (entity, style, inline_formatting_context) in
            (&entities, &style, &inline_formatting_context).join()
        {
            let &Position { x, y, width, .. } = positions.get(entity).unwrap();

            let mut lines = vec![];

            for child in inline_formatting_context.element_entities(&entities) {
                let content = text.get(child).unwrap().clone();
                let linebreaks = layout_text(&content.characters, style.font.font_size, width);

                let mut prev_break = 0;
                for break_point in linebreaks.into_iter().chain(Some(content.characters.len())) {
                    let line_content = &content.characters[prev_break..break_point];

                    let font_height = super::text::font_height(style.font.font_size);
                    let line_gap = style.font.line_height - font_height;

                    let line = entities
                        .build_entity()
                        .with(
                            Position {
                                x,
                                y,
                                height: style.font.line_height,
                                width,
                            },
                            &mut positions,
                        )
                        .with(
                            Text {
                                characters: line_content.to_owned(),
                                font_size: style.font.font_size,
                                baseline: super::text::font_ascender(style.font.font_size)
                                    + line_gap / 2.0,
                            },
                            &mut text,
                        )
                        .build();

                    lines.push(line.id());

                    prev_break = break_point;
                }
            }

            block_formatting_context
                .insert(entity, BlockFormattingContext { elements: lines })
                .unwrap();
        }
    }
}
