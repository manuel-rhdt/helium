use super::entity::Position;
use crate::style::properties::computed::ComputedValues;
use crate::style::properties::computed::{GenericSize, LengthPercentage, PixelLength};

use specs::{Join, ReadStorage, System, WriteStorage};

pub struct AssignHeights;

impl<'a> System<'a> for AssignHeights {
    type SystemData = (ReadStorage<'a, ComputedValues>, WriteStorage<'a, Position>);

    fn run(&mut self, (computed_values, mut positions): Self::SystemData) {
        for (style, pos) in (&computed_values, &mut positions).join() {
            match style.position.height {
                GenericSize::LengthPercentage(lp) => match lp {
                    LengthPercentage::Length(PixelLength(len)) => pos.height = len,
                    _ => unimplemented!()
                },
                _ => {}
            }
        }
    }
}
