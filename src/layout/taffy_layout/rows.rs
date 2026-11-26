//! Table rows that are split over pages.
//!
//! A row is one box of the flow, unless it may be taller than a page: then it is a container item
//! (`Item::Container`), the fragments of which the layout service finds ([`row_fragments`]). A row
//! is laid out whole with the table, and what is on a page is a part of it. The part is between
//! two *cuts*, offsets from the top of the row, measured in 1/64 pixel (the cut of the container
//! in the flow, `ContainerCut`).
//!
//! The cells of a row go on side by side, and each is cut where its own lines end: a piece of a
//! cell (a line of text, or a box that is not split) is on the fragment that its bottom is in,
//! so that no piece is cut in two, and a line across a cut goes on the next page, at its top, with
//! what follows it. A cell is therefore moved on its page by how much its first piece on the
//! page is above the cut ([`RowCuts::cell_origin`]), and the fragment is as high as its highest
//! cell ([`RowCuts::used`]). The fragments follow from the cuts alone, whatever the cuts before
//! them were.
//!
//! A grid container is cut the same way ([`RowCuts::stacked`]): its items are the cells, and they
//! are in more rows than one. An item that is not taller than half a page is one piece (it is not
//! cut, as a row of a table is not between its cells), a taller one is cut where its own lines
//! end. A cut that is between the rows has no item across it, and then the gap between the rows is
//! not at the top of the next page. Where a line is across a cut, the items that start behind it
//! are moved down with the ones that are moved to start at the cut.

use super::tree::{Content, UmbruchTree, lines_from};
use umbruch_paginate::{BreakKind, ContainerCut, Fragment};
use taffy::tree::NodeId;

/// The resolution of the cuts: an offset is a multiple of 1/64 pixel.
pub(super) const QUANT: f32 = 64.0;

/// The offset in 1/64 pixel.
pub(super) fn quantize(y: f32) -> u32 {
    (y.max(0.0) * QUANT).round() as u32
}

/// The offset in pixels.
pub(super) fn dequantize(offset: u32) -> f32 {
    offset as f32 / QUANT
}

/// The pieces of a cell that no cut may be in, by their top and their bottom from the top of the
/// row, in the order of their bottoms.
#[derive(Debug, Clone)]
pub(super) struct CellUnits {
    pub node: NodeId,
    pub units: Vec<(u32, u32)>,
    /// What each piece is, in the same order.
    pub marks: Vec<Mark>,
    /// Whether the cell is one piece that no cut is in (an item of a grid that is not taller than
    /// half a page).
    pub atomic: bool,
}

/// What a piece of a cell is: a line of a paragraph (which paragraph, the number of the line and
/// the number of lines, and the `orphans` and `widows` of it), or another piece (`paragraph` 0).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Mark {
    pub paragraph: u32,
    pub index: u32,
    pub count: u32,
    pub orphans: u32,
    pub widows: u32,
}

/// Where a row can be cut, for one width.
#[derive(Debug, Clone, Default)]
pub(super) struct RowCuts {
    /// The height of the row.
    pub height: u32,
    pub cells: Vec<CellUnits>,
    /// The offsets that a fragment can end at, in growing order, without the end of the row: the
    /// bottoms of the pieces of the cells.
    pub candidates: Vec<u32>,
    /// Whether the cells are the items of a grid, which are in rows of the grid below each other
    /// and not side by side in one row.
    pub stacked: bool,
}

impl RowCuts {
    /// How far the pieces that are across the cut `offset` are above it, at most (0: no piece is
    /// across it, and the cut is between the rows of a grid).
    pub fn crossing(&self, offset: u32) -> u32 {
        self.cells
            .iter()
            .filter_map(|cell| {
                let first = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
                cell.units.get(first).and_then(|&(top, _)| offset.checked_sub(top))
            })
            .max()
            .unwrap_or(0)
    }

    /// Where the part of the container that starts at the cut `offset` starts. In a grid that is
    /// the top of the first piece behind the cut if no piece is across the cut: the gap between
    /// two rows is not at the top of a page. In all other cases it is the cut.
    pub fn start(&self, offset: u32) -> u32 {
        if !self.stacked || offset == 0 || self.crossing(offset) > 0 {
            return offset;
        }
        self.cells
            .iter()
            .filter_map(|cell| {
                let first = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
                cell.units.get(first).map(|&(top, _)| top)
            })
            .min()
            .map_or(offset, |top| top.max(offset))
    }

