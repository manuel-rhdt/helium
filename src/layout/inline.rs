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
            let mut string = String::new();
            for child in inline_formatting_context.element_entities(&entities) {
                let text = text.get(child).unwrap();
                let _linebreaks = layout_text(&text.characters, style.font.font_size, width);
                string.push_str(&text.characters);
            }
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
                        characters: string,
                        font_size: style.font.font_size,
                        baseline: super::text::font_ascender(style.font.font_size) + line_gap / 2.0,
                    },
                    &mut text,
                )
                .build();
            block_formatting_context
                .insert(
                    entity,
                    BlockFormattingContext {
                        elements: vec![line.id()],
                    },
                )
                .unwrap();
        }
    }
}
