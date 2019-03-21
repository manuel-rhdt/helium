use super::block::BlockFormattingContext;
use super::entity::{Position, StackingContextRelativePosition};

use specs::{Entities, ReadStorage, System, WriteStorage};

#[derive(Debug)]
pub struct GlobalTransform;

impl<'a> System<'a> for GlobalTransform {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, BlockFormattingContext>,
        ReadStorage<'a, Position>,
        WriteStorage<'a, StackingContextRelativePosition>,
    );

    fn run(&mut self, (entities, formatting_context, pos, mut global_pos): Self::SystemData) {
        use specs::Join;

        for (self_entity, formatting_context) in (&entities, &formatting_context).join() {
            let StackingContextRelativePosition {
                x: global_x,
                y: global_y,
            } = *global_pos.entry(self_entity).unwrap().or_insert(Default::default());
            for child_entity in formatting_context.element_entities(&entities) {
                let Position {
                    x: rel_x, y: rel_y, ..
                } = *pos.get(child_entity).unwrap();
                global_pos
                    .insert(
                        child_entity,
                        StackingContextRelativePosition {
                            x: global_x + rel_x,
                            y: global_y + rel_y,
                        },
                    )
                    .unwrap();
            }
        }
    }
}
