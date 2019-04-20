use super::entity::Position;
use crate::style::properties::computed::PixelLength;

use specs::{Entities, ReadStorage, System, WriteStorage};

#[derive(Debug)]
pub struct BlockSizeContext {}

impl<'a> System<'a> for BlockSizeContext {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, BlockContainer>,
    );

    fn run(&mut self, (entities, mut positions, container): Self::SystemData) {
        use specs::Join;

        // We need a sorted list of all entities. We currently cheat by assuming
        // the list is correctly sorted given the order we construct the entities.
        let sorted_entities: Vec<_> = (&entities, &container).join().collect();

        for (entity, BlockContainer { elements }) in sorted_entities.iter().rev() {
            let first_child = elements.first();
            let last_child = elements.last();

            let start_y = first_child
                .map(|&x| positions.get(x).unwrap().y)
                .unwrap_or_default();
            let end_y = last_child
                .map(|&x| {
                    let pos = positions.get(x).unwrap();
                    pos.y + pos.height
                })
                .unwrap_or_default();

            let container_pos = positions.get_mut(*entity).unwrap();
            container_pos.y = start_y;
            if container_pos.height == PixelLength(0.0) {
                container_pos.height = end_y - start_y;
            }
        }
    }
}
