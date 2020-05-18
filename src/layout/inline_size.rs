use super::components::{ContainingBlock, Decoration, Position, IntrinsicSize};
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength as Px};

use specs::prelude::*;

#[derive(SystemData)]
pub struct InlineSizeResources<'a> {
    pub entities: Entities<'a>,
    pub computed_values: ReadStorage<'a, ComputedValues>,
    pub positions: WriteStorage<'a, Position>,
    pub containing_blocks: ReadStorage<'a, ContainingBlock>,
    pub decorations: WriteStorage<'a, Decoration>,
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

    // margin_left and margin_right are assigned in `calculate_inline_size`
    deco.margin.top = position.margin_top.absolute(Px(0.0), container_size);
    deco.margin.bottom = position.margin_bottom.absolute(Px(0.0), container_size);

    // TODO: Do this elsewhere?
    use cssparser::{Color, RGBA};
    fn get_color(col: Color) -> RGBA {
        match col {
            // TODO: CurrentColor
            Color::CurrentColor => RGBA::transparent(),
            Color::RGBA(rgba) => rgba,
        }
    }

    deco.background_color = get_color(background.background_color);
    deco.border_color.top = get_color(position.border_top_color);
    deco.border_color.bottom = get_color(position.border_bottom_color);
    deco.border_color.left = get_color(position.border_left_color);
    deco.border_color.right = get_color(position.border_right_color);

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
    intrinsic_size: Option<IntrinsicSize>,
    Decoration {
        margin,
        padding,
        border,
        ..
    }: &mut Decoration,
    ComputedValues { position, .. }: &ComputedValues,
) -> Px {
    match position.width {
        GenericSize::Auto => {
            margin.left = position.margin_left.absolute(Px(0.0), container_size);
            margin.right = position.margin_right.absolute(Px(0.0), container_size);
            (container_size
                - margin.left
                - margin.right
                - border.left
                - border.right
                - padding.left
                - padding.right)
                .max(Px(0.0))
        }
        GenericSize::LengthPercentage(lp) => {
            let width = lp.absolute(container_size);
            margin.left = match (position.margin_left, position.margin_right) {
                (GenericSize::Auto, GenericSize::Auto) => {
                    // center the child horizontally within its container
                    let border_box_width =
                        width + padding.left + padding.right + border.left + border.right;
                    (container_size - border_box_width) / 2.0
                }
                (GenericSize::LengthPercentage(val), GenericSize::Auto) => {
                    val.absolute(container_size)
                }
                (GenericSize::Auto, GenericSize::LengthPercentage(val)) => {
                    let margin_right = val.absolute(container_size);
                    let border_box_width =
                        width + padding.left + padding.right + border.left + border.right;
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
    }
}
