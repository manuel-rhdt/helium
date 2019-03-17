use super::block_size::BlockSizeContext;
use super::inline_size::InlineSizeContext;
use crate::default_cell_ext::DefaultCellExt;
use crate::display_list::DisplayList;
use crate::style::properties::computed::{
    ComputedValues, CssFloat, GenericSize, LengthPercentage, Percentage, PixelLength,
};
use crate::tree::TreeLinked;

use std::cell::Cell;
use std::fmt;

use specs::{world::{Index, EntitiesRes}, Component, VecStorage, Entity};

#[derive(Debug, Clone)]
pub struct BlockFormattingContext {
    pub elements: Vec<Index>,
}

impl BlockFormattingContext {
    pub fn element_entities<'a>(&'a self, entities_res: &'a EntitiesRes) -> impl 'a + Iterator<Item=Entity> {
        self.elements.iter().filter_map(move |&index| {
            let entity = entities_res.entity(index);
            if entities_res.is_alive(entity) {
                Some(entity)
            } else {
                None
            }
        })
    }
}

impl Component for BlockFormattingContext {
    type Storage = VecStorage<Self>;
}

// CSS2 10.3.3
pub fn calculate_inline_size(container_size: CssFloat, style: &ComputedValues) -> CssFloat {
    match style.position.width {
        GenericSize::Auto => container_size,
        GenericSize::LengthPercentage(lp) => match lp {
            LengthPercentage::Length(PixelLength(abs_size)) => abs_size,
            LengthPercentage::Percentage(Percentage(percent)) => container_size * percent,
            LengthPercentage::Both { .. } => unimplemented!(),
        },
    }
}
