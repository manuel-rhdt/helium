use super::{
    components::{BlockLevelElement, Decoration, EdgeValues, Position, TextList},
    normal_formatting::ContainerStack,
};
use crate::style::properties::computed::{ComputedValues, PageBreak, PixelLength};
use specs::{Read, ReadStorage, System, WriteStorage};

pub struct Paginator {
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

impl Paginator {
    fn content_height(&self) -> PixelLength {
        self.page_height - self.margin.top - self.margin.bottom
    }
}

impl<'world> System<'world> for Paginator {
    type SystemData = (
        Read<'world, TextList>,
        WriteStorage<'world, Position>,
        ReadStorage<'world, Decoration>,
        ReadStorage<'world, ComputedValues>,
    );
    fn run(&mut self, (text_list, mut positions, decorations, computed_values): Self::SystemData) {
        let mut page_state = PageState::new(&text_list.list);

        while let Some(elem) = page_state.take_element() {
            let style = computed_values.get(elem.entity());
            let page_break_before = style.map(|s| s.page.page_break_before).unwrap_or_default();
            let page_break_after = style.map(|s| s.page.page_break_after).unwrap_or_default();

            match elem {
                BlockLevelElement::ContainerStart(start) => {
                    let decoration = decorations.get(start).unwrap();
                    page_state.add_margin(decoration.margin.top);
                    page_state.add_content(decoration.border.top);
                    page_state.add_content(decoration.padding.top);

                    if page_state.vertical_pos() > self.content_height()
                        || page_break_before == PageBreak::Always
                    {
                        page_state.new_page()
                    }

                    page_state.in_progress_containers.start_container(start);
                    let position = positions.get_mut(start).unwrap();
                    position.y = self.margin.top + page_state.vertical_pos;
                    position.page = page_state.page_num;
                }
                BlockLevelElement::ContainerEnd(end) => {
                    let decoration = decorations.get(end).unwrap();
                    page_state.add_content(decoration.padding.bottom);
                    page_state.add_content(decoration.border.bottom);
                    page_state.add_margin(decoration.margin.bottom);

                    if page_state.vertical_pos() > self.content_height()
                        || page_break_after == PageBreak::Always
                    {
                        page_state.new_page()
                    }

                    let position = positions.get_mut(end).unwrap();
                    position.height = page_state.vertical_pos + self.margin.top - position.y;

                    let container = page_state.in_progress_containers.end_container();
                    assert_eq!(container.unwrap().root_entity, end);
                }
                BlockLevelElement::Leaf(leaf) => {
                    let position = positions.get_mut(leaf).unwrap();

                    position.y = self.margin.top + page_state.vertical_pos;
                    page_state.add_content(position.height);
                    if page_state.vertical_pos() > self.content_height() {
                        page_state.new_page();
                        position.y = self.margin.top + page_state.vertical_pos;
                        page_state.add_content(position.height);
                    }

                    position.page = page_state.page_num;
                }
            };
        }
    }
}

/// Contains state of the vertical layout process.
///
/// Its responsibilities include
/// * managing margin collapsing
#[derive(Debug, Clone, Default)]
struct PageState<'elements> {
    in_progress_containers: ContainerStack,
    /// The slice of elements that have to be processed.
    elements: &'elements [BlockLevelElement],
    page_num: usize,
    vertical_pos: PixelLength,
    /// Contains `Some(margin)` if the previous space was a margin.
    collapse_margin: Option<PixelLength>,
    is_at_page_start: bool,
}

impl<'elements> PageState<'elements> {
    fn new(elements: &'elements [BlockLevelElement]) -> Self {
        PageState {
            is_at_page_start: true,
            elements,
            ..Default::default()
        }
    }

    fn take_element(&mut self) -> Option<BlockLevelElement> {
        let (&first, remaining) = self.elements.split_first()?;
        self.elements = remaining;
        Some(first)
    }

    fn add_content(&mut self, content: PixelLength) {
        self.is_at_page_start = false;
        if content == PixelLength(0.0) {
            return;
        }
        self.collapse_margin = None;
        self.vertical_pos += content;
    }

    fn add_margin(&mut self, margin: PixelLength) {
        self.is_at_page_start = false;
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

    fn new_page(&mut self) {
        self.page_num += 1;
        self.vertical_pos = PixelLength(0.0);
        self.is_at_page_start = true;
        self.collapse_margin = None;
    }
}
