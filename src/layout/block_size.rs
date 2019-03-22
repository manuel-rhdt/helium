use super::block::BlockFormattingContext;
use super::entity::Position;
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength};

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

        for (entity, formatting_context, style) in sorted_entities.iter().rev() {
            let mut block_size = PixelLength(0.0);
            for child in formatting_context.element_entities(&entities) {
                let child_position = positions.get_mut(child).unwrap();
                let child_style = computed_values.get(child).cloned().unwrap_or_default();

                let margin_top = match child_style.position.margin_top {
                    GenericSize::Auto => PixelLength(0.0),
                    GenericSize::LengthPercentage(lp) => {
                        lp.absolute(child_position.width, child_style.font.font_size)
                    }
                };

                let margin_bottom = match child_style.position.margin_bottom {
                    GenericSize::Auto => PixelLength(0.0),
                    GenericSize::LengthPercentage(lp) => {
                        lp.absolute(child_position.width, child_style.font.font_size)
                    }
                };

                child_position.y += block_size + margin_top;

                block_size += child_position.height + margin_top + margin_bottom;
            }
            match style.position.height {
                GenericSize::Auto => positions.get_mut(*entity).unwrap().height = block_size,
                _ => {}
            };
        }
    }
}
