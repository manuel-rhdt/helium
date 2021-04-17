use crate::layout::components::ZIndex;
use specs::{Entities, Entity, Join, ReadStorage, System, WriteStorage};

use super::components::{ContainingBlock, Position};

pub struct RelativePositioningSystem;

impl<'world> System<'world> for RelativePositioningSystem {
    type SystemData = (
        Entities<'world>,
        ReadStorage<'world, ContainingBlock>,
        WriteStorage<'world, Position>,
    );

    fn run(&mut self, (entities, parents, mut positions): Self::SystemData) {
        for (entity, _) in (&entities, parents.mask()).join() {
            update_position(entity, &parents, &mut positions);
        }
    }
}

fn update_position(
    entity: Entity,
    parents: &ReadStorage<ContainingBlock>,
    positions: &mut WriteStorage<Position>,
) {
    if let Some(parent) = parents.get(entity) {
        update_position(parent.entity, parents, positions);
        let parent_pos = *positions
            .get(parent.entity)
            .expect("The parent entity has no position.");
        let pos = positions
            .get_mut(entity)
            .expect("The child entity has no position.");

        pos.x += parent_pos.x;
        pos.y += parent_pos.y;
        pos.z = ZIndex(parent_pos.z.0 + 1);
        pos.page = parent_pos.page;
    }
}