    /// The cells that have a piece across the cut `offset`, as where the cell ends and how far its
    /// content is moved down.
    fn crossing_cells(&self, offset: u32) -> Vec<(u32, u32)> {
        self.cells
            .iter()
            .filter_map(|cell| {
                let first = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
                let &(top, _) = cell.units.get(first)?;
                let end = cell.units.last()?.1;
                (top < offset).then(|| (end, offset - top))
            })
            .collect()
    }

    /// How far a cell is moved down on the fragment that starts at the cut `offset`: its content
    /// (to start at the cut, where a piece is across it), and its box. The cells of a grid that
    /// are below a cell that is moved down are moved down with it, by the most of what the ones
    /// above are. `crossing` is [`RowCuts::crossing_cells`].
    fn shifts(&self, cell: &CellUnits, offset: u32, crossing: &[(u32, u32)]) -> Option<(u32, u32)> {
        let first = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
        let &(top, _) = cell.units.get(first)?;
        if top < offset {
            return Some((offset - top, 0));
        }
        let below = if self.stacked {
            crossing.iter().filter(|&&(end, _)| end <= top).map(|&(_, shift)| shift).max().unwrap_or(0)
        } else {
            0
        };
        Some((0, below))
    }

    /// How far the box of a cell that has no piece across the cut `offset` is moved down in a
    /// grid, so that it is below the cells that are moved down to start at the cut.
    pub fn box_shift(&self, node: NodeId, offset: u32) -> u32 {
        let Some(cell) = self.cells.iter().find(|cell| cell.node == node) else {
            return 0;
        };
        self.shifts(cell, offset, &self.crossing_cells(offset)).map_or(0, |(_, below)| below)
    }

    /// Where the part of a cell that starts at the cut `offset` starts: the top of its first
    /// piece that ends behind the cut. That is above the cut where a line is across it. At the
    /// start of the row it is the top of the row (the padding of the cell is part of it).
    pub fn cell_origin(&self, cell: &CellUnits, offset: u32) -> Option<u32> {
        if offset == 0 {
            return Some(0);
        }
        let first = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
        cell.units.get(first).map(|&(top, _)| top.min(offset))
    }

    /// The height of the fragment from the cut `start` to the cut `end` (the end of the row is
    /// `height`): the highest of the parts of the cells, each from where it starts to the bottom of
    /// its last piece that ends before the cut. The fragment that ends the row is as high as what
    /// is left of the row, from the part of a cell that starts highest.
    pub fn used(&self, start: u32, end: u32) -> u32 {
        if self.stacked {
            return self.used_stacked(start, end);
        }
        if end >= self.height {
            let highest = self.cells.iter().filter_map(|cell| self.cell_origin(cell, start)).min();
            return self.height.saturating_sub(highest.unwrap_or(start));
        }
        let mut used = 0;
        for cell in &self.cells {
            let Some(origin) = self.cell_origin(cell, start) else {
                continue;
            };
            let last = cell.units.partition_point(|&(_, bottom)| bottom <= end);
            // the pieces that end in (start, end]
            let first = cell.units.partition_point(|&(_, bottom)| bottom <= start);
            if last > first {
                used = used.max(cell.units[last - 1].1.saturating_sub(origin));
            }
        }
        used
    }

    /// [`RowCuts::used`] for a grid: the fragment starts where [`RowCuts::start`] says. A cell that
    /// has a piece across the cut is moved down by how far that is above it, and the others by
    /// the most of that.
    fn used_stacked(&self, start: u32, end: u32) -> u32 {
        let from = self.start(start);
        let crossing = self.crossing_cells(start);
        if end >= self.height {
            let spill = crossing.iter().map(|&(_, shift)| shift).max().unwrap_or(0);
            return (self.height + spill).saturating_sub(from);
        }
        let mut used = 0;
        for cell in &self.cells {
            let Some((content, below)) = self.shifts(cell, start, &crossing) else {
                continue;
            };
            let last = cell.units.partition_point(|&(_, bottom)| bottom <= end);
            let first = cell.units.partition_point(|&(_, bottom)| bottom <= start);
            if last > first {
                used = used.max((cell.units[last - 1].1 + content + below).saturating_sub(from));
            }
        }
        used
    }

