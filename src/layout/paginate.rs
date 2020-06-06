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

#[derive(Debug, Copy, Clone)]
struct PageBreakOpportunity {
    before_index: usize,
    forced: bool,
    advance_before: PixelLength,
    advance_after: PixelLength,
}

impl<'world> System<'world> for Paginator {
    type SystemData = (
        Read<'world, TextList>,
        WriteStorage<'world, Position>,
        ReadStorage<'world, Decoration>,
        ReadStorage<'world, ComputedValues>,
    );
    fn run(&mut self, (text_list, mut positions, decorations, computed_values): Self::SystemData) {
        let mut break_points = vec![PageBreakOpportunity {
            before_index: 0,
            forced: false,
            advance_before: PixelLength(0.0),
            advance_after: PixelLength(0.0),
        }];

        let mut force_next_page_break = false;
        for (index, &elem) in text_list.list.iter().enumerate().skip(1) {
            let style = computed_values.get(elem.entity());
            let page_break_before = style.map(|s| s.page.page_break_before).unwrap_or_default();
            let page_break_after = style.map(|s| s.page.page_break_after).unwrap_or_default();

            let previous_elem = text_list.list[index - 1];

            let pos = positions.get(elem.entity()).unwrap();

            match (previous_elem, elem) {
                (BlockLevelElement::ContainerEnd(_), BlockLevelElement::ContainerStart(_))
                | (BlockLevelElement::Leaf(_), BlockLevelElement::Leaf(_)) => {
                    break_points.push(PageBreakOpportunity {
                        before_index: index,
                        forced: force_next_page_break || page_break_before == PageBreak::Always,
                        advance_before: pos.y,
                        advance_after: pos.y,
                    });
                    force_next_page_break = false;
                }
                _ => {}
            }

            match elem {
                BlockLevelElement::ContainerStart(_) => force_next_page_break = false,
                BlockLevelElement::ContainerEnd(_) | BlockLevelElement::Leaf(_) => {
                    if page_break_after == PageBreak::Always {
                        force_next_page_break = true;
                    }
                }
            }
        }

        // break_points.extend(text_list.list.windows(2).enumerate().filter_map(
        //     |(index, window)| match window {
        //         &[BlockLevelElement::ContainerEnd(end), BlockLevelElement::ContainerStart(start)] |
        //         &[BlockLevelElement::Leaf(end), BlockLevelElement::Leaf(start)]  =>
        //         {
        //             let end_style = computed_values.get(end);
        //             let page_break_after = end_style
        //                 .map(|s| s.page.page_break_after)
        //                 .unwrap_or_default();

        //             let start_style = computed_values.get(start);
        //             let page_break_before = start_style
        //                 .map(|s| s.page.page_break_before)
        //                 .unwrap_or_default();

        //             let forced = page_break_after == PageBreak::Always
        //                 || page_break_before == PageBreak::Always;

        //             Some(PageBreakOpportunity {
        //                 before_index: index + 1,
        //                 forced,
        //             })
        //         }
        //         _ => None,
        //     },
        // ));

        let content_height = text_list
            .list
            .last()
            .map(|last| positions.get(last.entity()).unwrap())
            .map(|pos| pos.y + pos.height)
            .unwrap_or_default();

        break_points.push(PageBreakOpportunity {
            before_index: text_list.list.len(),
            forced: false,
            advance_before: content_height,
            advance_after: content_height,
        });

        let optimal_sequence =
            super::paragraph::compute_breakpoints(break_points.len(), &mut |from, to| {
                let height = break_points[to].advance_after - break_points[from].advance_before;

                if height > self.content_height() {
                    std::f64::INFINITY
                } else {
                    (self.content_height().0 as f64 - height.0 as f64).powi(3)
                }
            });

        let mut optimal_sequence_iter = optimal_sequence.iter().map(|&index| break_points[index]);
        let mut next_break = optimal_sequence_iter.next();

        let mut current_page = 0;
        let mut y_offset = self.margin.top;
        for (index, &elem) in text_list.list.iter().enumerate() {
            match next_break {
                Some(PageBreakOpportunity {
                    before_index,
                    advance_after,
                    ..
                }) if before_index == index => {
                    current_page += 1;
                    y_offset = self.margin.top - advance_after;
                    next_break = optimal_sequence_iter.next();
                }
                _ => {}
            };
            if let Some(Position { page, y, .. }) = positions.get_mut(elem.entity()) {
                *page = current_page;
                *y = *y + y_offset;
            }
        }

        // let mut page_state = PageState::new(&text_list.list);

        // while let Some(elem) = page_state.take_element() {
        //     let style = computed_values.get(elem.entity());
        //     let page_break_before = style.map(|s| s.page.page_break_before).unwrap_or_default();
        //     let page_break_after = style.map(|s| s.page.page_break_after).unwrap_or_default();

        //     match elem {
        //         BlockLevelElement::ContainerStart(start) => {
        //             let decoration = decorations.get(start).unwrap();
        //             page_state.add_margin(decoration.margin.top);
        //             page_state.add_content(decoration.border.top);
        //             page_state.add_content(decoration.padding.top);

        //             if page_state.vertical_pos() > self.content_height()
        //                 || page_break_before == PageBreak::Always
        //             {
        //                 page_state.new_page()
        //             }

        //             page_state.in_progress_containers.start_container(start);
        //             let position = positions.get_mut(start).unwrap();
        //             position.y = self.margin.top + page_state.vertical_pos;
        //             position.page = page_state.page_num;
        //         }
        //         BlockLevelElement::ContainerEnd(end) => {
        //             let decoration = decorations.get(end).unwrap();
        //             page_state.add_content(decoration.padding.bottom);
        //             page_state.add_content(decoration.border.bottom);
        //             page_state.add_margin(decoration.margin.bottom);

        //             if page_state.vertical_pos() > self.content_height()
        //                 || page_break_after == PageBreak::Always
        //             {
        //                 page_state.new_page()
        //             }

        //             let position = positions.get_mut(end).unwrap();
        //             position.height = page_state.vertical_pos + self.margin.top - position.y;

        //             let container = page_state.in_progress_containers.end_container();
        //             assert_eq!(container.unwrap().root_entity, end);
        //         }
        //         BlockLevelElement::Leaf(leaf) => {
        //             let position = positions.get_mut(leaf).unwrap();

        //             position.y = self.margin.top + page_state.vertical_pos;
        //             page_state.add_content(position.height);
        //             if page_state.vertical_pos() > self.content_height() {
        //                 page_state.new_page();
        //                 position.y = self.margin.top + page_state.vertical_pos;
        //                 page_state.add_content(position.height);
        //             }

        //             position.page = page_state.page_num;
        //         }
        //     };
        // }
    }
}

// fn page_weight<'a>(range: impl std::ops::RangeBounds<usize>, state: &mut PageState<'a>) {}
