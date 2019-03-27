use super::block::BlockContainer;
use super::entity::{CanCollapse, Position};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px, Position as Pos};

use arrayvec::ArrayVec;
use shred_derive::SystemData;
use specs::{Component, Entities, Entity, Join, ReadStorage, System, VecStorage, WriteStorage};

#[derive(SystemData)]
pub struct BlockFragmentConstructorResources<'a> {
    pub entities: Entities<'a>,
    pub positions: WriteStorage<'a, Position>,
    pub can_collapse: WriteStorage<'a, CanCollapse>,
    pub computed_values: ReadStorage<'a, ComputedValues>,
}

pub struct BlockFormattingContextBuilder;

impl<'a> System<'a> for BlockFormattingContextBuilder {
    type SystemData = (
        ReadStorage<'a, BlockContainer>,
        WriteStorage<'a, BlockFormattingContext>,
        BlockFragmentConstructorResources<'a>,
    );

    fn run(&mut self, (block_containers, mut formatting_context, mut resources): Self::SystemData) {
        let mut current_entity = None;
        let mut current_block_formatting_context = None;

        if let Some((entity, block_container)) =
            (&resources.entities, &block_containers).join().next()
        {
            let current_block_formatting_context = current_block_formatting_context
                .get_or_insert_with(|| {
                    current_entity = Some(entity);
                    BlockFormattingContext::default()
                });

            // defined for convenience
            fn fragment_iter<'a>(
                container: &'a BlockContainer,
            ) -> BlockFragmentConstructor<impl 'a + Iterator<Item = Entity>> {
                BlockFragmentConstructor::new(container.elements.iter().cloned())
            }

            let mut stack = vec![fragment_iter(block_container)];
            'outer: while let Some(iter) = stack.last_mut() {
                while let Some(entity) = iter.step(&mut resources) {
                    if let Some(container) = block_containers.get(entity) {
                        stack.push(fragment_iter(container));
                        continue 'outer;
                    } else {
                        current_block_formatting_context.entities.push(entity);
                    }
                }
                stack.pop();
            }
        }

        if let (Some(entity), Some(context)) = (current_entity, current_block_formatting_context) {
            context.layout(&mut resources.positions, &resources.can_collapse);
            formatting_context.insert(entity, context).unwrap();
        }
    }
}

fn build_spacing_entity(
    can_collapse: bool,
    height: Px,
    resources: &mut BlockFragmentConstructorResources,
) -> Entity {
    let pos = Position {
        height: height,
        ..Default::default()
    };
    let builder = resources
        .entities
        .build_entity()
        .with(pos, &mut resources.positions);
    if can_collapse {
        builder
            .with(Default::default(), &mut resources.can_collapse)
            .build()
    } else {
        builder.build()
    }
}

/// Returns an iterator that returns up to five entities:
/// - An entity representing the top margin
/// - an entity representing the top (border + padding)
/// - the entity given in the first argument
/// - An entity representing the bottom margin
/// - an entity representing the bottom (border + padding)
fn block_fragments<'a>(
    entity: Entity,
    resources: &mut BlockFragmentConstructorResources<'_>,
) -> ArrayVec<[Entity; 5]> {
    let style = resources.computed_values.get(entity);

    let mut built_entities = ArrayVec::<[_; 5]>::new();

    if let Some(style) = style {
        let pos = resources
            .positions
            .get(entity)
            .expect("Entity needs to have a position");

        let &Pos {
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border_top_width,
            border_bottom_width,
            ..
        } = &*style.position;

        // TODO: Margin and Padding size is relative to container width!
        let margin_top = margin_top.absolute(Px(0.0), pos.width, style.font.font_size);
        let border_top = border_top_width.absolute(style.font.font_size);
        let padding_top = padding_top.absolute(Px(0.0), style.font.font_size);

        let margin_bottom = margin_bottom.absolute(Px(0.0), pos.width, style.font.font_size);
        let border_bottom = border_bottom_width.absolute(style.font.font_size);
        let padding_bottom = padding_bottom.absolute(Px(0.0), style.font.font_size);

        if margin_top > Px(0.0) {
            let entity = build_spacing_entity(true, margin_top, resources);
            built_entities.push(entity);
        }

        if padding_top > Px(0.0) || border_top > Px(0.0) {
            let entity = build_spacing_entity(false, padding_top + border_top, resources);
            built_entities.push(entity);
        }

        built_entities.push(entity);

        if margin_bottom > Px(0.0) {
            let entity = build_spacing_entity(true, margin_bottom, resources);
            built_entities.push(entity);
        }

        if padding_bottom > Px(0.0) || border_bottom > Px(0.0) {
            let entity = build_spacing_entity(false, padding_bottom + border_bottom, resources);
            built_entities.push(entity);
        }

        built_entities
    } else {
        built_entities.push(entity);
        built_entities
    }
}

pub struct BlockFragmentConstructor<I> {
    outer_iter: I,
    inner_iter: arrayvec::IntoIter<[Entity; 5]>,
}

impl<I: Iterator<Item = Entity>> BlockFragmentConstructor<I> {
    pub fn new(iter: I) -> Self {
        BlockFragmentConstructor {
            outer_iter: iter,
            inner_iter: ArrayVec::new().into_iter(),
        }
    }

    pub fn step(&mut self, resources: &mut BlockFragmentConstructorResources) -> Option<Entity> {
        if let Some(entity) = self.inner_iter.next() {
            Some(entity)
        } else {
            if let Some(new_entity) = self.outer_iter.next() {
                self.inner_iter = block_fragments(new_entity, resources).into_iter();
                Some(self.inner_iter.next().expect("Internal Error"))
            } else {
                None
            }
        }
    }
}

#[derive(Default)]
pub struct BlockFormattingContext {
    pub entities: Vec<Entity>,
}

impl Component for BlockFormattingContext {
    type Storage = VecStorage<Self>;
}

impl BlockFormattingContext {
    fn layout(
        &self,
        positions: &mut WriteStorage<'_, Position>,
        can_collapse: &WriteStorage<'_, CanCollapse>,
    ) {
        let mut block_size = Px(0.0);
        let mut previous_margin_size: Option<Px> = None;

        for &entity in &self.entities {
            let pos = positions.get_mut(entity).expect("Invalid Entity");

            if pos.height == Px(0.0) {
                pos.y = block_size;
                continue;
            }

            // Handle margin collapse
            if let Some(_) = can_collapse.get(entity) {
                pos.y = block_size;
                if let Some(previous_margin) = previous_margin_size {
                    previous_margin_size = Some(previous_margin.max(pos.height));
                } else {
                    previous_margin_size = Some(pos.height);
                }
            } else {
                if let Some(previous_margin) = previous_margin_size {
                    block_size += previous_margin;
                    previous_margin_size = None;
                }
                pos.y = block_size;
                block_size += pos.height;
            }
        }
    }
}
