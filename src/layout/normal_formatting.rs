use super::entity::{Decoration, Position, Text, UnscannedText};
use super::inline::{InlineFormatter, InlineFormatterData, InlineFormattingContext, InlinePos};
use super::inline_size::{calculate_inline_size, calculate_padding_and_border};
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength};

use shred_derive::SystemData;
use specs::{
    storage::BTreeStorage, Component, DenseVecStorage, Entities, Entity, ReadStorage, System,
    Write, WriteStorage,
};

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
            *data.pages = Pages(block_formatter.pages);
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockLevelElement {
    ContainerStart(Entity),
    Leaf(Entity),
    ContainerEnd(Entity),
}

impl BlockLevelElement {
    /// Returns the entity associated with the `BlockLevelElement`.
    pub fn entity(self) -> Entity {
        use BlockLevelElement::*;
        match self {
            ContainerStart(entity) | Leaf(entity) | ContainerEnd(entity) => entity,
        }
    }
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

/// A struct that holds all data needed for the block layout pass.
#[derive(SystemData)]
pub struct BlockFormatterData<'data> {
    pub entities: Entities<'data>,
    pub positions: WriteStorage<'data, Position>,
    pub computed_values: ReadStorage<'data, ComputedValues>,
    pub inline_contexts: ReadStorage<'data, InlineFormattingContext>,
    pub texts: WriteStorage<'data, Text>,
    pub inline_pos: WriteStorage<'data, InlinePos>,
    pub unscanned_texts: WriteStorage<'data, UnscannedText>,
    pub page_data: WriteStorage<'data, Page>,
    pub pages: Write<'data, Pages>,
}

pub struct BlockFormatter<'elements, 'data> {
    in_progress_containers: Vec<Container>,
    pub pages: Vec<Entity>,
    current_page: Page,
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
            current_page: Default::default(),
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
            let style = match self.data.computed_values.get(element.entity()) {
                Some(style) => style,
                None => continue,
            };

            let (offset, container_size) = self.current_container_size();
            let dimensions = calculate_container_dimensions(container_size, &style);

            let Decoration {
                padding,
                border,
                margin,
                ..
            } = &dimensions.decoration;

            let position = self
                .data
                .positions
                .entry(element.entity())
                .unwrap()
                .or_insert_with(Default::default);

            position.width = dimensions.inline_size;

            if let BlockLevelElement::ContainerStart(_) | BlockLevelElement::Leaf(_) = element {
                self.cursor += padding.top + border.top + margin.top;
                position.y = self.cursor;
            }

            match element {
                BlockLevelElement::ContainerStart(entity) => {
                    let container = Container {
                        entity,
                        inline_offset: offset + dimensions.inline_offset(),
                        dimensions,
                    };
                    self.in_progress_containers.push(container);
                }
                BlockLevelElement::Leaf(leaf) => {
                    while let Err(PageFullError) = self.layout_leaf(leaf, dimensions) {
                        self.new_page()
                    }
                }
                BlockLevelElement::ContainerEnd(_) => {
                    self.in_progress_containers
                        .pop()
                        .expect("Internal Error: Invalid tree");
                }
            }

            let position = self.data.positions.get_mut(element.entity()).unwrap();
            if let BlockLevelElement::ContainerEnd(_) | BlockLevelElement::Leaf(_) = element {
                position.height = self.cursor - position.y;
                self.cursor += padding.bottom + border.bottom + margin.bottom;
            }

