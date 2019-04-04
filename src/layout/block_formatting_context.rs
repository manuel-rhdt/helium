use super::block::BlockContainer;
use super::entity::{CanCollapse, Position};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px, Position as Pos};

use arrayvec::ArrayVec;
use shred_derive::SystemData;
use specs::{Component, Entities, Entity, Join, ReadStorage, System, VecStorage, WriteStorage};

use std::collections::HashMap;

#[derive(SystemData)]
pub struct BlockFragmentConstructorResources<'a> {
    pub entities: Entities<'a>,
    pub positions: WriteStorage<'a, Position>,
    pub can_collapse: WriteStorage<'a, CanCollapse>,
    pub computed_values: ReadStorage<'a, ComputedValues>,
}

#[derive(Default)]
pub struct BlockFormattingContextBuilder {
    fixed_offset_map: HashMap<Entity, OffsetFrom>,
}

impl BlockFormattingContextBuilder {
    pub fn new() -> Self {
        Default::default()
    }
}

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
                while let Some(entity) = iter.step(&mut resources, &mut self.fixed_offset_map) {
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
            context.layout(
                &mut resources.positions,
                &resources.can_collapse,
                &self.fixed_offset_map,
            );
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

/// Returns an iterator that returns up to six entities:
/// - An entity representing the top margin
/// - an entity representing the top (border + padding)
/// - the entity given in the first argument
/// - a placeholder entity to enforce height constraints
/// - An entity representing the bottom margin
/// - an entity representing the bottom (border + padding)
fn block_fragments<'a>(
    entity: Entity,
    resources: &mut BlockFragmentConstructorResources<'_>,
    offset_from_map: &mut HashMap<Entity, OffsetFrom>,
) -> ArrayVec<[Entity; 6]> {
    let style = resources.computed_values.get(entity);

    let mut built_entities = ArrayVec::new();

    if let Some(style) = style {
        let pos = resources
            .positions
            .get(entity)
            .expect("Entity needs to have a position");

        let &Pos {
            height,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border_top_width,
            border_bottom_width,
            ..
        } = &*style.position;

        let height = height.absolute(Px(0.0), pos.width);

        // TODO: Margin and Padding size is relative to container width!
        let margin_top = margin_top.absolute(Px(0.0), pos.width);
        let border_top = border_top_width;
        let padding_top = padding_top.absolute(Px(0.0));

        let margin_bottom = margin_bottom.absolute(Px(0.0), pos.width);
        let border_bottom = border_bottom_width;
        let padding_bottom = padding_bottom.absolute(Px(0.0));

        if margin_top > Px(0.0) {
            let entity = build_spacing_entity(true, margin_top, resources);
            built_entities.push(entity);
        }

        if padding_top > Px(0.0) || border_top > Px(0.0) {
            let entity = build_spacing_entity(false, padding_top + border_top, resources);
            built_entities.push(entity);
        }

        built_entities.push(entity);

        // if we have a given height, insert a placeholder element
        if height > Px(0.0) {
            let placeholder = build_spacing_entity(false, Px(0.0), resources);
            built_entities.push(placeholder);
            offset_from_map.insert(
                placeholder,
                OffsetFrom {
                    entity,
                    offset: height,
                },
            );
        }

        if padding_bottom > Px(0.0) || border_bottom > Px(0.0) {
            let entity = build_spacing_entity(false, padding_bottom + border_bottom, resources);
            built_entities.push(entity);
        }

        if margin_bottom > Px(0.0) {
            let entity = build_spacing_entity(true, margin_bottom, resources);
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
    inner_iter: arrayvec::IntoIter<[Entity; 6]>,
}

impl<I: Iterator<Item = Entity>> BlockFragmentConstructor<I> {
    pub fn new(iter: I) -> Self {
        BlockFragmentConstructor {
            outer_iter: iter,
            inner_iter: ArrayVec::new().into_iter(),
        }
    }

    pub fn step(
        &mut self,
        resources: &mut BlockFragmentConstructorResources,
        offset_from_map: &mut HashMap<Entity, OffsetFrom>,
    ) -> Option<Entity> {
        if let Some(entity) = self.inner_iter.next() {
            Some(entity)
        } else {
            if let Some(new_entity) = self.outer_iter.next() {
                self.inner_iter =
                    block_fragments(new_entity, resources, offset_from_map).into_iter();
                Some(self.inner_iter.next().expect("Internal Error"))
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct OffsetFrom {
    entity: Entity,
    offset: Px,
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
        offset_from_map: &HashMap<Entity, OffsetFrom>,
    ) {
        let mut cursor = Px(0.0);
        let mut previous_margin_size: Option<Px> = None;

        for &entity in &self.entities {
            if let Some(offset) = offset_from_map.get(&entity) {
                let from_pos = positions.get(offset.entity).expect("Invalid Entity");
                cursor = from_pos.y + offset.offset;
                continue;
            }

            let pos = positions.get_mut(entity).expect("Invalid Entity");

            // Handle margin collapse
            if pos.height == Px(0.0) {
                continue;
            }
            if let Some(_) = can_collapse.get(entity) {
                if let Some(previous_margin) = previous_margin_size {
                    previous_margin_size = Some(previous_margin.max(pos.height));
                } else {
                    previous_margin_size = Some(pos.height);
                }
                pos.y = cursor;
            } else {
                if let Some(previous_margin) = previous_margin_size {
                    cursor += previous_margin;
                    previous_margin_size = None;
                }
                pos.y = cursor;
                cursor += pos.height;
            }
        }
    }
}
