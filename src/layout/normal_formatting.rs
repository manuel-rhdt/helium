use super::entity::{Decoration, Position, Text, UnscannedText};
use super::inline::{InlineFormatter, InlineFormatterData, InlineFormattingContext, InlinePos};
use super::inline_size::{calculate_inline_size, calculate_padding_and_border};
use crate::style::properties::computed::{ComputedValues, PixelLength};

use shred_derive::SystemData;
use specs::{Component, DenseVecStorage, Entities, Entity, ReadStorage, System, WriteStorage};

pub struct BlockLevelLayout {
    pub page_width: PixelLength,
    pub page_height: PixelLength,
}

impl<'data> System<'data> for BlockLevelLayout {
    type SystemData = (
        BlockFormatterData<'data>,
        ReadStorage<'data, BlockFormattingContext>,
    );

    fn run(&mut self, (mut data, block_formatting_contexts): Self::SystemData) {
        use specs::Join;

        for BlockFormattingContext { elements } in block_formatting_contexts.join() {
            let mut block_formatter =
                BlockFormatter::new(&elements, (self.page_width, self.page_height), &mut data);
            block_formatter.layout();
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockLevelElement {
    ContainerStart(Entity),
    Leaf(Entity),
    ContainerEnd(Entity),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockFormattingContext {
    pub elements: Vec<BlockLevelElement>,
}

impl Component for BlockFormattingContext {
    type Storage = DenseVecStorage<Self>;
}

pub struct BlockFragment {
    pub parent: Entity,
    pub start: Entity,
    pub end: Entity,
}

impl Component for BlockFragment {
    type Storage = DenseVecStorage<Self>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct PageFullError;

#[derive(SystemData)]
pub struct BlockFormatterData<'data> {
    pub entities: Entities<'data>,
    pub positions: WriteStorage<'data, Position>,
    pub computed_values: ReadStorage<'data, ComputedValues>,
    pub inline_contexts: ReadStorage<'data, InlineFormattingContext>,
    pub texts: WriteStorage<'data, Text>,
    pub inline_pos: WriteStorage<'data, InlinePos>,
    pub unscanned_texts: WriteStorage<'data, UnscannedText>,
}

pub struct BlockFormatter<'elements, 'data> {
    in_progress_containers: Vec<Container>,
    pub pages: Vec<Entity>,
    pub elements: &'elements [BlockLevelElement],
    cursor: PixelLength,
    container_size: (PixelLength, PixelLength),
    inline_formatter: Option<InlineFormatter>,
    data: &'elements mut BlockFormatterData<'data>,
}

impl<'elements, 'data> BlockFormatter<'elements, 'data> {
    pub fn new(
        elements: &'elements [BlockLevelElement],
        container_size: (PixelLength, PixelLength),
        data: &'elements mut BlockFormatterData<'data>,
    ) -> Self {
        BlockFormatter {
            in_progress_containers: vec![],
            pages: vec![],
            elements,
            cursor: PixelLength(0.0),
            container_size,
            inline_formatter: None,
            data,
        }
    }

    pub fn remaining_block_size(&self) -> PixelLength {
        self.container_size.1 - self.cursor
    }

    /// Iterates through the elements of the flow and assigns the leaf positions on the individual
    /// pages.
    ///
    /// The container elements are not yet split into fragments. However we need to keep track of
    /// their sizes to correctly layout the leaves (i.e. leaving space for vertical margins and
    /// decoration and determining the width of leaves).
    pub fn layout(&mut self) -> Option<()> {
        assert!(self.in_progress_containers.is_empty());
        let mut iter = self.elements.iter();
        for &element in &mut iter {
            match element {
                BlockLevelElement::ContainerStart(entity) => {
                    let style = self.data.computed_values.get(entity).unwrap();
                    let (offset, container_size) = self.current_container_size();
                    let dimensions = calculate_container_dimensions(container_size, &style);

                    let container = Container {
                        entity,
                        inline_offset: offset + dimensions.inline_offset(),
                        dimensions,
                    };
                    self.in_progress_containers.push(container);

                    let Decoration {
                        padding,
                        border,
                        margin,
                        ..
                    } = &dimensions.decoration;
                    self.cursor += padding.top + border.top + margin.top;

                }
                BlockLevelElement::Leaf(leaf) => {
                    while let Err(PageFullError) = self.layout_leaf(leaf) {
                        self.new_page()
                    }
                }
                BlockLevelElement::ContainerEnd(_) => {
                    let container = self
                        .in_progress_containers
                        .pop()
                        .expect("Internal Error: Invalid tree");

                    let Decoration {
                        padding,
                        border,
                        margin,
                        ..
                    } = &container.dimensions.decoration;
                    self.cursor += padding.bottom + border.bottom + margin.bottom;
                }
            }

            // we have to reset the current inline formatter after an element has finished.
            self.inline_formatter = None;
        }
        assert!(self.in_progress_containers.is_empty());
        None
    }

    /// Put the leaf at the current cursor position.
    ///
    /// Returns `Err` if a new page needs to be started to finish the leaf.
    fn layout_leaf(&mut self, leaf: Entity) -> Result<(), PageFullError> {
        if let Some(inline_formatting_context) = self.data.inline_contexts.get(leaf) {
            // Initialize an inline formatter if `self.inline_formatter` is `None`.
            let (offset, width) = self.current_container_size();
            let remaining_block_size = self.remaining_block_size();
            let inline_formatter = self
                .inline_formatter
                .get_or_insert_with(|| InlineFormatter::new(width, width, remaining_block_size));

            loop {
                let mut inline_formatter_data = InlineFormatterData {
                    entities: &self.data.entities,
                    positions: &mut self.data.positions,
                    texts: &mut self.data.texts,
                    inline_pos: &mut self.data.inline_pos,
                    computed_values: &self.data.computed_values,
                    unscanned_texts: &mut self.data.unscanned_texts,
                };
                let mut iter = inline_formatter.iter(
                    &inline_formatting_context.elements,
                    &mut inline_formatter_data,
                );
                if let Some(line_stack) = iter.next() {
                    line_stack
                        .stack
                        .set_positions((offset, self.cursor), &mut self.data.positions);
                    self.cursor += line_stack.stack.height();
                    if line_stack.page_break_after {
                        return Err(PageFullError);
                    }
                } else {
                    break;
                }
            }
        }
        Ok(())
    }

    fn current_container_size(&self) -> (PixelLength, PixelLength) {
        self.in_progress_containers
            .last()
            .cloned()
            .map(|container| (container.inline_offset, container.dimensions.inline_size))
            .unwrap_or((PixelLength(0.0), self.container_size.0))
    }

    fn new_page(&mut self) {
        // self.cursor = PixelLength(0.0);
    }
}

#[derive(Debug, Copy, Clone)]
struct ContainerDimensions {
    decoration: Decoration,
    inline_size: PixelLength,
}

impl ContainerDimensions {
    fn inline_offset(self) -> PixelLength {
        self.decoration.border.left + self.decoration.margin.left + self.decoration.padding.left
    }
}

fn calculate_container_dimensions(
    container_size: PixelLength,
    style: &ComputedValues,
) -> ContainerDimensions {
    let mut decoration = calculate_padding_and_border(container_size, &style);
    let inline_size = calculate_inline_size(container_size, &mut decoration, &style);
    ContainerDimensions {
        decoration,
        inline_size,
    }
}

#[derive(Debug, Copy, Clone)]
struct Container {
    entity: Entity,
    inline_offset: PixelLength,
    dimensions: ContainerDimensions,
}
