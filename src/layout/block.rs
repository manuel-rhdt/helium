use crate::style::properties::computed::{
    ComputedValues, GenericSize, LengthPercentage, PixelLength, Length,
};

use specs::{
    world::{EntitiesRes, Index},
    Component, Entity, VecStorage,
};

#[derive(Debug, Clone)]
pub struct BlockFormattingContext {
    pub elements: Vec<Index>,
}

impl BlockFormattingContext {
    pub fn element_entities<'a>(
        &'a self,
        entities_res: &'a EntitiesRes,
    ) -> impl 'a + Iterator<Item = Entity> {
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
pub fn calculate_inline_size(container_size: PixelLength, style: &ComputedValues) -> PixelLength {
    match style.position.width {
        GenericSize::Auto => container_size,
        GenericSize::LengthPercentage(lp) => match lp {
            LengthPercentage::Length(Length::Absolute(abs_size)) => abs_size,
            LengthPercentage::Length(Length::Em(em_size)) => style.font.font_size * em_size,
            LengthPercentage::Percentage(percent) => container_size * percent,
            LengthPercentage::Both { .. } => unimplemented!(),
        },
    }
}
