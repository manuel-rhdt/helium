use super::entity::{ContainingBlock, Decoration, Position};
use crate::style::device::Device;
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength as Px};

use specs::{Entities, Entity, ReadStorage, System, WriteStorage};

use shred_derive::SystemData;

#[derive(SystemData)]
pub struct InlineSizeResources<'a> {
    pub entities: Entities<'a>,
    pub computed_values: ReadStorage<'a, ComputedValues>,
    pub positions: WriteStorage<'a, Position>,
    pub containing_blocks: ReadStorage<'a, ContainingBlock>,
    pub decorations: WriteStorage<'a, Decoration>,
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

            match (style, container_width) {
                (Some(style), Some(container_width)) => {
                    let mut decoration = calculate_padding_and_border(container_width, &style);
                    let inline_size =
                        calculate_inline_size(container_width, &mut decoration, &style);
                    let inline_offset =
                        decoration.margin.left + decoration.border.left + decoration.padding.left;

                    self.set_inline_pos(entity, inline_offset, inline_size, &mut resources);

                    if decoration != Default::default() {
                        resources.decorations.insert(entity, decoration).unwrap();
                    }
                }
                _ => {}
            };
        }
    }
}

pub fn calculate_padding_and_border(
    container_size: Px,
    ComputedValues {
        position,
        background,
        ..
    }: &ComputedValues,
) -> Decoration {
    let mut deco = Decoration::default();

    deco.border.top = position.border_top_width;
    deco.border.bottom = position.border_bottom_width;
    deco.border.left = position.border_left_width;
    deco.border.right = position.border_right_width;

    deco.padding.top = position.padding_top.absolute(container_size);
    deco.padding.bottom = position.padding_bottom.absolute(container_size);
    deco.padding.left = position.padding_left.absolute(container_size);
    deco.padding.right = position.padding_right.absolute(container_size);

    // margin_left and margin_right get calculated in `calculate_inline_size`
    deco.margin.top = position.margin_top.absolute(Px(0.0), container_size);
    deco.margin.bottom = position.margin_bottom.absolute(Px(0.0), container_size);

    use cssparser::{Color, RGBA};
    deco.background_color = match background.background_color {
        // TODO:
        Color::CurrentColor => RGBA::transparent(),
        Color::RGBA(rgba) => rgba,
    };

    deco
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
    deco: &mut Decoration,
    ComputedValues { position, .. }: &ComputedValues,
) -> Px {
    let width = match position.width {
        GenericSize::Auto => {
            deco.margin.left = position.margin_left.absolute(Px(0.0), container_size);
            deco.margin.right = position.margin_right.absolute(Px(0.0), container_size);
            (container_size
                - deco.margin.left
                - deco.margin.right
                - deco.border.left
                - deco.border.right
                - deco.padding.left
                - deco.padding.right)
                .max(Px(0.0))
        }
        GenericSize::LengthPercentage(lp) => {
            let width = lp.absolute(container_size);
            deco.margin.left = match (position.margin_left, position.margin_right) {
                (GenericSize::Auto, GenericSize::Auto) => {
                    // center the child horizontally within its container
                    let border_box_width = width
                        + deco.padding.left
                        + deco.padding.right
                        + deco.border.left
                        + deco.border.right;
                    (container_size - border_box_width) / 2.0
                }
                (GenericSize::LengthPercentage(val), GenericSize::Auto) => {
                    val.absolute(container_size)
                }
                (GenericSize::Auto, GenericSize::LengthPercentage(val)) => {
                    let margin_right = val.absolute(container_size);
                    let border_box_width = width
                        + deco.padding.left
                        + deco.padding.right
                        + deco.border.left
                        + deco.border.right;
                    container_size - border_box_width - margin_right
                }
                (
                    GenericSize::LengthPercentage(margin_left),
                    GenericSize::LengthPercentage(_margin_right),
                ) => {
                    // over constrained (we use the specified margin_left and width, ignoring margin_right)
                    margin_left.absolute(container_size)
                }
            };
            width
        }
    };

    width
}
