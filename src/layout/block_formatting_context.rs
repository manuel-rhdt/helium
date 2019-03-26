use super::block::BlockContainer;
use super::entity::{CanCollapse, Position};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px};

use specs::{
    world::EntitiesRes, Component, Entities, Entity, Join, ReadStorage, System, VecStorage,
    WriteStorage,
};

pub struct BlockFormattingContextBuilder;

impl<'a> System<'a> for BlockFormattingContextBuilder {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, BlockContainer>,
        WriteStorage<'a, BlockFormattingContext>,
        ReadStorage<'a, ComputedValues>,
        WriteStorage<'a, Position>,
        WriteStorage<'a, CanCollapse>,
    );

    fn run(
        &mut self,
        (
            entities,
            block_containers,
            mut formatting_context,
            computed_values,
            mut pos,
            mut can_collapse,
        ): Self::SystemData,
    ) {
        let mut current_entity = None;
        let mut current_block_formatting_context = None;

        if let Some((entity, block_container)) = (&entities, &block_containers).join().next() {
            let current_block_formatting_context = current_block_formatting_context
                .get_or_insert_with(|| {
                    current_entity = Some(entity);
                    BlockFormattingContext::default()
                });

            let mut stack = vec![BlockFragmentConstructor::new(
                block_container.elements.iter().cloned(),
            )];
            'outer: while let Some(constructor) = stack.last_mut() {
                while let Some(entity) =
                    constructor.step(&entities, &mut pos, &computed_values, &mut can_collapse)
                {
                    if let Some(container) = block_containers.get(entity) {
                        stack.push(BlockFragmentConstructor::new(
                            container.elements.iter().cloned(),
                        ));
                        continue 'outer;
                    } else {
                        current_block_formatting_context.entities.push(entity);
                    }
                }
                stack.pop();
            }
        }

        if let (Some(entity), Some(context)) = (current_entity, current_block_formatting_context) {
            context.layout(&mut pos, &can_collapse);
            formatting_context.insert(entity, context).unwrap();
        }
    }
}

#[derive(Debug, Clone)]
struct BlockFragmentConstructor<I> {
    entities: I,
    current_entity: Option<Entity>,
    state: BlockFragmentConstructorState,
}

#[derive(Debug, Copy, Clone)]
enum BlockFragmentConstructorState {
    MarginTop,
    DecorationTop,
    Entity,
    DecorationBottom,
    MarginBottom,
}

impl<I: Iterator<Item = Entity>> BlockFragmentConstructor<I> {
    fn new(entities: I) -> Self {
        Self {
            entities,
            current_entity: None,
            state: BlockFragmentConstructorState::MarginTop,
        }
    }

    fn build_entity(
        entities: &EntitiesRes,
        positions: &mut WriteStorage<Position>,
        can_collapse: bool,
        can_collapse_store: &mut WriteStorage<CanCollapse>,
        height: Px,
    ) -> Entity {
        let pos = Position {
            height: height,
            ..Default::default()
        };
        let builder = entities.build_entity().with(pos, positions);
        if can_collapse {
            builder.with(Default::default(), can_collapse_store).build()
        } else {
            builder.build()
        }
    }

    fn step(
        &mut self,
        entities: &EntitiesRes,
        positions: &mut WriteStorage<Position>,
        computed_values: &ReadStorage<ComputedValues>,
        can_collapse: &mut WriteStorage<CanCollapse>,
    ) -> Option<Entity> {
        loop {
            let entity = self.current_entity.or_else(|| self.entities.next())?;
            self.current_entity = Some(entity);
            let style = computed_values.get(entity).cloned().unwrap_or_default();
            let pos = positions
                .get(entity)
                .expect("Entity needs to have a position");

            match self.state {
                BlockFragmentConstructorState::MarginTop => {
                    self.state = BlockFragmentConstructorState::DecorationTop;
                    let margin_top = style.position.margin_top.absolute(
                        Px(0.0),
                        pos.width,
                        style.font.font_size,
                    );

                    if margin_top > Px(0.0) {
                        let margin_entity =
                            Self::build_entity(entities, positions, true, can_collapse, margin_top);
                        break Some(margin_entity);
                    }
                }
                BlockFragmentConstructorState::DecorationTop => {
                    self.state = BlockFragmentConstructorState::Entity;

                    let border_top = style
                        .position
                        .border_top_width
                        .absolute(style.font.font_size);

                    if border_top > Px(0.0) {
                        let decoration_entity = Self::build_entity(
                            entities,
                            positions,
                            false,
                            can_collapse,
                            border_top,
                        );
                        break Some(decoration_entity);
                    }
                }
                BlockFragmentConstructorState::Entity => {
                    self.state = BlockFragmentConstructorState::DecorationBottom;
                    break Some(entity);
                }
                BlockFragmentConstructorState::DecorationBottom => {
                    self.state = BlockFragmentConstructorState::MarginBottom;

                    let border_bottom = style
                        .position
                        .border_bottom_width
                        .absolute(style.font.font_size);

                    if border_bottom > Px(0.0) {
                        let decoration_entity = Self::build_entity(
                            entities,
                            positions,
                            false,
                            can_collapse,
                            border_bottom,
                        );
                        break Some(decoration_entity);
                    }
                }
                BlockFragmentConstructorState::MarginBottom => {
                    self.state = BlockFragmentConstructorState::MarginTop;
                    self.current_entity = None;

                    let margin_bottom = style.position.margin_bottom.absolute(
                        Px(0.0),
                        pos.width,
                        style.font.font_size,
                    );

                    if margin_bottom > Px(0.0) {
                        let margin_entity = Self::build_entity(
                            entities,
                            positions,
                            true,
                            can_collapse,
                            margin_bottom,
                        );
                        break Some(margin_entity);
                    }
                }
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