            // we have to reset the current inline formatter after an element has finished.
            self.inline_formatter = None;
        }
        assert!(self.in_progress_containers.is_empty());
        self.new_page();
        None
    }

    /// Put the leaf at the current cursor position.
    ///
    /// This method may make partial progress. If the current page is full, a `PageFullError` is
    /// returned. To finish the layout of the `leaf` entity, this method must be called, until it
    /// no longer returns an Error.
    ///
    /// Returns `Err` if a new page needs to be started to finish the leaf.
    fn layout_leaf(
        &mut self,
        leaf: Entity,
        dimensions: ContainerDimensions,
    ) -> Result<(), PageFullError> {
        if let Some(inline_formatting_context) = self.data.inline_contexts.get(leaf) {
            let (offset, width) = self.current_container_size();
            let remaining_block_size = self.remaining_block_size();
            // The following initializes a new inline formatter if self.inline_formatter is None.
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
                    self.current_page
                        .elements
                        .extend_from_slice(&line_stack.stack.entities());
                    if line_stack.page_break_after {
                        return Err(PageFullError);
                    }
                } else {
                    break;
                }
            }
        }

        let height = self
            .data
            .computed_values
            .get(leaf)
            .map(|style| style.position.height);
        if let Some(GenericSize::LengthPercentage(lp)) = height {
            let height = lp.absolute(dimensions.inline_size);
            self.cursor += height;

            if self.remaining_block_size() < PixelLength(0.0) {
                self.new_page();
            }

            if let Some(pos) = self.data.positions.get_mut(leaf) {
                pos.height = height;
            }
            self.current_page.append_element(leaf);
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
        self.cursor = PixelLength(0.0);
        let page = std::mem::replace(&mut self.current_page, Default::default());
        let page_entity = self
            .data
            .entities
            .build_entity()
            .with(page, &mut self.data.page_data)
            .build();
        self.pages.push(page_entity);
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
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

#[derive(Debug, Default, Clone)]
pub struct Page {
    elements: Vec<Entity>,
}

impl Page {
    pub fn append_element(&mut self, element: Entity) {
        self.elements.push(element)
    }
}

impl Component for Page {
    type Storage = BTreeStorage<Self>;
}

/// A resource that holds a sorted vector of all page entities.
#[derive(Debug, Default, Clone)]
pub struct Pages(Vec<Entity>);

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::TryInto;


    use crate::simple_dom::{div, html, Quantity::*};
    use crate::style::properties::computed::PixelLength;
    use shred::System;
    use specs::RunNow;

    #[derive(Debug, Copy, Clone)]
    struct BlockLayoutTest<'a> {
        html: &'a crate::simple_dom::Node,
        page_width: crate::simple_dom::Quantity,
        page_height: crate::simple_dom::Quantity,
    }

    impl<'a> BlockLayoutTest<'a> {
        fn layout(self) -> specs::World {
            let mut world = self.html.construct();

            let mut block_level_layout = BlockLevelLayout {
                page_width: self.page_width.try_into().unwrap(),
                page_height: self.page_height.try_into().unwrap(),
            };

            System::setup(&mut block_level_layout, &mut world.res);
            block_level_layout.run_now(&world.res);

            world
        }
    }

    #[test]
    fn test_vertical_stacking_of_consecutive_containers() {
        let html = &html(vec![
            div(vec![])
                .height(Px(10.0))
                .margin_top(Px(10.0))
                .margin_bottom(Px(30.0)),
            div(vec![]),
        ]);

        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(
            pos1,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(10.0),
                width: PixelLength(100.0),
                height: PixelLength(10.0)
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(50.0),
                width: PixelLength(100.0),
                height: PixelLength(0.0)
            }
        );
    }

    #[test]
    fn test_content_height_of_container_with_simple_children() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)),
            div(vec![]).height(Px(40.0)),
        ]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos: &Position = html.get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(100.0),
                height: PixelLength(50.0)
            }
        )
    }

    #[test]
    fn test_percentage_in_specified_size() {
        let html = &html(vec![
            div(vec![]).width(Percent(50.0)),
            div(vec![]).width(Percent(150.0)),
        ]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(
            pos1,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(50.0),
                height: PixelLength(0.0)
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(150.0),
                height: PixelLength(0.0)
            }
        );
    }

    #[test]
    fn test_margin_collapsing_between_block_level_siblings() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)).margin_bottom(Px(30.0)),
            div(vec![]).height(Px(10.0)).margin_top(Px(10.0)),
        ]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(
            pos1,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(0.0),
                height: PixelLength(10.0)
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(40.0),
                width: PixelLength(0.0),
                height: PixelLength(10.0)
            }
        );
    }

    #[test]
    fn test_page_is_generated() {
        let html = &html(vec![div(vec![]).height(Px(10.0))]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let pages = world.read_resource::<Pages>();
        assert_eq!(pages.0.len(), 1);

        let page_data = world.read_storage::<Page>();
        let page_data = page_data.get(pages.0[0]).unwrap();

        assert_eq!(page_data.elements[0], html["div"].entity());
    }

    #[test]
    fn test_page_break_simple() {
        let html = &html(vec![
            div(vec![]).height(Px(75.0)),
            div(vec![]).height(Px(75.0)),
        ]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let pages = world.read_resource::<Pages>();
        assert_eq!(pages.0.len(), 2);
    }

}
