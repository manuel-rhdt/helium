use super::components::{
    BlockLevelElement, Decoration, EdgeValues, IntrinsicSize, Position, TextList, TextRun, ZIndex,
};
use super::{
    inline::InlineFormattingContext,
    inline_size::{calculate_inline_size, calculate_padding_and_border},
};
use crate::style::properties::computed::{ComputedValues, GenericSize, PixelLength, TextAlign};

use std::collections::BTreeMap;

use specs::prelude::*;
use specs::storage::BTreeStorage;

pub struct BlockLevelLayout {
    pub page_width: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

impl BlockLevelLayout {
    pub fn run(&mut self, world: &mut World) {
        <BlockFormatterData as SystemData>::setup(world);
        let block_formatting_contexts = world.read_storage();
        for BlockFormattingContext { elements } in block_formatting_contexts.join() {
            let system_data = BlockFormatterData::fetch(world);

            let mut block_formatter =
                BlockFormatter::new(&elements, self.page_width, self.margin, system_data);
            block_formatter.layout();
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

/// An error that signals that layout cannot continue unless a new page is started.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct PageFullError;

#[derive(Debug, Clone, Default)]
pub struct ContainerStack {
    stack: Vec<ContainerData>,
}

#[allow(unused)]
impl ContainerStack {
    pub fn start_container(&mut self, entity: Entity) -> ContainerData {
        let container_data = ContainerData {
            root_entity: entity,
        };
        self.stack.push(container_data);
        container_data
    }

    pub fn end_container(&mut self) -> Option<ContainerData> {
        self.stack.pop()
    }

    pub fn clear(&mut self) {
        self.stack.clear()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }
}

#[derive(SystemData)]
pub struct BlockFormatterData<'data> {
    entities: Entities<'data>,
    positions: WriteStorage<'data, Position>,
    decorations: WriteStorage<'data, Decoration>,
    intrinsic_sizes: ReadStorage<'data, IntrinsicSize>,
    text_list: Write<'data, TextList>,
    computed_values: ReadStorage<'data, ComputedValues>,
    inline_formatting_contexts: WriteStorage<'data, InlineFormattingContext>,
    text_runs: ReadStorage<'data, TextRun>,
}

pub struct BlockFormatter<'elements, 'data> {
    elements: &'elements [BlockLevelElement],
    container_stack: ContainerStack,
    page_width: PixelLength,
    page_margin: EdgeValues<PixelLength>,
    data: BlockFormatterData<'data>,
    page_state: PageState,
}

impl<'elements, 'data> BlockFormatter<'elements, 'data> {
    /// Creates a new `BlockFormatter` that manages the layout of `elements` into pages.
    pub fn new(
        elements: &'elements [BlockLevelElement],
        page_width: PixelLength,
        page_margin: EdgeValues<PixelLength>,
        data: BlockFormatterData<'data>,
    ) -> Self {
        BlockFormatter {
            elements,
            page_width,
            page_margin,
            container_stack: Default::default(),
            data,
            page_state: Default::default(),
        }
    }

    /// Iterate through the elements of the flow and assigns the leaf positions on the individual
    /// pages.
    fn layout(&mut self) {
        let computed_values = self.data.computed_values.clone();
        for element in self.elements.iter() {
            let style = computed_values.get(element.entity()).unwrap().clone();
            self.process_element(*element, style);
        }
    }

    // fn change_state(&mut self)

    fn process_element(&mut self, element: BlockLevelElement, style: ComputedValues) {
        match element {
            BlockLevelElement::ContainerStart(entity) => {
                let container = self.container_stack.start_container(entity);
                self.start_container_fragment(container.root_entity, style);
            }
            BlockLevelElement::Leaf(leaf) => {
                let container = self.container_stack.stack.last().unwrap();
                assert_eq!(container.root_entity, leaf);
                let container_pos = *self.data.positions.get(container.root_entity).unwrap();

                self.layout_inline_formatting_context(
                    leaf,
                    &style,
                    container_pos.x,
                    container_pos.width,
                );

                if let Some(intrinsic_size) = self.data.intrinsic_sizes.get(leaf) {
                    self.data
                        .positions
                        .insert(
                            leaf,
                            Position {
                                x: container_pos.x,
                                y: self.page_state.vertical_pos(),
                                width: container_pos.width,
                                height: intrinsic_size.height().unwrap_or_default(),
                                z: self.z_index(),
                                page: 0,
                            },
                        )
                        .unwrap();
                    self.page_state
                        .add_content(intrinsic_size.height().unwrap_or_default());
                    self.data.text_list.list.push(BlockLevelElement::Leaf(leaf))
                }
            }
            BlockLevelElement::ContainerEnd(entity) => {
                let container = self
                    .container_stack
                    .end_container()
                    .expect("Internal Error");
                assert_eq!(container.root_entity, entity);
                self.end_container_fragment(container.root_entity);
            }
        }
    }

    fn start_container_fragment(&mut self, fragment: Entity, style: ComputedValues) {
        let container_pos = self.parent_container_pos();
        let dimensions = compute_element_dimensions(
            container_pos.width,
            self.data.intrinsic_sizes.get(fragment).cloned(),
            &style,
        );

        let decoration = dimensions.decoration;

        self.page_state.add_margin(decoration.margin.top);
        self.page_state.add_content(decoration.border.top);
        self.page_state.add_content(decoration.padding.top);

        let z = self.z_index();
        self.data
            .positions
            .entry(fragment)
            .unwrap()
            .replace(Position {
                x: container_pos.x + dimensions.inline_offset(),
                y: self.page_state.vertical_pos(),
                width: dimensions.inline_size,
                height: PixelLength(0.0),
                z,
                page: 0,
            });

        self.data.decorations.insert(fragment, decoration).unwrap();

        self.data
            .text_list
            .list
            .push(BlockLevelElement::ContainerStart(fragment));
    }

    /// Ends the current fragment of an element and appends it to the current page.
    ///
    /// # Arguments
    /// - `fragment` specifies the entity index of the fragment
    /// - `page_end`: if true this function assumes that the fragment should end abruptly (without
    ///    adding bottom padding and border)
    fn end_container_fragment(&mut self, fragment: Entity) {
        let parent_pos = self.parent_container_pos();

        self.data
            .text_list
            .list
            .push(BlockLevelElement::ContainerEnd(fragment));

        let mut pos = self.data.positions.get_mut(fragment).unwrap();
        let decoration = self.data.decorations.get(fragment).unwrap();
        let style = self.data.computed_values.get(fragment).unwrap();

        let content_height = self.page_state.vertical_pos() - pos.y;
        match style.position.height {
            GenericSize::Auto => {
                pos.height = content_height;
            }
            GenericSize::LengthPercentage(lp) => {
                pos.height = lp.absolute(parent_pos.width);
                // update the page state
                self.page_state.add_content(-content_height + pos.height);
            }
        }

        self.page_state.add_content(decoration.border.bottom);
        self.page_state.add_content(decoration.padding.bottom);
        self.page_state.add_margin(decoration.margin.bottom);
    }

    fn parent_container(&self) -> Option<ContainerData> {
        let num_cont = self.container_stack.len();
        if num_cont >= 2 {
            Some(self.container_stack.stack[num_cont - 2])
        } else {
            None
        }
    }

    fn parent_container_pos(&self) -> Position {
        if let Some(pos) = self
            .parent_container()
            .and_then(|container| self.data.positions.get(container.root_entity).cloned())
        {
            pos
        } else {
            Position {
                x: self.page_margin.left,
                y: PixelLength(0.0),
                width: self.page_width - self.page_margin.left - self.page_margin.right,
                height: PixelLength(0.0),
                z: Default::default(),
                page: 0,
            }
        }
    }

    fn z_index(&self) -> ZIndex {
        ZIndex(self.container_stack.len() as i64)
    }

    fn layout_inline_formatting_context(
        &mut self,
        leaf: Entity,
        style: &ComputedValues,
        line_start: PixelLength,
        line_width: PixelLength,
    ) {
        if let Some(ifm) = self.data.inline_formatting_contexts.get_mut(leaf) {
            let entities = &self.data.entities;

            let breakpoints = &ifm.break_points;
            assert!(breakpoints.len() >= 2 || breakpoints.is_empty());

            // TODO: percentage relative to parent container
            let indent = style.text.text_indent.absolute(line_width);

            let mut weight = |from: usize, to: usize| {
                let indent = if from == 0 { indent } else { PixelLength(0.0) };
                let advance = breakpoints[to].advance_before - breakpoints[from].advance_after;
                let line_width = line_width - indent;
                if to == breakpoints.len() - 1 && advance <= line_width {
                    // the last line does not need to be justified
                    0.0
                } else {
                    if advance > line_width {
                        std::f64::INFINITY
                    } else {
                        ((line_width - advance).0 as f64).powi(3)
                    }
                }
            };

            let mut break_point_sequence =
                super::paragraph::compute_breakpoints(breakpoints.len(), &mut weight);

            if !breakpoints.is_empty() {
                break_point_sequence.insert(0, 0);
                break_point_sequence.push(breakpoints.len() - 1);
            }

            // needed for borrow checker
            let text_runs = &self.data.text_runs;
            let intrinsic_sizes = &self.data.intrinsic_sizes;
            ifm.lines = break_point_sequence
                .windows(2)
                .map(|window| {
                    let (baseline, height) =
                        ifm.line_height(window[0], window[1], text_runs, intrinsic_sizes);
                    super::inline::Line {
                        from_break: window[0],
                        to_break: window[1],
                        height,
                        baseline,
                        entity: entities.create(),
                    }
                })
                .collect();

            for (line_num, &line) in ifm.lines.iter().enumerate() {
                let content_width = breakpoints[line.to_break].advance_before
                    - breakpoints[line.from_break].advance_after;

                let (x, width) = if line_num == 0 {
                    (line_start + indent, line_width - indent)
                } else {
                    (line_start, line_width)
                };

                let (x, width) = match ifm.text_align {
                    TextAlign::Left => (x, content_width),
                    TextAlign::Justify => (x, width),
                    TextAlign::Center => (x + (width - content_width) / 2.0, content_width),
                    TextAlign::Right => (x + width - content_width, content_width),
                };

                let pos = Position {
                    x,
                    y: self.page_state.vertical_pos(),
                    width,
                    height: line.height,
                    z: ZIndex(self.container_stack.len() as i64),
                    page: 0, //< will get assigned by paginator
                };

                assert!(line.height != PixelLength(0.0));

                self.data.positions.insert(line.entity, pos).unwrap();

                self.data
                    .text_list
                    .list
                    .push(BlockLevelElement::Leaf(line.entity));

                self.page_state.add_content(line.height);
            }
        }
    }
}

/// Describes the entities corresponding to a container element during layout.
///
/// It has the following fields:
/// * `root_entity` – The entity that describes the entire container.
/// * `current_fragment` – A fragment of the container that is currently under construction.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ContainerData {
    pub root_entity: Entity,
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
    intrinsic_size: Option<IntrinsicSize>,
    style: &ComputedValues,
) -> ElementDimensions {
    let mut decoration = calculate_padding_and_border(container_size, &style);
    let inline_size = calculate_inline_size(
        container_size,
        intrinsic_size.and_then(|s| s.width()),
        &mut decoration,
        &style,
    );
    ElementDimensions {
        decoration,
        inline_size,
    }
}

/// A struct that represents a page. Contains a list of all entities that are displayed on the page.
#[derive(Debug, Clone)]
pub struct Page {
    pub size: (PixelLength, PixelLength),
    elements: specs::BitSet,
}

impl Page {
    pub fn new(width: PixelLength, height: PixelLength) -> Self {
        Page {
            size: (width, height),
            elements: Default::default(),
        }
    }

