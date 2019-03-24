use super::block_size::BlockSizeContext;
use super::entity::{ContainingBlock, Position};
use crate::style::device::Device;
use crate::style::properties::computed::ComputedValues;

use specs::{Entities, ReadStorage, System, WriteStorage};

#[derive(Debug)]
pub struct LayoutSizeComputation {
    pub inline_context: InlineSizeContext,
    pub block_context: BlockSizeContext,
}

impl LayoutSizeComputation {
    pub fn new(device: Device) -> Self {
        LayoutSizeComputation {
            inline_context: InlineSizeContext { device },
            block_context: BlockSizeContext {},
        }
    }
}

#[derive(Debug)]
pub struct InlineSizeContext {
    pub device: Device,
}

impl InlineSizeContext {
    pub fn new(device: Device) -> Self {
        InlineSizeContext { device }
    }
}

impl<'a> System<'a> for InlineSizeContext {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, ComputedValues>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, ContainingBlock>,
    );

    fn run(&mut self, (entities, computed_values, mut positions, containers): Self::SystemData) {
        use specs::Join;

        // We need a sorted list of all entities. We currently cheat by assuming
        // the list is correctly sorted given the order we construct the entities.
        let sorted_entities: Vec<_> = (&entities).join().collect();

        // calculate inline-sizes
        for &entity in sorted_entities.iter() {
            let container = match containers.get(entity) {
                Some(ContainingBlock { entity: container }) => container.0,
                None => continue,
            };
            let style = match computed_values.get(entity) {
                Some(style) => style,
                None => continue,
            };
            let container_width = positions
                .get(container)
                .expect("Container has no width")
                .width;

            let (inline_offset, inline_size) =
                super::block::calculate_inline_size(container_width, &style);
            positions.get_mut(entity).map(|pos| {
                pos.x = inline_offset;
                pos.width = inline_size;
            });
        }
    }
}
