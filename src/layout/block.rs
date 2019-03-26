use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength as Px};

use specs::{Component, Entity, VecStorage};

#[derive(Debug, Clone)]
pub struct BlockContainer {
    pub elements: Vec<Entity>,
}

impl Component for BlockContainer {
    type Storage = VecStorage<Self>;
}

// https://www.w3.org/TR/CSS22/visudet.html#blockwidth
pub fn calculate_inline_size(container_size: Px, style: &ComputedValues) -> (Px, Px) {
    match style.position.width {
        GenericSize::Auto => {
            let margin_left =
                style
                    .position
                    .margin_left
                    .absolute(Px(0.0), container_size, style.font.font_size);
            let margin_right =
                style
                    .position
                    .margin_right
                    .absolute(Px(0.0), container_size, style.font.font_size);
            let width = (container_size - margin_left - margin_right).max(Px(0.0));
            (margin_left, width)
        }
        GenericSize::LengthPercentage(lp) => {
            let width = lp.absolute(container_size, style.font.font_size);
            let margin_left = match (style.position.margin_left, style.position.margin_right) {
                (GenericSize::Auto, GenericSize::Auto) => {
                    // center the child inside the container
                    (container_size - width) / 2.0
                }
                (GenericSize::LengthPercentage(val), GenericSize::Auto) => {
                    val.absolute(container_size, style.font.font_size)
                }
                (GenericSize::Auto, GenericSize::LengthPercentage(val)) => {
                    let margin_right = val.absolute(container_size, style.font.font_size);
                    container_size - width - margin_right
                }
                (
                    GenericSize::LengthPercentage(margin_left),
                    GenericSize::LengthPercentage(_margin_right),
                ) => {
                    // over constrained (we use the specified margin_left and width, ignoring margin_right)
                    margin_left.absolute(container_size, style.font.font_size)
                }
            };
            (margin_left, width)
        }
    }
}
