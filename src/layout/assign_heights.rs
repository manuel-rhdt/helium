use super::entity::{ContainingBlock, LayoutEntity, Position};
use crate::style::properties::computed::ComputedValues;
use crate::style::properties::computed::PixelLength as Px;

use specs::{Entities, Join, ReadStorage, System, WriteStorage};

pub struct AssignHeights;

impl<'a> System<'a> for AssignHeights {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, ComputedValues>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, ContainingBlock>,
    );

    fn run(
        &mut self,
        (entities, computed_values, mut positions, containing_block): Self::SystemData,
    ) {
        for (
            entity,
            style,
            &ContainingBlock {
                entity: LayoutEntity(container),
            },
        ) in (&entities, &computed_values, &containing_block).join()
        {
            let container_pos = positions.get(container).cloned().unwrap_or_default();
            if let Some(mut pos) =  positions.get_mut(entity) {
                pos.height = style
                    .position
                    .height
                    .absolute(Px(0.0), container_pos.height)
            }
        }
    }
}
