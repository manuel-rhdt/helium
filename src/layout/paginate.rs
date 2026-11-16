use super::components::{
    AbsoluteGroups, BlockLevelElement, Decoration, DecorationFragment, EdgeValues, Fragmented,
    PageGeometry, PageGroupBox, Position, TextList,
};
use super::inline::InlineFormattingContext;
use super::pages::{PageContext, PageMargins, Pages};
use crate::style::properties::computed::{ComputedValues, Display, PixelLength};
use helium_paginate::{
    BreakKind, CssObjective, ExactSolver, GalleyFlow, Kernel, PageSpec, PageTable, Solver,
};
use specs::{Entities, Entity, Join, Read, ReadStorage, System, Write, WriteStorage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Paginator {
    pub page_height: PixelLength,
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

/// The pagination problem of the galley: the break points as a flow of items, and the break
/// point that every place where a page can start stands for.
struct GalleyProblem {
    problem: helium_paginate::Problem,
    service: helium_paginate::GalleyService,
    break_point_at: HashMap<helium_paginate::Position, usize>,
}

fn in_paragraph(item: usize, line: usize) -> helium_paginate::Position {
    helium_paginate::Position::InParagraph { item, cut: helium_paginate::Cut(line as u32) }
}

/// The run of breaks between the lines of a paragraph that starts at `break_points[first]`: the
/// number of breaks, if the run has a break between every two lines of the paragraph and they
/// all have the same properties.
fn paragraph_run(break_points: &[PageBreakOpportunity], first: usize) -> Option<usize> {
    let head = break_points.get(first)?;
    let lines = head.in_lines?;
    if lines.before != 1 {
        return None;
    }
    let count = lines.total - 1;
    for number in 1..=count {
        let point = break_points.get(first + number - 1)?;
        let same = point.in_lines.is_some_and(|other| {
            other.block == lines.block
                && other.before == number
                && other.total == lines.total
                && other.orphans == lines.orphans
                && other.widows == lines.widows
        });
        if !same || point.forced || point.avoid != head.avoid {
            return None;
        }
    }
    Some(count)
}

/// The break points of the galley as a flow of boxes, paragraphs and breaks.
///
/// The extent of every box and of every line of a paragraph is the distance of the places where
/// a page that ends there and the one that starts there begin (`advance_after`), so the
/// extent of a page is the difference of two positions of the galley.
fn galley_problem(
    break_points: &[PageBreakOpportunity],
    content_heights: &[PixelLength],
    inline_size: f32,
    rtl: bool,
) -> GalleyProblem {
    let extent = |from: usize| {
        f64::from(break_points[from + 1].advance_after.0) - f64::from(break_points[from].advance_after.0)
    };
    let mut flow = GalleyFlow::new();
    // the place where a page that starts at the break point starts
    let mut places = vec![helium_paginate::Position::Item(0)];
    let mut index = 0;
    while index + 1 < break_points.len() {
        match paragraph_run(break_points, index + 1) {
            Some(count) => {
                let lines: Vec<f64> = (index..=index + count).map(extent).collect();
                let lines_in = break_points[index + 1].in_lines.unwrap();
                let avoid_inside = break_points[index + 1].avoid;
                let item = flow.len();
                flow.paragraph(&lines, lines_in.orphans as u32, lines_in.widows as u32, avoid_inside);
                for line in 1..=count {
                    places.push(in_paragraph(item, line));
                }
                index += count + 1;
            }
            None => {
                flow.content(extent(index));
                index += 1;
            }
        }
        let point = &break_points[index];
        if index + 1 < break_points.len() {
            if point.in_lines.is_some() {
                log::warn!("a break in a paragraph that is not part of a complete run");
            }
            flow.break_(if point.forced {
                BreakKind::Forced
            } else if point.avoid {
                BreakKind::Avoid
            } else {
                BreakKind::Allowed
            });
        }
        places.push(helium_paginate::Position::Item(flow.len()));
    }

    // COMPATIBILITY, to be removed in step 2 of docs/pagination.md: The paginator has always
    // guessed the page class (`PageMargins::edges_of_page_start`) and treated the number of a
    // break point as a page number, so the geometry of a page is a function of the place where
    // it starts alone. The table is filled with those guesses, and the kernel's page kind is
    // ignored by it. The exact solver then handles the page class no better than the code it
    // replaces. Likewise `stop_at_forced` is off, since the paginator never stopped there.
    let mut break_point_at = HashMap::new();
    let mut specs = HashMap::new();
    for (index, &place) in places.iter().enumerate() {
        specs.insert(place, PageSpec { inline_size, block_size: content_heights[index].0 });
        break_point_at.insert(place, index);
    }
    let pages = PageTable::new(rtl, move |place, _| specs[&place]).left_right_differ(false);
    let (problem, service) = flow.finish(pages);
    GalleyProblem { problem, service, break_point_at }
}

/// The break points at which the pages after the first start.
fn choose_breaks(galley: &mut GalleyProblem) -> Vec<usize> {
    let mut kernel = Kernel::new(&galley.problem, &mut galley.service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    debug_assert!(kernel.realize(&solution).is_ok());
    solution
        .pages
        .iter()
        .skip(1)
        .map(|start| galley.break_point_at[&kernel.position(start.at)])
        .collect()
}

impl<'world> System<'world> for Paginator {
    type SystemData = (
        Entities<'world>,
        Read<'world, TextList>,
        Read<'world, AbsoluteGroups>,
        Read<'world, PageMargins>,
        Read<'world, PageGeometry>,
        Write<'world, Pages>,
        WriteStorage<'world, Position>,
        WriteStorage<'world, Decoration>,
        WriteStorage<'world, Fragmented>,
        WriteStorage<'world, DecorationFragment>,
        ReadStorage<'world, PageGroupBox>,
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
            geometry,
            mut pages,
            mut positions,
            mut decorations,
            mut fragmented,
            mut fragments,
            group_boxes,
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
                .or_else(|| group_boxes.get(entity).map(|group| group.name.clone()))
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

        // The margins of the page that starts at every break point, which give the height that is
        // available to the content of the page. The first page of the document and the first
        // page of a page group can have other margins than the pages that follow them.
        let leaf_name = |elem: BlockLevelElement| page_name(elem).flatten();
        let name_after = |index: usize| {
            text_list.list[index.min(text_list.list.len())..]
                .iter()
                .find(|elem| matches!(elem, BlockLevelElement::Leaf(_)))
                .and_then(|&elem| leaf_name(elem))
        };
        let name_before = |index: usize| {
            text_list.list[..index.min(text_list.list.len())]
                .iter()
                .rev()
                .find(|elem| matches!(elem, BlockLevelElement::Leaf(_)))
                .and_then(|&elem| leaf_name(elem))
        };
        let page_start_edges: Vec<EdgeValues<PixelLength>> = break_points
            .iter()
            .enumerate()
            .map(|(number, point)| {
                let name = name_after(point.before_index);
                let first_in_group = name.is_some()
                    && (number == 0 || (point.forced && name_before(point.before_index) != name));
                page_margins.edges_of_page_start(number == 0, name, first_in_group)
            })
            .collect();
        let content_heights: Vec<PixelLength> = page_start_edges
            .iter()
            .map(|edges| self.page_height - edges.top - edges.bottom)
            .collect();

        let inline_size = (geometry.width - geometry.margin.left - geometry.margin.right).0;
        let mut galley = galley_problem(&break_points, &content_heights, inline_size, page_margins.rtl);
        let chosen_breaks = choose_breaks(&mut galley);

        let mut optimal_sequence_iter = chosen_breaks.iter().map(|&index| break_points[index]);
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
            rtl: page_margins.rtl,
        };
        let mut group_source = first_source;
        // The content of a page group that was laid out for the margins of its page is not moved.
        let shift = |page: &PageContext, open: &[Entity], index: usize| {
            let in_group_box = open.iter().any(|&entity| group_boxes.contains(entity))
                || text_list.list[index..]
                    .iter()
                    .map_while(|elem| match elem {
                        BlockLevelElement::ContainerStart(entity) => Some(*entity),
                        _ => None,
                    })
                    .any(|entity| group_boxes.contains(entity));
            if in_group_box {
                PixelLength(0.0)
            } else {
                page_margins.shift(page)
            }
        };
        let mut x_offset = shift(&first_page, &[], 0);
        let mut x_offsets = vec![x_offset];
        pages.0 = vec![first_page];
        let first_edges = page_margins.edges(&pages.0[0]);
        let mut y_offset = first_edges.top;
        // the vertical offset that was applied to the contents of every page, and the top and
        // the bottom of its content area
        let mut page_offsets = vec![y_offset];
        let mut page_areas = vec![(first_edges.top, self.page_height - first_edges.bottom)];

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
                        rtl: page_margins.rtl,
                    };
                    group_source = source;
                    x_offset = shift(&page, &open_containers, index);
                    x_offsets.push(x_offset);
                    pages.0.push(page);
                    let edges = page_margins.edges(&pages.0[current_page]);
                    y_offset = edges.top - advance_after;
                    page_offsets.push(y_offset);
                    page_areas.push((edges.top, self.page_height - edges.bottom));
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
                    page_areas[page].0
                };
                let bottom = if is_last {
                    bottom_edge + page_offsets[page]
                } else {
                    page_areas[page].1
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
        // (the ones in an `inline-block` box are placed with the box, once its line is known)
        let in_atom = |mut group: usize| {
            while let Some(parent) = absolute_groups.groups[group].parent {
                if absolute_groups.groups[parent].owner.is_some() {
                    return true;
                }
                if absolute_groups.groups[parent].anchor.is_some() {
                    // placed with the box it is anchored to
                    return false;
                }
                group = parent;
            }
            false
        };
        for (index, group) in absolute_groups.groups.iter().enumerate() {
            if group.owner.is_some() || group.anchor.is_some() || in_atom(index) {
                continue;
            }
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
    use helium_paginate::{Item, Position};
    use specs::{Builder, World, WorldExt};

    fn point(advance: f32) -> PageBreakOpportunity {
        PageBreakOpportunity::start(PixelLength(advance))
    }

    fn break_in(block: Entity, advance: f32, before: usize, total: usize) -> PageBreakOpportunity {
        PageBreakOpportunity {
            in_lines: Some(BreakInLines { block, before, total, orphans: 2, widows: 3 }),
            ..point(advance)
        }
    }

    fn problem(points: &[PageBreakOpportunity]) -> GalleyProblem {
        let heights = vec![PixelLength(100.0); points.len()];
        galley_problem(points, &heights, 300.0, false)
    }

    #[test]
    fn the_breaks_between_the_lines_of_a_paragraph_become_one_item() {
        let mut world = World::new();
        let block = world.create_entity().build();
        let points = [
            point(0.0),
            point(10.0),
            break_in(block, 30.0, 1, 4),
            break_in(block, 50.0, 2, 4),
            break_in(block, 70.0, 3, 4),
            point(90.0),
            point(100.0),
        ];
        let galley = problem(&points);
        assert!(matches!(
            galley.problem.items[..],
            [
                Item::Box { .. },
                Item::Break { kind: BreakKind::Allowed },
                Item::Paragraph { orphans: 2, widows: 3, avoid_inside: false, .. },
                Item::Break { kind: BreakKind::Allowed },
                Item::Box { .. },
            ]
        ));
        // every break point is a place where a page can start
        let mut places: Vec<_> = galley.break_point_at.iter().map(|(&at, &index)| (index, at)).collect();
        places.sort_by_key(|&(index, _)| index);
        let places: Vec<_> = places.into_iter().map(|(_, at)| at).collect();
        assert_eq!(
            places,
            [
                Position::Item(0),
                Position::Item(2),
                in_paragraph(2, 1),
                in_paragraph(2, 2),
                in_paragraph(2, 3),
                Position::Item(4),
                Position::Item(5),
            ]
        );
    }

    #[test]
    fn a_page_of_the_galley_is_the_difference_of_two_positions() {
        let points = [point(0.0), point(10.1), point(30.3), point(30.3 + 0.7)];
        let mut galley = problem(&points);
        let mut kernel = Kernel::new(&galley.problem, &mut galley.service);
        let first = kernel.first_page();
        let used: Vec<f32> = kernel.candidates(&first).iter().map(|fill| fill.used).collect();
        let expected = [10.1f32 - 0.0, 30.3 - 0.0, (30.3f32 + 0.7) - 0.0];
        assert_eq!(used, expected);
    }

    #[test]
    fn forced_and_avoid_breaks_keep_their_kind() {
        let points = [
            point(0.0),
            PageBreakOpportunity { forced: true, ..point(10.0) },
            PageBreakOpportunity { avoid: true, ..point(20.0) },
            point(30.0),
        ];
        let galley = problem(&points);
        assert!(matches!(
            galley.problem.items[..],
            [
                Item::Box { .. },
                Item::Break { kind: BreakKind::Forced },
                Item::Box { .. },
                Item::Break { kind: BreakKind::Avoid },
                Item::Box { .. },
            ]
        ));
    }

    #[test]
    fn breaks_inside_a_paragraph_with_other_properties_are_not_a_run() {
        let mut world = World::new();
        let block = world.create_entity().build();
        // the second break is avoided, the first is not
        let points = [
            point(0.0),
            break_in(block, 10.0, 1, 3),
            PageBreakOpportunity { avoid: true, ..break_in(block, 20.0, 2, 3) },
            point(30.0),
        ];
        assert_eq!(paragraph_run(&points, 1), None);
        // a paragraph that starts before its first line is not a run either
        let points = [point(0.0), break_in(block, 10.0, 2, 3), point(30.0)];
        assert_eq!(paragraph_run(&points, 1), None);
    }
}
