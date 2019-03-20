use super::block::BlockFormattingContext;
use super::entity::{Position, Text};
use super::text::layout_text;

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
        WriteStorage<'a, Position>,
        ReadStorage<'a, InlineFormattingContext>,
        WriteStorage<'a, BlockFormattingContext>,
        WriteStorage<'a, Text>,
    );

    fn run(
        &mut self,
        (
            entities,
            mut positions,
            inline_formatting_context,
            mut block_formatting_context,
            mut text,
        ): Self::SystemData,
    ) {
        for (entity, inline_formatting_context) in (&entities, &inline_formatting_context).join() {
            let self_width = positions.get(entity).unwrap().width;
            for _child in inline_formatting_context.element_entities(&entities) {
                let _linebreaks =
                    layout_text("Lorem ipsum dolor", self_width);
            }
            let line_height = super::text::line_height();
            let line = entities
                .build_entity()
                .with(
                    Position {
                        height: line_height,
                        width: self_width,
                        ..Default::default()
                    },
                    &mut positions,
                )
                .with(
                    Text {
                        characters: "Lorem ipsum".to_string(),
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