    pub fn append_element(&mut self, element: Entity) {
        if cfg!(debug_assertions) && self.elements.contains(element.id()) {
            panic!(
                "Tried to append the entity {:?} to the page although it already existed",
                element
            )
        }
        self.elements.add(element.id());
    }

    pub fn add_elements_to_set(&self, set: &mut specs::BitSet) {
        for entity in &self.elements {
            set.add(entity);
        }
    }

    pub fn contains(&self, element: Entity) -> bool {
        self.elements.contains(element.id())
    }
}

/// If an entity has this component, it is made up of two or more block layout fragment.
#[derive(Debug, Default, Clone)]
pub struct Fragments {
    fragments: Vec<Entity>,
}

impl Component for Fragments {
    type Storage = BTreeStorage<Self>;
}

/// Contains state of the vertical layout process.
///
/// Its responsibilities include
/// * managing margin collapsing
#[derive(Debug, Clone, Default)]
struct PageState {
    vertical_pos: PixelLength,
    /// Contains `Some(margin)` if the previous space was a margin.
    collapse_margin: Option<PixelLength>,
}

impl PageState {
    fn add_content(&mut self, content: PixelLength) {
        if content == PixelLength(0.0) {
            return;
        }
        self.collapse_margin = None;
        self.vertical_pos += content;
    }

