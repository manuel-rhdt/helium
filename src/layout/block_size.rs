use super::block::BlockFormattingContext;
use super::entity::Position;
use crate::style::properties::computed::{ComputedValues, GenericSize};

use specs::{Entities, ReadStorage, System, WriteStorage};

#[derive(Debug)]
pub struct BlockSizeContext {}

impl<'a> System<'a> for BlockSizeContext {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, ComputedValues>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, BlockFormattingContext>,
    );

    fn run(&mut self, (entities, computed_values, mut positions, context): Self::SystemData) {
        use specs::Join;

        // We need a sorted list of all entities. We currently cheat by assuming
        // the list is correctly sorted given the order we construct the entities.
        let sorted_entities: Vec<_> = (&entities, &context, &computed_values).join().collect();

        for (entity, formatting_context, computed_values) in sorted_entities.iter().rev() {
            let mut block_size = 0.0;
            for child in formatting_context.element_entities(&entities) {
                let mut child_position = positions.get_mut(child).unwrap();
                child_position.y += block_size;
                block_size += child_position.height;
            }
            match computed_values.position.height {
                GenericSize::Auto => positions.get_mut(*entity).unwrap().height = block_size,
                _ => {}
            };
        }
    }
}