    /// The part of a cell that is on the fragment from `start` to `end`, as how far the cell is
    /// moved down (to start where the fragment does) and where the part ends, from the top of the
    /// part of the row on the page (`None`: it goes to the end of the row).
    pub fn cell_part(&self, node: NodeId, start: u32, end: u32) -> Option<(u32, Option<u32>)> {
        let cell = self.cells.iter().find(|cell| cell.node == node)?;
        let shift = match self.cell_origin(cell, start) {
            Some(origin) => start.saturating_sub(origin),
            None => 0,
        };
        // (the cells of a grid that are not across the cut are moved down as a box)
        let moved = if self.stacked && shift == 0 { self.box_shift(node, start) } else { shift };
        let until = (end < self.height).then(|| end.saturating_sub(self.start(start)) + moved);
        Some((shift, until))
    }
}

impl RowCuts {
    /// Whether a fragment that ends at the cut `offset` leaves fewer lines of a paragraph of a
    /// cell in front of the cut than its `orphans`, or behind it than its `widows`.
    pub fn violations(&self, offset: u32) -> (bool, bool) {
        let (mut orphans, mut widows) = (false, false);
        for cell in &self.cells {
            let split = cell.units.partition_point(|&(_, bottom)| bottom <= offset);
            if split == 0 || split >= cell.units.len() {
                continue;
            }
            let (before, after) = (cell.marks[split - 1], cell.marks[split]);
            if before.paragraph == 0 || before.paragraph != after.paragraph {
                continue;
            }
            let in_front = before.index + 1;
            orphans |= in_front < before.orphans;
            widows |= before.count.saturating_sub(in_front) < before.widows;
        }
        (orphans, widows)
    }
}

impl UmbruchTree<'_, '_> {
    /// Where the row can be cut, as it is laid out now.
    ///
    /// `page` is the size of the page: a grid is cut inside of its items only where they are
    /// taller than half of it.
    pub(super) fn row_cuts(&mut self, row: NodeId, page: (f32, f32)) -> RowCuts {
        let stacked = self.node(row).style.display == taffy::Display::Grid;
        let height = quantize(self.node(row).layout.size.height);
        let mut cells = vec![];
        let mut candidates = vec![];
        for cell in self.node(row).children.clone() {
            let (hidden, top) = {
                let node = self.node(cell);
                (
                    node.style.display == taffy::Display::None
                        || node.style.position == taffy::Position::Absolute,
                    node.layout.location.y,
                )
            };
            if hidden {
                continue;
            }
            let mut found: Vec<(f32, f32, Mark)> = vec![];
            let cell_height = self.node(cell).layout.size.height;
            // (an item of a grid that is not taller than half of the page is not cut)
            let atomic = stacked && cell_height * 2.0 <= page.1;
            if atomic {
                found.push((top, top + cell_height, Mark::default()));
            } else {
                self.collect_units(cell, top, &mut found);
            }
            let mut pieces: Vec<((u32, u32), Mark)> = found
                .into_iter()
                .map(|(top, bottom, mark)| ((quantize(top), quantize(bottom)), mark))
                .filter(|&((top, bottom), _)| bottom > top)
                .collect();
            pieces.sort_by_key(|&((top, bottom), _)| (bottom, top));
            // (a single piece is one that no cut is in)
            let atomic = atomic || (stacked && pieces.len() <= 1);
            if stacked && !atomic {
                // the padding and the border of an item go with its first and its last piece (no
                // cut leaves them on a page of their own); the space that an item that is stretched
                // has below its content does not
                let layout = self.node(cell).layout;
                let (item_top, item_bottom) = (top, top + cell_height);
                if let Some(((first, _), _)) = pieces.first_mut() {
                    let edge = layout.padding.top + layout.border.top;
                    *first = (*first).min(quantize((dequantize(*first) - edge).max(item_top)));
                }
                if let Some(((_, last), _)) = pieces.last_mut() {
                    let edge = layout.padding.bottom + layout.border.bottom;
                    *last = (*last).max(quantize((dequantize(*last) + edge).min(item_bottom)));
                }
            }
            let (units, marks): (Vec<_>, Vec<_>) = pieces.into_iter().unzip();
            candidates.extend(units.iter().map(|&(_, bottom)| bottom).filter(|&bottom| bottom > 0 && bottom < height));
            cells.push(CellUnits { node: cell, units, marks, atomic });
        }
        if stacked {
            // no cut is inside of an item that is not cut, and none leaves nothing but the
            // padding of the grid for the next page
            let content_end = cells.iter().flat_map(|cell| cell.units.iter().map(|&(_, bottom)| bottom)).max().unwrap_or(0);
            let atomic: Vec<(u32, u32)> = cells.iter().filter(|cell| cell.atomic).flat_map(|cell| cell.units.iter().copied()).collect();
            candidates.retain(|&offset| {
                offset < content_end && !atomic.iter().any(|&(top, bottom)| top < offset && offset < bottom)
            });
        }
        candidates.sort_unstable();
        candidates.dedup();
        RowCuts { height, cells, candidates, stacked }
    }

