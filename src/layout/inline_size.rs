use super::block_size::BlockSizeContext;
use super::entity::{ContainingBlock, Position};
use crate::style::device::Device;
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength as Px};

use specs::{Entities, Entity, ReadStorage, System, WriteStorage};

use shred_derive::SystemData;

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

#[derive(SystemData)]
pub struct InlineSizeResources<'a> {
    pub entities: Entities<'a>,
    pub computed_values: ReadStorage<'a, ComputedValues>,
    pub positions: WriteStorage<'a, Position>,
    pub containing_blocks: ReadStorage<'a, ContainingBlock>,
}

#[derive(Debug)]
pub struct InlineSizeContext {
    pub device: Device,
}

impl InlineSizeContext {
    pub fn new(device: Device) -> Self {
        InlineSizeContext { device }
    }

    pub fn get_container_width(
        &self,
        entity: Entity,
        resources: &InlineSizeResources,
    ) -> Option<Px> {
        let container = resources.containing_blocks.get(entity)?.entity.0;
        let container_width = resources.positions.get(container)?.width;

        Some(container_width)
    }

    pub fn set_inline_pos(
        &self,
        entity: Entity,
        offset: Px,
        width: Px,
        resources: &mut InlineSizeResources,
    ) {
        resources.positions.get_mut(entity).map(|pos| {
            pos.x = offset;
            pos.width = width;
        });
    }
}

impl<'a> System<'a> for InlineSizeContext {
    type SystemData = InlineSizeResources<'a>;

    fn run(&mut self, mut resources: Self::SystemData) {
        use specs::Join;

        // We need a sorted list of all entities. We currently cheat by assuming
        // the list is correctly sorted given the order we construct the entities.
        let sorted_entities: Vec<_> = resources.entities.join().collect();

        // calculate inline-sizes
        for &entity in sorted_entities.iter() {
            let style = resources.computed_values.get(entity);
            let container_width = self.get_container_width(entity, &resources);

            let (inline_offset, inline_size) = match (style, container_width) {
                (Some(style), Some(container_width)) => {
                    calculate_inline_size(container_width, &style)
                }
                _ => continue,
            };

            self.set_inline_pos(entity, inline_offset, inline_size, &mut resources);
        }
    }
}

/// Does the constraint resolution needed to calculate the inline size of an entity
/// and its inline offset from the content edge of its container.
///
/// Returns a tuple where the first value is the inline offset and the second value
/// the inline size.
///
/// https://www.w3.org/TR/CSS22/visudet.html#blockwidth
pub fn calculate_inline_size(
    container_size: Px,
    ComputedValues { position, font, .. }: &ComputedValues,
) -> (Px, Px) {
    let font_size = font.font_size;
    let (margin_left, width) = match position.width {
        GenericSize::Auto => {
            let margin_left = position
                .margin_left
                .absolute(Px(0.0), container_size, font_size);
            let margin_right = position
                .margin_right
                .absolute(Px(0.0), container_size, font_size);
            let width = (container_size - margin_left - margin_right).max(Px(0.0));
            (margin_left, width)
        }
        GenericSize::LengthPercentage(lp) => {
            let width = lp.absolute(container_size, font_size);
            let margin_left = match (position.margin_left, position.margin_right) {
                (GenericSize::Auto, GenericSize::Auto) => {
                    // center the child horizontally within its container
                    (container_size - width) / 2.0
                }
                (GenericSize::LengthPercentage(val), GenericSize::Auto) => {
                    val.absolute(container_size, font_size)
                }
                (GenericSize::Auto, GenericSize::LengthPercentage(val)) => {
                    let margin_right = val.absolute(container_size, font_size);
                    container_size - width - margin_right
                }
                (
                    GenericSize::LengthPercentage(margin_left),
                    GenericSize::LengthPercentage(_margin_right),
                ) => {
                    // over constrained (we use the specified margin_left and width, ignoring margin_right)
                    margin_left.absolute(container_size, font_size)
                }
            };

            (margin_left, width)
        }
    };
    let border_left = position.border_left_width.absolute(font_size);
    let border_right = position.border_right_width.absolute(font_size);

    let padding_left = position.padding_left.absolute(container_size, font_size);
    let padding_right = position.padding_right.absolute(container_size, font_size);

    let offset = margin_left + border_left + padding_left;
    let width = width - border_left - padding_left - padding_right - border_right;

    (offset, width)
}
