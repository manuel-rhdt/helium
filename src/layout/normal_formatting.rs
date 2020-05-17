use super::components::{BlockLevelElement, Decoration, EdgeValues, Position, TextList, ZIndex};
use super::inline_size::{calculate_inline_size, calculate_padding_and_border};
use crate::style::properties::computed::{ComputedValues, PixelLength, TextAlign};

use std::collections::BTreeMap;

use specs::prelude::*;
use specs::storage::BTreeStorage;

pub struct BlockLevelLayout<'world> {
    pub page_width: PixelLength,
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
    pub world: &'world World,
}

impl<'world> BlockLevelLayout<'world> {
    pub fn run(&mut self) {
        let block_formatting_contexts = self.world.read_storage();
        for BlockFormattingContext { elements } in block_formatting_contexts.join() {
            let mut block_formatter = BlockFormatter::new(
                &elements,
                (self.page_width, self.page_height),
                self.margin,
                &self.world,
            );
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

pub struct BlockFormatter<'elements, 'data> {
    elements: &'elements [BlockLevelElement],
    container_stack: ContainerStack,
    page_size: (PixelLength, PixelLength),
    page_margin: EdgeValues<PixelLength>,
    world: &'data World,
    positions: WriteStorage<'data, Position>,
    decorations: WriteStorage<'data, Decoration>,
    text_list: Write<'data, TextList>,
}

impl<'elements, 'data> BlockFormatter<'elements, 'data> {
    /// Creates a new `BlockFormatter` that manages the layout of `elements` into pages.
    pub fn new(
        elements: &'elements [BlockLevelElement],
        page_size: (PixelLength, PixelLength),
        page_margin: EdgeValues<PixelLength>,
        world: &'data World,
    ) -> Self {
        BlockFormatter {
            elements,
            page_size,
            page_margin,
            world,
            positions: world.write_storage(),
            decorations: world.write_storage(),
            text_list: Write::fetch(world),
            container_stack: Default::default(),
        }
    }

    /// Iterate through the elements of the flow and assigns the leaf positions on the individual
    /// pages.
    fn layout(&mut self) {
        let computed_values = self.world.read_storage::<ComputedValues>();
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
                let pos = *self.positions.get(container.root_entity).unwrap();

                let mut inline_formatting_contexts = self.world.write_storage();
                if let Some(inline_formatting_context) = inline_formatting_contexts.get_mut(leaf) {
                    self.layout_inline_formatting_context(
                        inline_formatting_context,
                        &style,
                        pos.x,
                        pos.width,
                    );
                } else if self.positions.contains(leaf) {
                    self.text_list.list.push(BlockLevelElement::Leaf(leaf))
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
        let dimensions = compute_element_dimensions(container_pos.width, &style);

        let z = self.z_index();
        let position = self
            .positions
            .entry(fragment)
            .unwrap()
            .or_insert_with(|| Position {
                x: container_pos.x + dimensions.inline_offset(),
                y: PixelLength(0.0),
                width: dimensions.inline_size,
                height: PixelLength(0.0),
                z,
                page: 0,
            });
        position.x = container_pos.x + dimensions.inline_offset();
        position.z = z;

        self.decorations
            .insert(fragment, dimensions.decoration)
            .unwrap();

        self.text_list
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
        self.text_list
            .list
            .push(BlockLevelElement::ContainerEnd(fragment));
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
            .and_then(|container| self.positions.get(container.root_entity).cloned())
        {
            pos
        } else {
            Position {
                x: self.page_margin.left,
                y: self.page_margin.top,
                width: self.page_size.0 - self.page_margin.left - self.page_margin.right,
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
        ifm: &mut super::inline::InlineFormattingContext,
        style: &ComputedValues,
        line_start: PixelLength,
        line_width: PixelLength,
    ) {
        let entities = self.world.entities();

        let breakpoints = &ifm.break_points;

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

        ifm.lines = break_point_sequence
            .windows(2)
            .map(|window| {
                let (baseline, height) = ifm
                    .line_height(window[0], window[1], self.world.read_storage())
                    .unwrap_or_default();
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
                y: PixelLength(0.0),
                width,
                height: line.height,
                z: self.z_index(),
                page: 0,
            };
            self.positions.insert(line.entity, pos).unwrap();

            self.text_list
                .list
                .push(BlockLevelElement::Leaf(line.entity));
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
    style: &ComputedValues,
) -> ElementDimensions {
    let mut decoration = calculate_padding_and_border(container_size, &style);
    let inline_size = calculate_inline_size(container_size, &mut decoration, &style);
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
        page_height: crate::simple_dom::Quantity,
    }

    impl<'a> BlockLayoutTest<'a> {
        fn layout(self) -> specs::World {
            let mut world = self.html.construct();
            let page_width = self.page_width.try_into().unwrap();
            let page_height = self.page_height.try_into().unwrap();

            let mut block_level_layout = BlockLevelLayout {
                page_width,
                page_height,
                margin: EdgeValues::all(PixelLength(0.0)),
                world: &world,
            };

            block_level_layout.run();

            let mut pages_create = PagesCreate {
                page_width,
                page_height,
            };
            System::setup(&mut pages_create, &mut world);
            pages_create.run_now(&world);

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
                z: ZIndex(0),
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
                z: ZIndex(1),
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
                z: ZIndex(2),
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
                z: ZIndex(1),
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
                z: ZIndex(1),
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
                z: ZIndex(1),
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

        let pages = world.read_resource::<Option<Pages>>();
        let pages = pages.as_ref().unwrap();
        assert_eq!(pages.num_pages(), 1);

        let page_data = pages.page(0);

        assert!(page_data.contains(html["div"].entity()));
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

        let pages = world.read_resource::<Option<Pages>>();
        let pages = pages.as_ref().unwrap();
        assert_eq!(pages.num_pages(), 2);

        let page0 = pages.page(0);
        let page1 = pages.page(1);

        assert!(page0.contains(html[0].entity()));
        assert!(page1.contains(html[1].entity()));
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

        let pages = world.read_resource::<Option<Pages>>();
        let pages = pages.as_ref().unwrap();
        assert_eq!(pages.num_pages(), 2);

        let page0 = pages.page(0);
        let mut bitset = specs::BitSet::new();
        page0.add_elements_to_set(&mut bitset);

        assert!(bitset.contains(html[0].entity().id()));
        assert!(bitset.contains(html[1].entity().id()));
        assert!(!bitset.contains(html[2].entity().id()));
    }
}
