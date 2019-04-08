use super::block::BlockContainer;
use super::entity::{Position, Text, UnscannedText};
use super::text::{font::match_font, layout_text};
use crate::style::properties::computed::{ComputedValues, LineHeight, PixelLength as Px};

use specs::{Component, Entity, VecStorage};
use specs::{Entities, Join, ReadStorage, System, WriteStorage};

use shred_derive::SystemData;

#[derive(Debug, Clone)]
pub struct InlineFormattingContext {
    pub elements: Vec<Entity>,
}

impl Component for InlineFormattingContext {
    type Storage = VecStorage<Self>;
}

#[derive(SystemData)]
pub struct InlineLayoutData<'a> {
    entities: Entities<'a>,
    computed_values: ReadStorage<'a, ComputedValues>,
    positions: WriteStorage<'a, Position>,
    inline_contexts: ReadStorage<'a, InlineFormattingContext>,
    block_containers: WriteStorage<'a, BlockContainer>,
    texts: WriteStorage<'a, Text>,
    unscanned_texts: WriteStorage<'a, UnscannedText>,
    inline_pos: WriteStorage<'a, InlinePos>,
}

pub struct InlineLayout;

impl<'a> System<'a> for InlineLayout {
    type SystemData = InlineLayoutData<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        for (entity, inline_formatting_context) in (&data.entities, &data.inline_contexts).join() {
            let &Position { x, width, .. } = data.positions.get(entity).unwrap();

            let mut inline_formatter = InlineFormatter::new(
                x,
                width,
                &data.entities,
                &mut data.positions,
                &mut data.texts,
                &mut data.inline_pos,
            );

            for &child in &inline_formatting_context.elements {
                let style = data.computed_values.get(child).expect("No style assigned");
                let mut content = if let Some(content) = data.unscanned_texts.get_mut(child) {
                    content.clone()
                } else {
                    panic!();
                };
                let font = match_font(&style).unwrap();
                let font_height = font.font_height(style.font.font_size);
                let line_height = match style.font.line_height {
                    LineHeight::Normal => font_height * 1.2,
                    LineHeight::LengthPercentage(lp) => lp.absolute(style.font.font_size),
                };
                layout_text(
                    &mut content.characters,
                    &font,
                    style.font.font_size,
                    line_height,
                    &mut inline_formatter,
                );
            }

            data.block_containers
                .insert(
                    entity,
                    BlockContainer {
                        elements: inline_formatter.finish(),
                    },
                )
                .unwrap();
        }
    }
}

pub struct InlineFormatter<'refs, 'system> {
    line_start: Px,
    line_width: Px,
    cursor: Px,
    current_line: Vec<Entity>,
    finished_lines: Vec<Entity>,
    pub entities: &'refs Entities<'system>,
    pub positions: &'refs mut WriteStorage<'system, Position>,
    pub texts: &'refs mut WriteStorage<'system, Text>,
    pub inline_pos: &'refs mut WriteStorage<'system, InlinePos>,
}

impl<'refs, 'system> InlineFormatter<'refs, 'system> {
    pub fn new(
        line_start: Px,
        line_width: Px,
        entities: &'refs Entities<'system>,
        positions: &'refs mut WriteStorage<'system, Position>,
        texts: &'refs mut WriteStorage<'system, Text>,
        inline_pos: &'refs mut WriteStorage<'system, InlinePos>,
    ) -> Self {
        InlineFormatter {
            line_start,
            line_width,
            cursor: Px(0.0),
            current_line: vec![],
            finished_lines: vec![],
            entities,
            positions,
            texts,
            inline_pos,
        }
    }

    pub fn remaining_width(&self) -> Px {
        self.line_width - self.cursor
    }

    pub fn append_fragment(&mut self, fragment: Entity, advance: Px, line_height: Px) {
        self.positions
            .insert(
                fragment,
                Position {
                    x: self.cursor,
                    y: Px(0.0), //< will be determined later (see `InlineFormatter::new_line`)
                    width: advance,
                    height: line_height,
                },
            )
            .unwrap();
        self.current_line.push(fragment);
        self.cursor += advance;
    }

    pub fn new_line(&mut self) {
        let line = std::mem::replace(&mut self.current_line, vec![]);
        let line_entity = if line.is_empty() {
            self.cursor = Px(0.0);
            return;
        } else {
            self.entities.create()
        };

        // Align the fragments along the baseline.
        // First find max ascender and max descender...
        let texts = &*self.texts;
        let get_ascent = |entity, positions: &WriteStorage<'_, _>| {
            let &Position { height, .. } = positions.get(entity).unwrap();
            texts.get(entity).map(|t| t.baseline).unwrap_or(height)
        };
        let get_descent = |entity, positions: &WriteStorage<'_, _>| {
            let &Position { height, .. } = positions.get(entity).unwrap();
            texts.get(entity).map(|t| height - t.baseline).unwrap_or(Px(0.0))
        };
        let mut max_ascent = Px(0.0);
        let mut max_descent = Px(0.0);
        for &fragment in &line {
            self.inline_pos
                .insert(fragment, InlinePos { line: line_entity })
                .unwrap();

            if get_ascent(fragment, &self.positions) > max_ascent {
                max_ascent = get_ascent(fragment, &self.positions)
            }
            if get_descent(fragment, &self.positions) > max_descent {
                max_descent = get_descent(fragment, &self.positions)
            }
        }

        // ...then shift the individual fragments to align at the baseline/
        for &fragment in &line {
            let ascent = get_ascent(fragment, &self.positions);
            let pos = self.positions.get_mut(fragment).unwrap();
            pos.y = max_ascent - ascent;
        }

        let pos = Position {
            x: self.line_start,
            y: Px(0.0), //< gets assigned in block formatting pass
            height: max_ascent + max_descent,
            width: self.cursor,
        };
        self.positions.insert(line_entity, pos).unwrap();
        self.finished_lines.push(line_entity);

        self.cursor = Px(0.0);
    }

    pub fn finish(mut self) -> Vec<Entity> {
        self.new_line();
        self.finished_lines
    }
}

#[derive(Debug)]
pub struct InlinePos {
    pub line: Entity,
}

impl Component for InlinePos {
    type Storage = VecStorage<Self>;
}

pub struct InlinePositionAssign;

use super::entity::StackingContextRelativePosition;

impl<'a> System<'a> for InlinePositionAssign {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, StackingContextRelativePosition>,
        ReadStorage<'a, InlinePos>,
    );

    fn run(&mut self, (entities, mut positions, global_pos, inline_pos): Self::SystemData) {
        for (entity, &InlinePos { line }) in (&entities, &inline_pos).join() {
            let line_pos = global_pos.get(line).cloned().unwrap();
            let self_pos = positions.get_mut(entity).unwrap();

            self_pos.x += line_pos.x;
            self_pos.y += line_pos.y;
        }
    }
}