    fn add_margin(&mut self, margin: PixelLength) {
        if margin == PixelLength(0.0) {
            return;
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

/// A resource that holds a sorted vector of all page entities.
#[derive(Debug, Default, Clone)]
pub struct Pages(BTreeMap<usize, Page>);

impl Pages {
    pub fn page(&self, index: usize) -> &Page {
        &self.0[&index]
    }

    pub fn iter(&self) -> impl Iterator<Item = &Page> {
        self.0.values()
    }

    pub fn num_pages(&self) -> usize {
        self.0.keys().last().map(|x| x + 1).unwrap_or_default()
    }
}

pub struct PagesCreate {
    pub page_width: PixelLength,
    pub page_height: PixelLength,
}

impl<'a> System<'a> for PagesCreate {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        Write<'a, Option<Pages>>,
    );

    fn run(&mut self, (entities, positions, mut pages): Self::SystemData) {
        let mut page_list = Pages::default();
        for (entity, Position { page, .. }) in (&entities, &positions).join() {
            page_list
                .0
                .entry(*page)
                .or_insert_with(|| Page::new(self.page_width, self.page_height))
                .append_element(entity)
        }
        *pages = Some(page_list);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::convert::TryInto;

    use crate::simple_dom::{body, div, html, Quantity::*};
    use crate::style::properties::computed::PixelLength;

    #[derive(Debug, Copy, Clone)]
    struct BlockLayoutTest<'a> {
        html: &'a crate::simple_dom::Node,
        page_width: crate::simple_dom::Quantity,
    }

    impl<'a> BlockLayoutTest<'a> {
        fn layout(self) -> specs::World {
            let mut world = self.html.construct();
            let page_width = self.page_width.try_into().unwrap();

            world.insert(TextList::default());
            let mut block_level_layout = BlockLevelLayout {
                page_width,
                margin: EdgeValues::all(PixelLength(0.0)),
            };

            block_level_layout.run(&mut world);

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
                z: ZIndex(1),
                page: 0
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
                z: ZIndex(2),
                page: 0,
            }
        );

        assert_eq!(
            pos_nested,
            &Position {
                x: PixelLength(35.0),
                y: PixelLength(0.0),
                width: PixelLength(65.0),
                height: PixelLength(0.0),
                z: ZIndex(3),
                page: 0,
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
                z: ZIndex(2),
                page: 0,
            }
        );
        assert_eq!(
            pos2,
            &Position {
                x: PixelLength(0.0),
                y: PixelLength(0.0),
                width: PixelLength(150.0),
                height: PixelLength(0.0),
                z: ZIndex(2),
                page: 0,
            }
        );
    }

    #[test]
    fn test_margin_top_offsets_first_element() {
        let html = &html(vec![div(vec![]).margin_top(Px(10.0))]);
        let test = BlockLayoutTest {
            html,
            page_width: Px(100.0),
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
                z: ZIndex(2),
                page: 0,
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
        };
        let world = test.layout();

        let positions = world.read_storage();
        let pos2: &Position = html[1].get(&positions);

        assert_eq!(pos2.y, PixelLength(40.0));
    }
}
