use super::entity::{Decoration, Position, Text, UnscannedText, ZIndex};
use super::inline::{InlineFormatter, InlineFormatterData, InlineFormattingContext, InlinePos};
use super::inline_size::{calculate_inline_size, calculate_padding_and_border};
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength};

use specs::prelude::*;
use specs::storage::BTreeStorage;

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

/// A struct that holds all storages and resources needed for the block layout pass.
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
    pub line_boxes: WriteStorage<'data, super::inline::LineBox>,
    pub decorations: WriteStorage<'data, Decoration>,
    pub fragments: WriteStorage<'data, Fragments>,
}

pub struct BlockFormatter<'elements, 'data> {
    in_progress_containers: Vec<Entity>,
    pub pages: Vec<Entity>,
    current_page: Page,
    /// The slice of elements that have to be processed
    pub elements: &'elements [BlockLevelElement],
    layout_state: BlockLayoutState,
    page_size: (PixelLength, PixelLength),
    inline_formatter: Option<InlineFormatter>,
    data: &'elements mut BlockFormatterData<'data>,
}

impl<'elements, 'data> BlockFormatter<'elements, 'data> {
    /// Creates a new `BlockFormatter` that manages the layout of `elements` into pages.
    pub fn new(
        elements: &'elements [BlockLevelElement],
        page_size: (PixelLength, PixelLength),
        data: &'elements mut BlockFormatterData<'data>,
    ) -> Self {
        BlockFormatter {
            in_progress_containers: vec![],
            pages: vec![],
            current_page: Default::default(),
            elements,
            layout_state: BlockLayoutState {
                vertical_pos: PixelLength(0.0),
                page_height: page_size.1,
                collapse_margin: None,
            },
            page_size,
            inline_formatter: None,
            data,
        }
    }

    /// Returns the remaining space in the block direction until the current page is full.
    pub fn remaining_block_size(&self) -> PixelLength {
        self.layout_state.remaining_block_size()
    }

    /// Iterates through the elements of the flow and assigns the leaf positions on the individual
    /// pages.
    ///
    /// The container elements are not yet split into fragments. However we need to keep track of
    /// their sizes to correctly layout the leaves (i.e. leaving space for vertical margins and
    /// decoration and determining the width of leaves).
    pub fn layout(&mut self) -> Option<()> {
        assert!(self.in_progress_containers.is_empty());
        for &element in self.elements.iter() {
            self.process_element(element);
        }
        assert!(self.in_progress_containers.is_empty());
        self.new_page();
        None
    }

    fn process_element(&mut self, element: BlockLevelElement) -> Option<()> {
        if let BlockLevelElement::ContainerStart(_) | BlockLevelElement::Leaf(_) = element {
            self.start_element(element.entity())?
        }

        match element {
            BlockLevelElement::ContainerStart(entity) => {
                self.in_progress_containers.push(entity);
            }
            BlockLevelElement::Leaf(leaf) => {
                while let Err(PageFullError) = self.layout_leaf(leaf) {
                    self.new_page()
                }
                // we have to reset the current inline formatter after an element has finished.
                self.inline_formatter = None;
            }
            BlockLevelElement::ContainerEnd(entity) => {
                let container = self
                    .in_progress_containers
                    .pop()
                    .expect("Internal Error: Invalid tree");
                assert_eq!(container, entity);
                self.current_page.append_element(container);
            }
        }

        if let BlockLevelElement::ContainerEnd(_) | BlockLevelElement::Leaf(_) = element {
            self.end_element(element.entity())?
        }

        Some(())
    }