    /// The pieces of the content of a box that no cut may be in, by their top and their bottom
    /// from the top of the row: the lines of the paragraphs, and the boxes that are not split.
    /// `top` is where the border box of the box is.
    fn collect_units(&mut self, id: NodeId, top: f32, units: &mut Vec<(f32, f32, Mark)>) {
        let (layout, content, entity, shift, whole, hidden) = {
            let node = self.node(id);
            let whole = node.columns.is_some()
                || node.orthogonal
                || node.flow.is_vertical()
                || !node.style.size.height.is_auto()
                || matches!(node.style.display, taffy::Display::Flex | taffy::Display::Grid);
            let hidden = node.style.display == taffy::Display::None
                || node.style.position == taffy::Position::Absolute;
            (node.layout, node.content, node.entity, node.content_shift, whole, hidden)
        };
        if hidden {
            return;
        }
        let bottom = top + layout.size.height;
        match content {
            Content::Replaced | Content::Table => units.push((top, bottom, Mark::default())),
            _ if whole => units.push((top, bottom, Mark::default())),
            Content::Inline => {
                let (line_width, indent) = super::service::paragraph_width(self, id);
                let heights: Vec<f32> = {
                    let tree = &mut *self;
                    match tree.inline_ctxs.get(entity) {
                        Some(ifc) => lines_from(
                            &mut tree.lines,
                            ifc,
                            entity.id(),
                            None,
                            crate::style::properties::computed::PixelLength(line_width),
                            crate::style::properties::computed::PixelLength(indent),
                            tree.text_runs,
                            tree.intrinsic_sizes,
                            tree.aligns,
                        )
                        .iter()
                        .map(|line| line.height.0)
                        .collect(),
                        None => vec![],
                    }
                };
                let (orphans, widows) = self
                    .computed_values
                    .get(entity)
                    .map_or((2, 2), |style| (style.page.orphans as u32, style.page.widows as u32));
                let count = heights.len() as u32;
                let mut y = top + layout.border.top + layout.padding.top + shift;
                for (index, height) in heights.into_iter().enumerate() {
                    let mark = Mark { paragraph: entity.id() + 1, index: index as u32, count, orphans, widows };
                    units.push((y, y + height, mark));
                    y += height;
                }
            }
            Content::Blocks => {
                // (the place of a child is from the border box of its parent: the padding and the
                // border are in it)
                let origin = top + shift;
                for child in self.node(id).children.clone() {
                    let child_top = origin + self.node(child).layout.location.y;
                    self.collect_units(child, child_top, units);
                }
            }
        }
    }
}

