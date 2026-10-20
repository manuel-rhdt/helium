use super::components::{
    AbsoluteGroups, BlockLevelElement, Decoration, EdgeValues, Position, TextList,
};
use crate::style::properties::computed::{ComputedValues, PageBreak, PixelLength};
use specs::{Read, ReadStorage, System, WriteStorage};
use std::collections::HashSet;

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
        Read<'world, AbsoluteGroups>,
        WriteStorage<'world, Position>,
        ReadStorage<'world, Decoration>,
        ReadStorage<'world, ComputedValues>,
    );
    fn run(
        &mut self,
        (text_list, absolute_groups, mut positions, _decorations, computed_values): Self::SystemData,
    ) {
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
                // never part of the text list
                BlockLevelElement::Absolute(_) => {}
                BlockLevelElement::ContainerEnd(_) | BlockLevelElement::Leaf(_) => {
                    if page_break_after == PageBreak::Always {
                        force_next_page_break = true;
                    }
                }
            }
        }

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
                    if break_points[to].forced {
                        return std::f64::NEG_INFINITY;
                    }
                    (self.content_height().0 as f64 - height.0 as f64).powi(3)
                }
            });

        let mut optimal_sequence_iter = optimal_sequence
            .iter()
            .skip(1)
            .map(|&index| break_points[index]);
        let mut next_break = optimal_sequence_iter.next();

        let mut current_page = 0;
        let mut y_offset = self.margin.top;
        // the vertical offset that was applied to the contents of every page
        let mut page_offsets = vec![y_offset];

        // we keep track of which entities we already processed
        let mut processed_entities = HashSet::new();

        // TODO: Split block containers appropriately
        for (index, &elem) in text_list.list.iter().enumerate() {
            match next_break {
                Some(PageBreakOpportunity {
                    before_index,
                    advance_after,
                    ..
                }) if before_index == index => {
                    current_page += 1;
                    y_offset = self.margin.top - advance_after;
                    page_offsets.push(y_offset);
                    next_break = optimal_sequence_iter.next();
                }
                _ => {}
            };

            if processed_entities.insert(elem.entity()) {
                if let Some(Position { page, y, .. }) = positions.get_mut(elem.entity()) {
                    *page = current_page;
                    *y = *y + y_offset;
                }
            }
        }

        // Absolutely positioned boxes are not part of the text list. They are placed on the page
        // of their containing block, which for the initial containing block is the first page.
        for group in &absolute_groups.groups {
            let page = group
                .containing_block
                .and_then(|cb| positions.get(cb))
                .map_or(0, |pos| pos.page);
            let offset = page_offsets[page];
            for &entity in &group.entities {
                if let Some(pos) = positions.get_mut(entity) {
                    pos.page = page;
                    pos.y += offset;
                }
            }
        }
    }
}