    fn start_element(&mut self, element: Entity) -> Option<()> {
        let style = self.data.computed_values.get(element)?;
        let container_pos = self.current_container_pos();
        let dimensions = compute_element_dimensions(container_pos.width, &style);

        let Decoration {
            padding,
            border,
            margin,
            ..
        } = &dimensions.decoration;

        self.layout_state.add_margin(margin.top);
        self.layout_state.add_content(border.top);
        self.layout_state.add_content(padding.top);

        let position = self
            .data
            .positions
            .entry(element)
            .ok()?
            .or_insert_with(Default::default);

        position.x = container_pos.x + dimensions.inline_offset();
        position.y = self.layout_state.vertical_pos();
        position.width = dimensions.inline_size;
        position.z = ZIndex(self.in_progress_containers.len() as i64);
        self.data
            .decorations
            .insert(element, dimensions.decoration)
            .ok()?;

        Some(())
    }

    fn end_element(&mut self, element: Entity) -> Option<()> {
        // TODO: handle multi-page elements
        let Decoration {
            padding,
            border,
            margin,
            ..
        } = self.data.decorations.get(element)?;
        let position = self.data.positions.get_mut(element)?;

        position.height = self.layout_state.vertical_pos() - position.y;
        self.layout_state.add_content(padding.bottom);
        self.layout_state.add_content(border.bottom);
        self.layout_state.add_margin(margin.bottom);
        Some(())
    }

    /// Put the leaf at the current cursor position.
    ///
    /// This method may make partial progress. If the current page is full, a `PageFullError` is
    /// returned. This signals to the caller that a new page has to be started before calling this
    /// function again. To finish the layout of the `leaf` entity, this method must be called, until
    /// it no longer returns an Error.
    ///
    /// Returns `Err` if a new page needs to be started to finish the leaf. If the return value is
    /// `Ok(())` the leaf is completely laid out.
    fn layout_leaf(&mut self, leaf: Entity) -> Result<(), PageFullError> {
        let &Position { x, width, .. } = self.data.positions.get(leaf).unwrap();
        let mut fragment_height = PixelLength(0.0);
        let container_width = self.current_container_pos().width;
        let z_index = self.z_index();
        let result = if let Some(inline_formatting_context) = self.data.inline_contexts.get(leaf) {
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
                    line_boxes: &mut self.data.line_boxes,
                };
                let mut iter = inline_formatter.iter(
                    &inline_formatting_context.elements,
                    &mut inline_formatter_data,
                );
                if let Some(line_stack) = iter.next() {
                    line_stack
                        .stack
                        .set_positions(
                            (x, self.layout_state.vertical_pos(), ZIndex(z_index.0 + 1)),
                            &mut self.data.positions,
                        )
                        .expect("line_stack entities were invalidated while still in use.");
                    fragment_height += line_stack.stack.height();
                    self.current_page
                        .elements
                        .extend_from_slice(&line_stack.stack.entities());
                    if line_stack.page_break_after {
                        break Err(PageFullError);
                    }
                } else {
                    break Ok(());
                }
            }
        } else {
            Ok(())
        };

        let height = self
            .data
            .computed_values
            .get(leaf)
            .map(|style| style.position.height);
        if let Some(GenericSize::LengthPercentage(lp)) = height {
            let height = lp.absolute(container_width);
            self.layout_state.add_content(height);

            if self.remaining_block_size() < PixelLength(0.0) {
                return Err(PageFullError);
            }
        } else {
            self.layout_state.add_content(fragment_height);
        }
        self.current_page.append_element(leaf);

        result
    }

    fn current_container(&self) -> Option<Entity> {
        self.in_progress_containers.last().cloned()
    }

    fn current_container_pos(&self) -> Position {
        if let Some(container) = self.current_container() {
            self.data.positions.get(container).cloned().unwrap()
        } else {
            Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: self.page_size.0,
                height: self.page_size.1,
                z: Default::default(),
            }
        }
    }

    fn new_page(&mut self) {
        // split containers:
        // - add `Fragments` component to container
        // - create two fragments
        // - add the first fragment to the current page
        for &container in &self.in_progress_containers {
            let decoration = self
                .data
                .decorations
                .remove(container)
                .expect("expected component");
            let mut pos = *self.data.positions.get(container).unwrap();
            pos.height = self.layout_state.vertical_pos() - pos.y;
            let fragment1 = self
                .data
                .entities
                .build_entity()
                .with(decoration, &mut self.data.decorations)
                .with(pos, &mut self.data.positions)
                .build();
            self.data
                .fragments
                .insert(
                    container,
                    Fragments {
                        fragments: vec![fragment1],
                    },
                )
                .unwrap();
            self.current_page.append_element(fragment1);
        }

        self.layout_state.reset();

        let page = std::mem::replace(&mut self.current_page, Default::default());
        let page_entity = self
            .data
            .entities
            .build_entity()
            .with(page, &mut self.data.page_data)
            .build();
        self.pages.push(page_entity);
    }

    fn z_index(&self) -> ZIndex {
        ZIndex(self.in_progress_containers.len() as i64)
    }
}