/// The fragments that a row can have when it starts at `from` and `available` is what is left
/// of the page, in the order of growing content: the row cut at every place that fits, or else
/// at the first one, and the rest of it if that fits. A row that is not taller than a page is
/// not cut: it is the one fragment that is the row (and, where it does not fit, it is a page
/// break in front of the row that the page takes instead).
pub(super) fn row_fragments(
    cuts: &RowCuts,
    from: Option<ContainerCut>,
    available: f64,
    page: (f32, f32),
) -> Vec<Fragment> {
    let start = from.map_or(0, |cut| cut.item);
    let fragment = |end: Option<u32>| {
        let (orphan_violation, widow_violation) = end.map_or((false, false), |offset| cuts.violations(offset));
        Fragment {
            end: end.map(|offset| ContainerCut::at_item(offset as usize)),
            used: f64::from(dequantize(cuts.used(start, end.unwrap_or(cuts.height)))),
            end_kind: BreakKind::Allowed,
            orphan_violation,
            widow_violation,
            avoid_violation: false,
        }
    };
    let whole = fragment(None);
    // A row that is at most half as high as the page, and as wide, is kept whole; the others can
    // be split where the page ends (CSS Tables 3, 6.1: they are "freely fragmentable").
    let (width, height) = page;
    // (a grid is cut between its rows whatever its height is: a grid that has no place to be cut
    // in has no candidates)
    let kept_whole = !cuts.stacked && dequantize(cuts.height) * 2.0 <= height.min(width);
    if from.is_none() && kept_whole {
        return vec![whole];
    }
    let windowed: Vec<u32>;
    let mut candidates: &[u32] = &cuts.candidates;
    if cuts.stacked {
        // A fragment is at least as high as what it ends at is below where it starts, so the
        // ends that fit are the ones up to there; of these the last are the ones that fill the
        // page best, and the first one is the least that can be placed.
        const WINDOW: usize = 32;
        let from = cuts.start(start);
        let limit = from.saturating_add((available.max(0.0) * f64::from(QUANT)).ceil() as u32).saturating_add(1);
        let low = candidates.partition_point(|&offset| offset <= start);
        let high = candidates.partition_point(|&offset| offset <= limit).max(low);
        let mut chosen: Vec<u32> = candidates.get(low).copied().into_iter().collect();
        chosen.extend_from_slice(&candidates[high.saturating_sub(WINDOW).max(low)..high]);
        chosen.dedup();
        windowed = chosen;
        candidates = &windowed;
    }
    let mut found: Vec<Fragment> = candidates
        .iter()
        .copied()
        .filter(|&offset| offset > start)
        .map(|offset| fragment(Some(offset)))
        .collect();
    found.push(whole);
    // the content grows with the end of the fragment (the fragments that end where a piece does,
    // and what a cell goes on with from the cut, can be a little less than the one before)
    found.sort_by(|left, right| left.used.total_cmp(&right.used));
    let fitting = found.iter().take_while(|fragment| fragment.used <= available + 1.0e-3).count();
    if fitting == 0 {
        found.truncate(1);
        return found;
    }
    // (a few of the largest are enough: the pages choose among the ends that fill the page best)
    const MOST: usize = 12;
    let first = fitting.saturating_sub(MOST);
    found[first..fitting].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(index: usize, units: &[(f32, f32)]) -> CellUnits {
        CellUnits {
            node: NodeId::from(index),
            units: units.iter().map(|&(top, bottom)| (quantize(top), quantize(bottom))).collect(),
            marks: vec![Mark::default(); units.len()],
            atomic: false,
        }
    }

    /// Two cells that have a line of 10 and of 17 pixels.
    fn two_cells() -> RowCuts {
        let tens: Vec<(f32, f32)> = (0..30).map(|line| (line as f32 * 10.0, (line + 1) as f32 * 10.0)).collect();
        let seventeens: Vec<(f32, f32)> = (0..17).map(|line| (line as f32 * 17.0, (line + 1) as f32 * 17.0)).collect();
        let cells = vec![cell(1, &tens), cell(2, &seventeens)];
        let mut candidates: Vec<u32> = cells
            .iter()
            .flat_map(|cell| cell.units.iter().map(|&(_, bottom)| bottom))
            .filter(|&bottom| bottom < quantize(300.0))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        RowCuts { height: quantize(300.0), cells, candidates, stacked: false }
    }

    #[test]
    fn a_row_of_up_to_half_a_page_is_one_fragment() {
        // 300 pixels on a page of 600: kept whole, even where it does not fit what is left
        let fragments = row_fragments(&two_cells(), None, 30.0, (700.0, 600.0));
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].end, None);
        assert_eq!(fragments[0].used, 300.0);
    }

    #[test]
    fn a_row_of_more_than_half_a_page_can_be_split_where_the_page_ends() {
        // 300 pixels on a page of 400 (which it fits): freely fragmentable (CSS Tables 3, 6.1)
        let fragments = row_fragments(&two_cells(), None, 150.0, (700.0, 400.0));
        assert!(fragments.len() >= 2);
        assert!(fragments.iter().all(|fragment| fragment.used <= 150.0 && fragment.end.is_some()));
    }

    #[test]
    fn a_cut_inside_a_paragraph_that_leaves_one_line_is_a_violation() {
        let mark = |index| Mark { paragraph: 7, index, count: 5, orphans: 2, widows: 2 };
        let cell = CellUnits {
            node: NodeId::from(1usize),
            units: (0..5).map(|line| (quantize(line as f32 * 10.0), quantize((line + 1) as f32 * 10.0))).collect(),
            marks: (0..5).map(mark).collect(),
            atomic: false,
        };
        let row = RowCuts { height: quantize(50.0), cells: vec![cell], candidates: vec![], stacked: false };
        // after the first line: an orphan; after the fourth: a widow; after the second: neither
        assert_eq!(row.violations(quantize(10.0)), (true, false));
        assert_eq!(row.violations(quantize(20.0)), (false, false));
        assert_eq!(row.violations(quantize(40.0)), (false, true));
    }

    #[test]
    fn a_line_across_a_cut_goes_on_the_next_fragment_at_its_top() {
        let row = two_cells();
        // cut at 100: the lines of the first cell end there; the second cell has a line from 85
        // to 102 across it, which is on the next fragment
        let cut = quantize(100.0);
        let second = &row.cells[1];
        assert_eq!(row.cell_origin(second, cut), Some(quantize(85.0)));
        assert_eq!(row.cell_part(NodeId::from(2usize), 0, cut), Some((0, Some(cut))));
        // the second cell is moved down by 15 to start at the cut, and it has 100 - 0 + 15 of room
        let (shift, until) = row.cell_part(NodeId::from(2usize), cut, quantize(200.0)).unwrap();
        assert_eq!(dequantize(shift), 15.0);
        assert_eq!(until.map(dequantize), Some(115.0));
        // the fragment is as high as the cell that needs most: the lines of the second cell that
        // end in (100, 200] are the ones that end at 119, 136, 153, 170 and 187, from 85
        assert_eq!(dequantize(row.used(cut, quantize(200.0))), 187.0 - 85.0);
        // the first fragment has what ends before the cut, from the top
        assert_eq!(dequantize(row.used(0, cut)), 100.0);
    }

    #[test]
    fn every_piece_is_on_one_fragment() {
        let row = two_cells();
        let cuts = [0, quantize(100.0), quantize(200.0), row.height];
        for cell in &row.cells {
            for &(_, bottom) in &cell.units {
                let on: Vec<_> = cuts
                    .windows(2)
                    .filter(|pair| bottom > pair[0] && bottom <= pair[1])
                    .collect();
                assert_eq!(on.len(), 1, "piece ending at {bottom}");
            }
        }
    }

    #[test]
    fn a_tall_row_is_cut_where_it_fits() {
        let row = two_cells();
        let fragments = row_fragments(&row, None, 120.0, (400.0, 140.0));
        assert!(fragments.len() >= 2);
        assert!(fragments.iter().all(|fragment| fragment.used <= 120.0));
        assert!(fragments.iter().all(|fragment| fragment.end.is_some()));
    }

    #[test]
    fn the_last_fragment_ends_the_row() {
        let row = two_cells();
        let last = row_fragments(&row, Some(ContainerCut::at_item(quantize(290.0) as usize)), 140.0, (400.0, 140.0));
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].end, None);
        // (the second cell has a line from 272 to 289 across the cut at 290... no: it ends at 289)
        assert!(last[0].used > 0.0 && last[0].used <= 20.0, "{}", last[0].used);
    }

    #[test]
    fn nothing_that_fits_is_the_least_that_can_be_placed() {
        let cells = vec![cell(1, &[(0.0, 200.0)])];
        let row = RowCuts { height: quantize(300.0), cells, candidates: vec![quantize(200.0)], stacked: false };
        let fragments = row_fragments(&row, None, 100.0, (400.0, 140.0));
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].used, 200.0);
    }
}
