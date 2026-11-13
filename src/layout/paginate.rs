use super::components::{
    AbsoluteGroups, BlockLevelElement, Decoration, DecorationFragment, EdgeValues, Fragmented,
    Position, TextList,
};
use super::inline::InlineFormattingContext;
use super::pages::{PageContext, PageMargins, Pages};
use crate::style::properties::computed::{ComputedValues, Display, PixelLength};
use specs::{Entities, Entity, Join, Read, ReadStorage, System, Write, WriteStorage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The cost of a page break at a place where a break should be avoided (`break-inside: avoid`
/// or `break-before`/`break-after: avoid`). It is larger than the cost of any page that is
/// merely not full, so a break is only avoided if the content fits otherwise, but finite so
/// that content that does not fit on a page is broken anyway.
const AVOID_BREAK_COST: f64 = 1e12;
/// The cost of a page that ends at a forced break; it outweighs all other costs.
const FORCED_BREAK_COST: f64 = -1e15;
/// The cost of a page that has fewer lines of a paragraph at its end than `orphans` or at its
/// start than `widows`. Violating these is preferred to violating an `avoid` value.
const ORPHANS_WIDOWS_COST: f64 = 1e11;

pub struct Paginator {
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

impl Paginator {
    fn content_height(&self) -> PixelLength {
        self.page_height - self.margin.top - self.margin.bottom
    }
}

/// A break between two lines of a paragraph.
#[derive(Debug, Copy, Clone)]
struct BreakInLines {
    /// The block with the lines.
    block: Entity,
    /// The number of lines in front of the break.
    before: usize,
    total: usize,
    orphans: usize,
    widows: usize,
}

#[derive(Debug, Copy, Clone)]
struct PageBreakOpportunity {
    before_index: usize,
    forced: bool,
    /// The break is discouraged by `break-before`, `break-after` or `break-inside`.
    avoid: bool,
    /// Set for breaks inside a paragraph.
    in_lines: Option<BreakInLines>,
    advance_before: PixelLength,
    advance_after: PixelLength,
}

impl PageBreakOpportunity {
    fn start(advance: PixelLength) -> Self {
        PageBreakOpportunity {
            before_index: 0,
            forced: false,
            avoid: false,
            in_lines: None,
            advance_before: advance,
            advance_after: advance,
        }
    }
}

/// Whether the page between two breaks has too few lines of a paragraph at its start or end.
fn violates_orphans_or_widows(from: &PageBreakOpportunity, to: &PageBreakOpportunity) -> bool {
    if let Some(end) = to.in_lines {
        let start = from
            .in_lines
            .filter(|start| start.block == end.block)
            .map_or(0, |start| start.before);
        if end.before - start < end.orphans {
            return true;
        }
    }
    if let Some(start) = from.in_lines {
        let end = to
            .in_lines
            .filter(|end| end.block == start.block)
            .map_or(start.total, |end| end.before);
        if end - start.before < start.widows {
            return true;
        }
    }
    false
}

impl<'world> System<'world> for Paginator {
    type SystemData = (
        Entities<'world>,
        Read<'world, TextList>,
        Read<'world, AbsoluteGroups>,
        Read<'world, PageMargins>,
        Write<'world, Pages>,
        WriteStorage<'world, Position>,
        WriteStorage<'world, Decoration>,
        WriteStorage<'world, Fragmented>,
        WriteStorage<'world, DecorationFragment>,
        ReadStorage<'world, ComputedValues>,
        ReadStorage<'world, InlineFormattingContext>,
    );
    fn run(
        &mut self,
        (
            entities,
            text_list,
            absolute_groups,
            page_margins,
            mut pages,
            mut positions,
            mut decorations,
            mut fragmented,
            mut fragments,
            computed_values,
            inline_contexts,
        ): Self::SystemData,
    ) {
        let mut break_points = vec![PageBreakOpportunity::start(PixelLength(0.0))];

        // the paragraph, number and count of the lines in the text list
        let mut line_info: HashMap<Entity, (Entity, usize, usize)> = HashMap::new();
        for (owner, context) in (&entities, &inline_contexts).join() {
            for (number, line) in context.lines.iter().enumerate() {
                line_info.insert(line.entity, (owner, number, context.lines.len()));
            }
        }

        // The name of the page that an element is printed on (the `page` property), if the
        // element has a style. Lines have the page name of their paragraph.
        let page_name = |elem: BlockLevelElement| {
            let entity = elem.entity();
            computed_values
                .get(entity)
                .or_else(|| {
                    line_info
                        .get(&entity)
                        .and_then(|&(owner, ..)| computed_values.get(owner))
                })
                .map(|style| style.page.page.0.clone())
        };

        // The values of `break-after` of the boxes that end in front of the element at `index`
        // of the text list and of `break-before` of those that start there. They are the boxes
        // that end or start right at the break: a break value of a first or last child
        // propagates to its parent.
        // https://www.w3.org/TR/css-break-3/#break-propagation
        let list = &text_list.list;
        let break_values = |index: usize| -> (bool, bool) {
            let (mut forced, mut avoid) = (false, false);
            let mut add = |value: crate::style::properties::computed::Break| {
                forced |= value.is_forced();
                avoid |= value.is_avoid();
            };
            let mut at = index;
            while at > 0 {
                at -= 1;
                match list[at] {
                    BlockLevelElement::ContainerEnd(entity) => {
                        if let Some(style) = computed_values.get(entity) {
                            add(style.page.break_after);
                        }
                    }
                    BlockLevelElement::Leaf(entity) => {
                        if let Some(style) = computed_values.get(entity) {
                            add(style.page.break_after);
                        }
                        break;
                    }
                    _ => break,
                }
            }
            let mut at = index;
            while at < list.len() {
                match list[at] {
                    BlockLevelElement::ContainerStart(entity) => {
                        if let Some(style) = computed_values.get(entity) {
                            add(style.page.break_before);
                        }
                    }
                    BlockLevelElement::Leaf(entity) => {
                        if let Some(style) = computed_values.get(entity) {
                            add(style.page.break_before);
                        }
                        break;
                    }
                    _ => break,
                }
                at += 1;
            }
            (forced, avoid && !forced)
        };

        // > 0 inside of boxes that are not broken across pages
        let mut unbreakable_depth = 0usize;
        // > 0 inside of boxes with `break-inside: avoid`
        let mut avoid_inside_depth = 0usize;
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
            let avoids_inside = style.is_some_and(|s| s.page.break_inside.is_avoid());

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
                    let (mut forced, avoid_between) = break_values(index);
                    // a change of the page name forces a page break
                    if let (Some(before), Some(after)) = (page_name(previous_elem), page_name(elem))
                    {
                        forced |= before != after;
                    }
                    let in_lines = match (previous_elem, elem) {
                        (BlockLevelElement::Leaf(a), BlockLevelElement::Leaf(b)) => {
                            match (line_info.get(&a), line_info.get(&b)) {
                                (Some(&(block, number, total)), Some(&(other, next, _)))
                                    if block == other && next == number + 1 =>
                                {
                                    let page = computed_values.get(block).map(|s| &s.page);
                                    Some(BreakInLines {
                                        block,
                                        before: next,
                                        total,
                                        orphans: page.map_or(2, |p| p.orphans),
                                        widows: page.map_or(2, |p| p.widows),
                                    })
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
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
                        avoid: !forced && (avoid_between || avoid_inside_depth > 0),
                        in_lines,
                        advance_before: pos.y,
                        advance_after,
                    });
                }
                _ => {}
            }

            match elem {
                BlockLevelElement::ContainerStart(_) => {
                    if avoids_inside {
                        avoid_inside_depth += 1;
                    }
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
                    if avoids_inside && is_end {
                        avoid_inside_depth = avoid_inside_depth.saturating_sub(1);
                    }
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
            ..PageBreakOpportunity::start(content_height)
        });

        let last_break_point = break_points.len() - 1;
        let optimal_sequence =
            super::paragraph::compute_breakpoints(break_points.len(), &mut |from, to| {
                // a page starts where the content before the break ends (`advance_after`), which keeps
                // the margin and padding that follow a forced break
                let height = break_points[to].advance_after - break_points[from].advance_after;

                if height > self.content_height() {
                    if to == from + 1 {
                        // There is no break opportunity in between: an unbreakable box that is
                        // taller than a page gets a page of its own and overflows it, so that
                        // the content after it is not lost.
                        1e30
                    } else {
                        f64::INFINITY
                    }
                } else {
                    // A forced break is always taken, but how the page in front of it is
                    // filled still matters for the widows of its last paragraph.
                    let mut cost = if break_points[to].forced {
                        FORCED_BREAK_COST
                    } else if to == last_break_point {
                        // the last page is allowed to be partially filled
                        0.0
                    } else {
                        (self.content_height().0 as f64 - height.0 as f64).powi(3)
                    };
                    if break_points[to].avoid {
                        cost += AVOID_BREAK_COST;
                    }
                    if violates_orphans_or_widows(&break_points[from], &break_points[to]) {
                        cost += ORPHANS_WIDOWS_COST;
                    }
                    cost
                }
            });

        let mut optimal_sequence_iter = optimal_sequence
            .iter()
            .skip(1)
            .map(|&index| break_points[index]);
        let mut next_break = optimal_sequence_iter.next();

        let mut current_page = 0;
        // the pages in the document, and how far the content of a page is moved sideways
        // because of the left and right margins of the page
        // The name of a page is the name of its first box: the first leaf at or after the element
        // that starts the page, with the boxes that are open around it.
        let first_leaf = |open: &[Entity], index: usize| {
            let mut around = open.to_vec();
            for &elem in &text_list.list[index..] {
                match elem {
                    BlockLevelElement::ContainerStart(entity) => around.push(entity),
                    BlockLevelElement::Leaf(_) => return Some((around, elem)),
                    _ => return None,
                }
            }
            None
        };
        // The box that the name of a page comes from: the outermost of the boxes around the first
        // element on the page that have that name. A page group is a run of pages of one such box.
        let page_start = |open: &[Entity], index: usize| -> (Option<Arc<str>>, Option<Entity>) {
            let Some((around, leaf)) = first_leaf(open, index) else {
                return (None, None);
            };
            let Some(name) = page_name(leaf).flatten() else {
                return (None, None);
            };
            let own = line_info.get(&leaf.entity()).map_or(leaf.entity(), |info| info.0);
            let mut source = own;
            for &entity in around.iter().rev() {
                let named = computed_values
                    .get(entity)
                    .is_some_and(|style| style.page.page.0.as_ref() == Some(&name));
                if !named {
                    break;
                }
                source = entity;
            }
            (Some(name), Some(source))
        };
        let mut open_containers: Vec<Entity> = vec![];
        let (first_name, first_source) = page_start(&open_containers, 0);
        let first_page = PageContext {
            index: 0,
            name: first_name,
            first_in_group: true,
        };
        let mut group_source = first_source;
        let mut x_offset = page_margins.shift(&first_page);
        let mut x_offsets = vec![x_offset];
        pages.0 = vec![first_page];
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
                    let (name, source) = page_start(&open_containers, index);
                    // a page starts a page group if the name comes from another box than on
                    // the page before
                    let page = PageContext {
                        index: current_page,
                        first_in_group: source.is_some() && source != group_source,
                        name,
                    };
                    group_source = source;
                    x_offset = page_margins.shift(&page);
                    x_offsets.push(x_offset);
                    pages.0.push(page);
                    y_offset = self.margin.top - advance_after;
                    page_offsets.push(y_offset);
                    next_break = optimal_sequence_iter.next();
                }
                _ => {}
            };

            page_of_index.push(current_page);
            match elem {
                BlockLevelElement::ContainerStart(entity) => open_containers.push(entity),
                BlockLevelElement::ContainerEnd(_) => {
                    open_containers.pop();
                }
                _ => {}
            }

            if processed_entities.insert(elem.entity()) {
                if let Some(Position { page, x, y, .. }) = positions.get_mut(elem.entity()) {
                    *page = current_page;
                    *x = *x + x_offset;
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
                || deco.has_background_image
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
                    fragment_deco.radius.top_left = Default::default();
                    fragment_deco.radius.top_right = Default::default();
                    if let Some(edges) = &mut fragment_deco.collapsed {
                        edges.top.paint = false;
                        edges.left.extend_start = PixelLength(0.0);
                        edges.right.extend_start = PixelLength(0.0);
                    }
                }
                if !is_last {
                    fragment_deco.padding.bottom = PixelLength(0.0);
                    fragment_deco.border.bottom = PixelLength(0.0);
                    fragment_deco.radius.bottom_left = Default::default();
                    fragment_deco.radius.bottom_right = Default::default();
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
                    .with(DecorationFragment(entity), &mut fragments)
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
                    pos.x += x_offsets[page];
                    pos.y += offset;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use specs::{Builder, World, WorldExt};

    fn break_in(block: Entity, before: usize, total: usize) -> PageBreakOpportunity {
        PageBreakOpportunity {
            in_lines: Some(BreakInLines {
                block,
                before,
                total,
                orphans: 2,
                widows: 2,
            }),
            ..PageBreakOpportunity::start(PixelLength(0.0))
        }
    }

    #[test]
    fn orphans_and_widows_count_the_lines_on_the_page() {
        let mut world = World::new();
        let block = world.create_entity().build();
        let start = PageBreakOpportunity::start(PixelLength(0.0));
        // a ten-line paragraph broken after 1, 2, 8 and 9 lines
        assert!(violates_orphans_or_widows(&start, &break_in(block, 1, 10)));
        assert!(!violates_orphans_or_widows(&start, &break_in(block, 2, 10)));
        assert!(!violates_orphans_or_widows(&break_in(block, 8, 10), &start));
        assert!(violates_orphans_or_widows(&break_in(block, 9, 10), &start));
    }

    #[test]
    fn a_page_in_the_middle_of_a_paragraph_counts_its_own_lines() {
        let mut world = World::new();
        let block = world.create_entity().build();
        // lines 4 to 5 of 12 are on the page: too few for both orphans and widows
        let from = break_in(block, 3, 12);
        assert!(!violates_orphans_or_widows(&from, &break_in(block, 5, 12)));
        assert!(violates_orphans_or_widows(&from, &break_in(block, 4, 12)));
    }

    #[test]
    fn breaks_in_other_blocks_do_not_interact() {
        let mut world = World::new();
        let (a, b) = (world.create_entity().build(), world.create_entity().build());
        // the page ends after the second line of b; the lines of a at its start are all there
        assert!(!violates_orphans_or_widows(&break_in(a, 8, 10), &break_in(b, 2, 5)));
    }
}
