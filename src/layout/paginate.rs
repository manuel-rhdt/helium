use super::components::{
    AbsoluteGroups, BlockLevelElement, Decoration, DecorationFragment, EdgeValues, Fragmented,
    Position, TextList,
};
use crate::style::properties::computed::{ComputedValues, Display, PageBreak, PixelLength};
use specs::{Entities, Entity, Read, ReadStorage, System, WriteStorage};
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
        Entities<'world>,
        Read<'world, TextList>,
        Read<'world, AbsoluteGroups>,
        WriteStorage<'world, Position>,
        WriteStorage<'world, Decoration>,
        WriteStorage<'world, Fragmented>,
        WriteStorage<'world, DecorationFragment>,
        ReadStorage<'world, ComputedValues>,
    );
    fn run(
        &mut self,
        (
            entities,
            text_list,
            absolute_groups,
            mut positions,
            mut decorations,
            mut fragmented,
            mut fragments,
            computed_values,
        ): Self::SystemData,
    ) {
        let mut break_points = vec![PageBreakOpportunity {
            before_index: 0,
            forced: false,
            advance_before: PixelLength(0.0),
            advance_after: PixelLength(0.0),
        }];

        let mut force_next_page_break = false;
        // > 0 inside of boxes that are not broken across pages
        let mut unbreakable_depth = 0usize;
        // For every table that is open at the current element: the bottom border edge of the
        // lowest cell seen so far. A table may only break between two rows if no cell spans
        // across them (`rowspan`).
        let mut open_tables: Vec<PixelLength> = vec![];
        for (index, &elem) in text_list.list.iter().enumerate().skip(1) {
            let style = computed_values.get(elem.entity());
            let display = style.map(|s| s.box_.display);
            // Flex and grid containers are kept on one page, and so are table rows (with their
            // cells): a table breaks only between its rows.
            // TODO: allow breaks between the lines of flex containers and the rows of grids
            let is_monolithic = display.is_some_and(|d| {
                d.is_flex_or_grid() || matches!(d, Display::TableRow | Display::TableCell)
            });
            let is_table = display.is_some_and(|d| d.is_table());
            let page_break_before = style.map(|s| s.page.page_break_before).unwrap_or_default();
            let page_break_after = style.map(|s| s.page.page_break_after).unwrap_or_default();

            let previous_elem = text_list.list[index - 1];

            let pos = positions.get(elem.entity()).unwrap();

            match (previous_elem, elem) {
                (BlockLevelElement::ContainerEnd(_), BlockLevelElement::ContainerStart(_))
                | (BlockLevelElement::Leaf(_), BlockLevelElement::Leaf(_))
                    if unbreakable_depth == 0
                        && open_tables
                            .last()
                            .is_none_or(|&cells_bottom| cells_bottom.0 <= pos.y.0 + 0.01) =>
                {
                    let forced = force_next_page_break || page_break_before == PageBreak::Always;
                    // After a forced break the margins after the break are kept, so the next
                    // page starts where the content in front of the break ends. After an
                    // unforced break they are truncated.
                    // https://www.w3.org/TR/css-break-3/#break-margins
                    let advance_after = if forced {
                        let previous = previous_elem.entity();
                        let bottom = positions.get(previous).map(|p| p.y + p.height);
                        let extra = match previous_elem {
                            BlockLevelElement::ContainerEnd(_) => decorations
                                .get(previous)
                                .map(|d| d.padding.bottom + d.border.bottom)
                                .unwrap_or_default(),
                            _ => PixelLength(0.0),
                        };
                        bottom.map_or(pos.y, |bottom| PixelLength((bottom + extra).0.min(pos.y.0)))
                    } else {
                        pos.y
                    };
                    break_points.push(PageBreakOpportunity {
                        before_index: index,
                        forced,
                        advance_before: pos.y,
                        advance_after,
                    });
                    force_next_page_break = false;
                }
                _ => {}
            }

            match elem {
                BlockLevelElement::ContainerStart(_) => {
                    force_next_page_break = false;
                    if is_monolithic {
                        unbreakable_depth += 1;
                    }
                    if is_table {
                        open_tables.push(PixelLength(f32::NEG_INFINITY));
                    }
                }
                // never part of the text list
                BlockLevelElement::Absolute(_) => {}
                BlockLevelElement::ContainerEnd(entity) | BlockLevelElement::Leaf(entity) => {
                    let is_end = matches!(elem, BlockLevelElement::ContainerEnd(_));
                    if is_monolithic && is_end {
                        unbreakable_depth = unbreakable_depth.saturating_sub(1);
                    }
                    if is_end && is_table {
                        open_tables.pop();
                    }
                    if is_end && display == Some(Display::TableCell) {
                        if let Some(cells_bottom) = open_tables.last_mut() {
                            let extra = decorations
                                .get(entity)
                                .map(|d| d.padding.bottom + d.border.bottom)
                                .unwrap_or_default();
                            let bottom = pos.y + pos.height + extra;
                            if bottom.0 > cells_bottom.0 {
                                *cells_bottom = bottom;
                            }
                        }
                    }
                    if page_break_after == PageBreak::Always && unbreakable_depth == 0 {
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

        let last_break_point = break_points.len() - 1;
        let optimal_sequence =
            super::paragraph::compute_breakpoints(break_points.len(), &mut |from, to| {
                let height = break_points[to].advance_after - break_points[from].advance_before;

                if height > self.content_height() {
                    if to == from + 1 {
                        // There is no break opportunity in between: an unbreakable box that is
                        // taller than a page gets a page of its own and overflows it, so that
                        // the content after it is not lost.
                        1e30
                    } else {
                        std::f64::INFINITY
                    }
                } else {
                    if break_points[to].forced {
                        return std::f64::NEG_INFINITY;
                    }
                    if to == last_break_point {
                        // the last page is allowed to be partially filled
                        return 0.0;
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

        // the page of every element in the text list
        let mut page_of_index = Vec::with_capacity(text_list.list.len());

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

            page_of_index.push(current_page);

            if processed_entities.insert(elem.entity()) {
                if let Some(Position { page, y, .. }) = positions.get_mut(elem.entity()) {
                    *page = current_page;
                    *y = *y + y_offset;
                }
            }
        }

        // Boxes that contain a page break are split: every page gets the part of the decoration
        // that is on that page. A box has no border or padding at the edges of a break.
        let mut open: Vec<(Entity, usize)> = vec![];
        let mut to_split = vec![];
        for (index, &elem) in text_list.list.iter().enumerate() {
            match elem {
                BlockLevelElement::ContainerStart(entity) => open.push((entity, index)),
                BlockLevelElement::ContainerEnd(entity) => {
                    if let Some((start_entity, start)) = open.pop() {
                        debug_assert_eq!(start_entity, entity);
                        if page_of_index[start] != page_of_index[index] {
                            to_split.push((entity, page_of_index[start], page_of_index[index]));
                        }
                    }
                }
                _ => {}
            }
        }
        let page_top = self.margin.top;
        let page_bottom = self.margin.top + self.content_height();
        for (entity, first_page, last_page) in to_split {
            let (pos, deco) = match (
                positions.get(entity).copied(),
                decorations.get(entity).copied(),
            ) {
                (Some(pos), Some(deco)) => (pos, deco),
                _ => continue,
            };
            let visible = deco.background_color.alpha > 0
                || deco.border.top > PixelLength(0.0)
                || deco.border.bottom > PixelLength(0.0)
                || deco.border.left > PixelLength(0.0)
                || deco.border.right > PixelLength(0.0);
            if !visible {
                continue;
            }
            let layout_y = pos.y - page_offsets[first_page];
            let top_edge = layout_y - deco.padding.top - deco.border.top;
            let bottom_edge = layout_y + pos.height + deco.padding.bottom + deco.border.bottom;
            fragmented.insert(entity, Fragmented).unwrap();
            for page in first_page..=last_page {
                let is_first = page == first_page;
                let is_last = page == last_page;
                let top = if is_first {
                    top_edge + page_offsets[page]
                } else {
                    page_top
                };
                let bottom = if is_last {
                    bottom_edge + page_offsets[page]
                } else {
                    page_bottom
                };
                let mut fragment_deco = deco;
                if !is_first {
                    fragment_deco.padding.top = PixelLength(0.0);
                    fragment_deco.border.top = PixelLength(0.0);
                    if let Some(edges) = &mut fragment_deco.collapsed {
                        edges.top.paint = false;
                        edges.left.extend_start = PixelLength(0.0);
                        edges.right.extend_start = PixelLength(0.0);
                    }
                }
                if !is_last {
                    fragment_deco.padding.bottom = PixelLength(0.0);
                    fragment_deco.border.bottom = PixelLength(0.0);
                    if let Some(edges) = &mut fragment_deco.collapsed {
                        edges.bottom.paint = false;
                        edges.left.extend_end = PixelLength(0.0);
                        edges.right.extend_end = PixelLength(0.0);
                    }
                }
                let content_top = top + fragment_deco.border.top + fragment_deco.padding.top;
                let content_bottom =
                    bottom - fragment_deco.border.bottom - fragment_deco.padding.bottom;
                let height = if content_bottom > content_top {
                    content_bottom - content_top
                } else {
                    PixelLength(0.0)
                };
                entities
                    .build_entity()
                    .with(
                        Position {
                            y: content_top,
                            height,
                            page,
                            ..pos
                        },
                        &mut positions,
                    )
                    .with(fragment_deco, &mut decorations)
                    .with(DecorationFragment, &mut fragments)
                    .build();
            }
        }

        // Absolutely positioned boxes are not part of the text list. They are placed on the page
        // of their containing block, which for the initial containing block is the first page.
        for group in absolute_groups
            .groups
            .iter()
            .filter(|group| group.owner.is_none())
        {
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