struct BlockLayoutState {
    vertical_pos: PixelLength,
    page_height: PixelLength,
    /// Contains `Some(margin)` if the previous space was a margin.
    collapse_margin: Option<PixelLength>,
}

impl BlockLayoutState {
    fn reset(&mut self) {
        self.vertical_pos = PixelLength(0.0);
        self.collapse_margin = None;
    }

    fn remaining_block_size(&self) -> PixelLength {
        self.page_height - self.vertical_pos
    }

    fn add_content(&mut self, content: PixelLength) {
        if content == PixelLength(0.0) {
            return;
        }
        self.collapse_margin = None;
        self.vertical_pos += content;
    }

    fn add_margin(&mut self, margin: PixelLength) {
        if margin == PixelLength(0.0) {
            return
        }
        if let Some(prev_margin) = self.collapse_margin {
            let margin = prev_margin.max(margin);
            self.collapse_margin = Some(margin);
            let additional_margin = margin - prev_margin;
            self.vertical_pos += additional_margin;
        } else {
            self.collapse_margin = Some(margin);
            self.vertical_pos += margin;
        }
    }

    fn vertical_pos(&self) -> PixelLength {
        self.vertical_pos
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
struct ElementDimensions {
    decoration: Decoration,
    inline_size: PixelLength,
}

impl ElementDimensions {
    /// Returns distance between the left edge of the element's content area and the left edge of
    /// the containing element's content area.
    ///
    /// In other words, this function returns the combined length of the left margin, left padding
    /// and left border width.
    fn inline_offset(self) -> PixelLength {
        self.decoration.border.left + self.decoration.margin.left + self.decoration.padding.left
    }
}

/// Uses the constraints given by the width, height, margin, padding and border properties to
/// compute the dimensions of an element.
///
/// # Arguments
/// - `container_size`: the horizontal size of the content area of the containing element
/// - `style`: the computed css properties of the element
fn compute_element_dimensions(
    container_size: PixelLength,
    style: &ComputedValues,
) -> ElementDimensions {
    let mut decoration = calculate_padding_and_border(container_size, &style);
    let inline_size = calculate_inline_size(container_size, &mut decoration, &style);
    ElementDimensions {
        decoration,
        inline_size,
    }
}

#[derive(Debug, Copy, Clone)]
struct Container {
    entity: Entity,
    inline_offset: PixelLength,
    dimensions: ElementDimensions,
}

// TODO: consider if the vec is needed (we could replace it by a bitset).
#[derive(Debug, Default, Clone)]
pub struct Page {
    elements: Vec<Entity>,
}

impl Page {
    pub fn append_element(&mut self, element: Entity) {
        self.elements.push(element)
    }

    pub fn add_elements_to_set(&self, set: &mut specs::BitSet) {
        for &entity in &self.elements {
            set.add(entity.id());
        }
    }
}

impl Component for Page {
    type Storage = BTreeStorage<Self>;
}

/// If an entity has this component, it is made up of two or more block layout fragment.
#[derive(Debug, Default, Clone)]
pub struct Fragments {
    fragments: Vec<Entity>,
}

impl Component for Fragments {
    type Storage = BTreeStorage<Self>;
}

/// A resource that holds a sorted vector of all page entities.
#[derive(Debug, Default, Clone)]
pub struct Pages(pub Vec<Entity>);

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::TryInto;

    use crate::simple_dom::{body, div, html, Quantity::*};
    use crate::style::properties::computed::PixelLength;
    use specs::{RunNow, System};

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

            System::setup(&mut block_level_layout, &mut world);
            block_level_layout.run_now(&world);

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

        assert_eq!(pos1.y, PixelLength(10.0));
        assert_eq!(pos2.y, PixelLength(50.0));
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
                height: PixelLength(50.0),
                z: ZIndex(0)
            }
        )
    }

    const EPSILON: PixelLength = PixelLength(1e-5);

    #[test]
    fn test_nested_elements_are_positioned_correctly() {
        let child = div(vec![]).height(Px(20.0)).margin_bottom(Px(30.0));
        let html = &html(vec![body(vec![div(vec![child.clone(), child])
            .padding(Px(10.0))
            .margin_top(Px(5.0))])
        .margin_left(Px(10.0))]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(200.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let container_pos: &Position = html["body"][0].get(&positions);

        assert_eq!(
            (container_pos.x, container_pos.y),
            (PixelLength(20.0), PixelLength(15.0))
        );

        let child1_pos = html["body"][0][0].get(&positions);
        assert!((child1_pos.x - container_pos.x).abs() <= EPSILON);
        assert!((child1_pos.y - container_pos.y).abs() <= EPSILON);
        assert!(child1_pos.height == PixelLength(20.0));

        let child2_pos = html["body"][0][1].get(&positions);
        assert!((child2_pos.x - container_pos.x).abs() <= EPSILON);
        assert!((child2_pos.y - child1_pos.y - PixelLength(50.0)).abs() <= EPSILON);
        assert!(child2_pos.height == PixelLength(20.0));
    }

    #[test]
    fn test_left_margin_in_absolute_units_is_respected() {
        let html = &html(vec![
            div(vec![div(vec![]).margin_left(Px(15.0))]).margin_left(Px(20.0))
        ]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos: &Position = html["div"].get(&positions);
        let pos_nested: &Position = html["div"]["div"].get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(20.0),
                y: PixelLength(0.0),
                width: PixelLength(80.0),
                height: PixelLength(0.0),
                z: ZIndex(1)
            }
        );

        assert_eq!(
            pos_nested,
            &Position {
                x: PixelLength(35.0),
                y: PixelLength(0.0),
                width: PixelLength(65.0),
                height: PixelLength(0.0),
                z: ZIndex(2)
            }
        );
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
                height: PixelLength(0.0),
                z: ZIndex(1)
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(150.0),
                height: PixelLength(0.0),
                z: ZIndex(1)
            }
        );
    }

    #[test]
    fn test_margin_top_offsets_first_element() {
        let html = &html(vec![div(vec![]).margin_top(Px(10.0))]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
            page_height: Px(100.0),
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos: &Position = html[0].get(&positions);

        assert_eq!(
            pos,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(10.0),
                width: PixelLength(100.0),
                height: PixelLength(0.0),
                z: ZIndex(1)
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
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(pos2.y, PixelLength(40.0));
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

        assert!(page_data.elements.contains(&html["div"].entity()));
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

        let page_data = world.read_storage::<Page>();
        let page0 = page_data.get(pages.0[0]).unwrap();
        let page1 = page_data.get(pages.0[1]).unwrap();

        assert!(page0.elements.contains(&html[0].entity()));
        assert!(page1.elements.contains(&html[1].entity()));
    }

    #[test]
    fn test_page_returns_correct_bitset() {
        let html = &html(vec![
            div(vec![]).height(Px(25.0)),
            div(vec![]).height(Px(25.0)),
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

        let page_data = world.read_storage::<Page>();
        let page0 = page_data.get(pages.0[0]).unwrap();
        let mut bitset = specs::BitSet::new();
        page0.add_elements_to_set(&mut bitset);

        assert!(bitset.contains(html[0].entity().id()));
        assert!(bitset.contains(html[1].entity().id()));
        assert!(!bitset.contains(html[2].entity().id()));
    }
}
