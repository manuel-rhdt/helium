//! Tables, laid out as CSS grids.
//!
//! Taffy has no table layout. A table is turned into a grid instead:
//!
//! * the cells become the grid items, each placed at the row and column it has in the table
//!   (with its `colspan` and `rowspan`);
//! * there is one track per row and column; the `width` of the cells and of the `<col>` elements
//!   that cover a column becomes its size, and the `height` of the rows and cells the size of
//!   their row;
//! * `border-spacing` becomes the gap between the tracks, and an empty track at each edge puts
//!   the spacing between the outermost cells and the border of the table as well;
//! * captions are grid items that span all columns in rows of their own above the cells.
//!
//! Rows and row groups are no Taffy nodes. After the grid is laid out, they get the boxes of the
//! tracks they span (see [`HeliumTree::place_table_parts`]), and the cells are moved to be
//! relative to the row they are in, so the rest of the layout sees an ordinary box tree.
//!
//! `vertical-align` in cells puts their contents at the top, in the middle or at the bottom (see
//! [`HeliumTree::align_cell_content`]); `baseline` counts as `top`, the baselines of the cells
//! in a row are not aligned. Not supported yet: `caption-side`. Grid tracks share extra width equally
//! instead of in proportion to their contents. `border-collapse: collapse` is resolved in
//! [`super::collapse`].
//!
//! https://www.w3.org/TR/CSS22/tables.html

use taffy::{LayoutPartialTree, compute_grid_layout};
use taffy::geometry::{Line, Point, Size};
use taffy::style::{
    AvailableSpace, Dimension, GridTemplateComponent, LengthPercentageAuto, MaxTrackSizingFunction,
    MinTrackSizingFunction,
};
use taffy::style_helpers::{auto, length, line, minmax, span};
use taffy::tree::Layout;
use taffy::tree::{LayoutInput, LayoutOutput, NodeId, RequestedAxis, RunMode, SizingMode};

use crate::layout::absolute::shrink_to_fit;
use crate::style::properties::computed::{GenericSize, LengthPercentage};
use crate::style::properties::specified::Display;

use super::tree::{Content, HeliumTree};

/// How the boxes of a table are arranged in its grid.
#[derive(Debug, Default)]
pub(super) struct TableStructure {
    /// The in-flow children of the table in document order: captions, row groups, rows and
    /// cells that are not in a row. The out-of-flow children are among the children of the node.
    pub boxes: Vec<NodeId>,
    /// The number of captions, which are in the first grid rows.
    captions: usize,
    /// Whether there is an empty track in front of the first row.
    leading_row_track: bool,
    /// The number of table rows and columns (without the empty tracks at the edges).
    rows: usize,
    columns: usize,
    /// Row groups and rows with the table rows they span (`start..end`), in the order they
    /// have to be placed: a group before its rows.
    parts: Vec<TablePart>,
    /// The cells with the box they are placed relative to (their row, their row group, or the
    /// table).
    cells: Vec<(NodeId, Option<NodeId>)>,
    /// For every table row, whether a cell spans the row and the one after it: a page break
    /// between the two rows would cut the cell.
    blocked_after: Vec<bool>,
    /// Whether there are cells that are in no row box, so that the table cannot be split between
    /// the boxes of its rows.
    loose_cells: bool,
    /// The percentage (as taffy takes it) that the cells and `<col>` elements give each column.
    column_percents: Vec<Option<f32>>,
    /// The fixed width that the cells and `<col>` elements give each column.
    column_lengths: Vec<Option<f32>>,
    /// The cells that are in one column only, for each column.
    column_cells: Vec<Vec<NodeId>>,
    /// The widths of the columns for `table-layout: fixed`: from the `<col>` element of the
    /// column, or else from the cell of the first row. A length is of the whole column; a cell
    /// that spans columns gives each of them its part.
    fixed_lengths: Vec<Option<f32>>,
    fixed_percents: Vec<Option<f32>>,
}

/// A box of a table that a page break can come in front of or behind: a caption, a row group or
/// a row.
#[derive(Debug, Clone)]
pub(super) struct TableKid {
    pub node: NodeId,
    /// The table rows of the box.
    pub rows: std::ops::Range<usize>,
    /// The rows of a row group.
    pub kids: Vec<TableKid>,
}

#[derive(Debug)]
struct TablePart {
    node: NodeId,
    /// The row group a row is in.
    parent: Option<NodeId>,
    start: usize,
    end: usize,
}

/// A row of the table: a row box or an anonymous row around cells that are not in a row.
#[derive(Debug)]
struct Row {
    node: Option<NodeId>,
    cells: Vec<NodeId>,
}

/// A row group: a row group box or an anonymous group around rows outside of row groups.
#[derive(Debug)]
struct Group {
    node: Option<NodeId>,
    rows: Vec<Row>,
}

/// The grid tracks of a laid out grid container, relative to its border box.
#[derive(Debug, Clone, Default)]
pub(super) struct GridTracks {
    pub rows: Vec<Line<f32>>,
    pub columns: Vec<Line<f32>>,
}

/// The cells of the table that occupy a slot of the grid.
#[derive(Default)]
struct Slots(Vec<Vec<bool>>);

impl Slots {
    fn is_taken(&self, row: usize, column: usize) -> bool {
        self.0
            .get(row)
            .and_then(|r| r.get(column))
            .copied()
            .unwrap_or(false)
    }

    fn take(&mut self, row: usize, column: usize) {
        if self.0.len() <= row {
            self.0.resize_with(row + 1, Vec::new);
        }
        let r = &mut self.0[row];
        if r.len() <= column {
            r.resize(column + 1, false);
        }
        r[column] = true;
    }
}

impl HeliumTree<'_, '_> {
    /// The row group of a table that is repeated at the top of every page that goes on inside of
    /// the table: the first `table-header-group`, if only captions are in front of it. (A header
    /// that comes behind the rows is laid out at the top of the table, but it is not repeated.)
    pub(super) fn repeated_header(&self, table: NodeId) -> Option<NodeId> {
        let span = self.node(table).span.as_ref()?;
        for &kid in &span.kids {
            match self.display(kid) {
                Display::TableCaption => continue,
                Display::TableHeaderGroup => {
                    let has_rows = self.node(kid).span.as_ref().is_some_and(|span| !span.kids.is_empty());
                    return has_rows.then_some(kid);
                }
                _ => return None,
            }
        }
        None
    }

    /// The row group of a table that is repeated behind the last row of every page that ends
    /// inside of the table: the first `table-footer-group`, which is laid out last in the table.
    pub(super) fn repeated_footer(&self, table: NodeId) -> Option<NodeId> {
        let span = self.node(table).span.as_ref()?;
        let last = *span.kids.last()?;
        let has_rows = self.node(last).span.as_ref().is_some_and(|span| !span.kids.is_empty());
        (self.display(last) == Display::TableFooterGroup && has_rows).then_some(last)
    }

    fn display(&self, id: NodeId) -> Display {
        self.computed_values
            .get(self.node(id).entity)
            .map_or(Display::Block, |style| style.box_.display)
    }

    /// The in-flow children of a node. The children of a box with inline content are all out of
    /// flow.
    fn in_flow_children(&self, id: NodeId) -> Vec<NodeId> {
        let node = self.node(id);
        if node.content != Content::Blocks {
            return vec![];
        }
        node.children
            .iter()
            .copied()
            .filter(|&child| !super::tree::node_is_out_of_flow(self, child))
            .collect()
    }

    /// The rows of a row group or of the table. Cells (and other boxes) that are not in a row
    /// are put in anonymous rows.
    /// https://www.w3.org/TR/CSS22/tables.html#anonymous-boxes
    fn collect_rows(&self, children: &[NodeId], rows: &mut Vec<Row>) {
        let mut in_anonymous_row = false;
        for &child in children {
            if self.display(child) == Display::TableRow {
                rows.push(Row {
                    node: Some(child),
                    cells: self.in_flow_children(child),
                });
                in_anonymous_row = false;
            } else {
                if !in_anonymous_row {
                    rows.push(Row {
                        node: None,
                        cells: vec![],
                    });
                    in_anonymous_row = true;
                }
                rows.last_mut().unwrap().cells.push(child);
            }
        }
    }

    /// Turns a table node into a grid container whose items are the cells and captions of the
    /// table.
    pub(super) fn build_table(&mut self, table: NodeId) {
        if self.node(table).content != Content::Blocks {
            // a table without rows or cells
            return;
        }
        let children = std::mem::take(&mut self.node_mut(table).children);
        let mut structure = TableStructure::default();
        let mut out_of_flow = vec![];
        let mut captions = vec![];
        let mut header = None;
        let mut footer = None;
        let mut bodies: Vec<Group> = vec![];
        // the children that are neither captions nor row groups, which are in anonymous groups
        let mut loose: Vec<NodeId> = vec![];

        let flush = |tree: &Self, loose: &mut Vec<NodeId>, bodies: &mut Vec<Group>| {
            if !loose.is_empty() {
                let mut rows = vec![];
                tree.collect_rows(loose, &mut rows);
                bodies.push(Group { node: None, rows });
                loose.clear();
            }
        };
        for &child in &children {
            if super::tree::node_is_out_of_flow(self, child) {
                out_of_flow.push(child);
                continue;
            }
            structure.boxes.push(child);
            let display = self.display(child);
            if display == Display::TableCaption {
                captions.push(child);
            } else if display.is_table_row_group() {
                flush(self, &mut loose, &mut bodies);
                let mut rows = vec![];
                self.collect_rows(&self.in_flow_children(child), &mut rows);
                let group = Group {
                    node: Some(child),
                    rows,
                };
                // Only the first header and footer group are moved to the top and bottom.
                match display {
                    Display::TableHeaderGroup if header.is_none() => header = Some(group),
                    Display::TableFooterGroup if footer.is_none() => footer = Some(group),
                    _ => bodies.push(group),
                }
            } else {
                loose.push(child);
            }
        }
        flush(self, &mut loose, &mut bodies);
        let groups: Vec<Group> = header.into_iter().chain(bodies).chain(footer).collect();

        // the out-of-flow children of row groups, rows and cells that are not in rows stay in
        // their boxes, but they are laid out by the table
        for group in &groups {
            let mut boxes: Vec<NodeId> = group.node.into_iter().collect();
            boxes.extend(group.rows.iter().filter_map(|row| row.node));
            for id in boxes {
                out_of_flow.extend(
                    self.node(id)
                        .children
                        .iter()
                        .copied()
                        .filter(|&child| super::tree::node_is_out_of_flow(self, child)),
                );
            }
        }

        // place the cells
        // https://html.spec.whatwg.org/multipage/tables.html#forming-a-table
        let mut slots = Slots::default();
        // (cell, row, column, rows, columns)
        let mut placements = vec![];
        let mut row_index = 0;
        for group in &groups {
            let group_start = row_index;
            let group_end = group_start + group.rows.len();
            for row in &group.rows {
                let mut column = 0;
                for &cell in &row.cells {
                    while slots.is_taken(row_index, column) {
                        column += 1;
                    }
                    let cell_span = self
                        .table_spans
                        .get(self.node(cell).entity)
                        .copied()
                        .unwrap_or_default();
                    let columns = usize::from(cell_span.columns.max(1));
                    // `rowspan` does not reach beyond the row group, and 0 spans its remaining
                    // rows
                    let rows = match cell_span.rows {
                        0 => group_end - row_index,
                        rows => usize::from(rows).min(group_end - row_index),
                    };
                    for r in row_index..row_index + rows {
                        for c in column..column + columns {
                            slots.take(r, c);
                        }
                    }
                    placements.push((cell, row_index, column, rows, columns));
                    let parent = row.node.or(group.node);
                    structure.loose_cells |= row.node.is_none();
                    structure.cells.push((cell, parent));
                    column += columns;
                }
                if let Some(node) = row.node {
                    structure.parts.push(TablePart {
                        node,
                        parent: group.node,
                        start: row_index,
                        end: row_index + 1,
                    });
                }
                row_index += 1;
            }
            if let Some(node) = group.node {
                // the group goes in front of its rows
                let first_row =
                    structure.parts.len() - group.rows.iter().filter(|r| r.node.is_some()).count();
                structure.parts.insert(
                    first_row,
                    TablePart {
                        node,
                        parent: None,
                        start: group_start,
                        end: group_end,
                    },
                );
            }
        }
        structure.rows = row_index;
        structure.columns = slots.0.iter().map(Vec::len).max().unwrap_or(0);
        structure.blocked_after = vec![false; structure.rows];
        for &(_, row, _, rows, _) in &placements {
            for blocked in structure.blocked_after.iter_mut().skip(row).take(rows.saturating_sub(1)) {
                *blocked = true;
            }
        }
        if self.has_collapsed_borders(table) {
            let parts: Vec<super::collapse::Part> = structure
                .parts
                .iter()
                .map(|part| super::collapse::Part {
                    node: part.node,
                    is_row: self.display(part.node) == Display::TableRow,
                    start: part.start,
                    end: part.end,
                })
                .collect();
            self.collapse_borders(
                table,
                &placements,
                &parts,
                structure.rows,
                structure.columns,
            );
        }
        structure.captions = captions.len();
        let has_cells = structure.rows > 0 && structure.columns > 0;
        structure.leading_row_track = has_cells && captions.is_empty();

        // the grid tracks
        let mut column_tracks = vec![MinMaxTrack::default(); structure.columns];
        let mut row_tracks = vec![0.0f32; structure.rows];
        for part in &structure.parts {
            if part.end == part.start + 1 && self.display(part.node) == Display::TableRow {
                if let Some(height) = self.fixed_length(part.node, |s| s.size.height) {
                    row_tracks[part.start] = row_tracks[part.start].max(height);
                }
            }
        }
        let mut fixed_lengths: Vec<Option<f32>> = vec![None; structure.columns];
        let mut fixed_percents: Vec<Option<f32>> = vec![None; structure.columns];
        for &(cell, row, column, rows, columns) in &placements {
            let mut extras = self.inline_extras(cell);
            // a `width` or `height` of a `border-box` cell is the one of the box with the padding
            // and the border
            if self.node(cell).style.box_sizing == taffy::BoxSizing::BorderBox {
                extras = Size { width: 0.0, height: 0.0 };
            }
            if row == 0 {
                let share = columns.max(1) as f32;
                let width = self.computed_values.get(self.node(cell).entity).map(|s| s.position.width);
                for index in column..(column + columns).min(fixed_lengths.len()) {
                    match width {
                        Some(GenericSize::LengthPercentage(LengthPercentage::PixelLength(px))) => {
                            fixed_lengths[index].get_or_insert((px.0 + extras.width) / share);
                        }
                        Some(GenericSize::LengthPercentage(LengthPercentage::Percentage(percent))) => {
                            fixed_percents[index].get_or_insert(percent.0 / share);
                        }
                        _ => {}
                    }
                }
            }
            if columns == 1 {
                let track = &mut column_tracks[column];
                match self
                    .computed_values
                    .get(self.node(cell).entity)
                    .map(|s| s.position.width)
                {
                    Some(GenericSize::LengthPercentage(LengthPercentage::PixelLength(px))) => {
                        track.length = Some(track.length.unwrap_or(0.0).max(px.0 + extras.width));
                    }
                    Some(GenericSize::LengthPercentage(LengthPercentage::Percentage(p))) => {
                        track.percent = Some(track.percent.unwrap_or(0.0).max(p.0));
                    }
                    _ => {}
                }
            }
            if rows == 1 {
                if let Some(height) = self.fixed_length(cell, |s| s.size.height) {
                    row_tracks[row] = row_tracks[row].max(height + extras.height);
                }
            }
        }

        // The widths of `<col>` and `<colgroup>` (CSS 2.1, 17.5.2.2): a column is at least as
        // wide as the column element that covers it; its cells can still make it wider.
        let table_columns = self
            .table_columns
            .get(self.node(table).entity)
            .cloned()
            .unwrap_or_default();
        let mut index = 0;
        for column in table_columns.columns {
            let span = usize::from(column.span.max(1));
            for position in index..(index + span).min(fixed_lengths.len()) {
                match column.width {
                    Some(GenericSize::LengthPercentage(LengthPercentage::PixelLength(px))) => {
                        fixed_lengths[position] = Some(px.0);
                        fixed_percents[position] = None;
                    }
                    Some(GenericSize::LengthPercentage(LengthPercentage::Percentage(percent))) => {
                        fixed_percents[position] = Some(percent.0);
                        fixed_lengths[position] = None;
                    }
                    _ => {}
                }
            }
            for track in column_tracks.iter_mut().skip(index).take(span) {
                match column.width {
                    Some(GenericSize::LengthPercentage(LengthPercentage::PixelLength(px))) => {
                        track.length = Some(track.length.unwrap_or(0.0).max(px.0));
                    }
                    Some(GenericSize::LengthPercentage(LengthPercentage::Percentage(percent))) => {
                        track.percent = Some(track.percent.unwrap_or(0.0).max(percent.0));
                    }
                    _ => {}
                }
            }
            index += span;
        }

        // the grid lines of the first row and column (1-based, after the empty tracks)
        let first_row_line = 1 + captions.len() + usize::from(structure.leading_row_track);
        let first_column_line = 1 + usize::from(has_cells);
        let grid_line = |index: usize| line(i16::try_from(index).unwrap_or(i16::MAX));
        let grid_span = |count: usize| span(u16::try_from(count).unwrap_or(u16::MAX));

        let mut items = vec![];
        for (index, &caption) in captions.iter().enumerate() {
            let style = &mut self.node_mut(caption).style;
            style.grid_row = Line {
                start: grid_line(index + 1),
                end: grid_span(1),
            };
            style.grid_column = Line {
                start: line(1),
                end: line(-1),
            };
            items.push(caption);
        }
        for &(cell, row, column, rows, columns) in &placements {
            let style = &mut self.node_mut(cell).style;
            style.grid_row = Line {
                start: grid_line(first_row_line + row),
                end: grid_span(rows),
            };
            style.grid_column = Line {
                start: grid_line(first_column_line + column),
                end: grid_span(columns),
            };
            // The sizes of the cells are the sizes of their tracks, and cells have no margins.
            style.size = Size::auto();
            style.margin = taffy::geometry::Rect {
                left: LengthPercentageAuto::length(0.0),
                right: LengthPercentageAuto::length(0.0),
                top: LengthPercentageAuto::length(0.0),
                bottom: LengthPercentageAuto::length(0.0),
            };
            items.push(cell);
        }
        items.extend(out_of_flow);

        structure.column_percents = column_tracks.iter().map(|track| track.percent).collect();
        structure.column_lengths = column_tracks.iter().map(|track| track.length).collect();
        structure.fixed_lengths = fixed_lengths;
        structure.fixed_percents = fixed_percents;
        structure.column_cells = vec![vec![]; structure.columns];
        for &(cell, _, column, _, columns) in &placements {
            if columns == 1 {
                structure.column_cells[column].push(cell);
            }
        }
        let mut columns: Vec<GridTemplateComponent<String>> = vec![];
        let mut rows: Vec<GridTemplateComponent<String>> = vec![auto(); captions.len()];
        if has_cells {
            columns.push(length(0.0));
            columns.extend(column_tracks.iter().map(MinMaxTrack::track));
            columns.push(length(0.0));
            if structure.leading_row_track {
                rows.push(length(0.0));
            }
            rows.extend(row_tracks.iter().map(|&height| {
                if height > 0.0 {
                    minmax(
                        MinTrackSizingFunction::length(height),
                        MaxTrackSizingFunction::auto(),
                    )
                } else {
                    auto()
                }
            }));
            rows.push(length(0.0));
        }

        let node = self.node_mut(table);
        node.content = Content::Table;
        node.children = items;
        node.style.grid_template_columns = columns;
        node.style.grid_template_rows = rows;
        node.style.grid_auto_rows = vec![];
        node.style.grid_auto_columns = vec![];
        if !has_cells {
            // only captions, no spacing between them
            node.style.gap.width = taffy::style::LengthPercentage::length(0.0);
        }
        node.table = Some(Box::new(structure));
    }

    /// The boxes of a table that a page may break between, in the order of the grid, or `None`
    /// if the table cannot be split between them.
    pub(super) fn table_kids(&self, table: NodeId) -> Option<Vec<TableKid>> {
        let structure = self.node(table).table.as_deref()?;
        if structure.loose_cells || structure.rows == 0 {
            return None;
        }
        let mut kids: Vec<TableKid> = structure
            .boxes
            .iter()
            .filter(|&&node| self.display(node) == Display::TableCaption)
            .map(|&node| TableKid { node, rows: 0..0, kids: vec![] })
            .collect();
        for part in &structure.parts {
            if part.parent.is_some() {
                continue;
            }
            let rows = part.start..part.end;
            let is_group = self.display(part.node).is_table_row_group();
            let inner = if is_group {
                structure
                    .parts
                    .iter()
                    .filter(|row| row.parent == Some(part.node))
                    .map(|row| TableKid { node: row.node, rows: row.start..row.end, kids: vec![] })
                    .collect()
            } else {
                vec![]
            };
            kids.push(TableKid { node: part.node, rows, kids: inner });
        }
        Some(kids)
    }

    /// Whether a page break behind the table row `row` would cut a cell that spans it and the
    /// next one.
    pub(super) fn table_row_blocked(&self, table: NodeId, row: usize) -> bool {
        self.node(table)
            .table
            .as_ref()
            .and_then(|structure| structure.blocked_after.get(row).copied())
            .unwrap_or(false)
    }

    /// A definite length of a node from its Taffy style.
    fn fixed_length(&self, id: NodeId, get: impl Fn(&taffy::Style) -> Dimension) -> Option<f32> {
        let dimension = get(&self.node(id).style);
        (dimension.tag() == taffy::CompactLength::LENGTH_TAG).then(|| dimension.value())
    }

    /// The fixed horizontal and vertical padding and border of a box.
    fn inline_extras(&self, id: NodeId) -> Size<f32> {
        let style = &self.node(id).style;
        let fixed = |lp: taffy::style::LengthPercentage| {
            let raw = lp.into_raw();
            if raw.tag() == taffy::CompactLength::LENGTH_TAG {
                raw.value()
            } else {
                0.0
            }
        };
        Size {
            width: fixed(style.padding.left)
                + fixed(style.padding.right)
                + fixed(style.border.left)
                + fixed(style.border.right),
            height: fixed(style.padding.top)
                + fixed(style.padding.bottom)
                + fixed(style.border.top)
                + fixed(style.border.bottom),
        }
    }

    /// Lays out a table. A table whose width is `auto` is as wide as its contents, but no wider
    /// than the available space (and no narrower than its narrowest layout).
    /// https://www.w3.org/TR/CSS22/tables.html#auto-table-layout
    pub(super) fn compute_table_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let mut inputs = inputs;
        if self.is_fixed_layout(id) {
            let output = self.compute_fixed_table_layout(id, inputs);
            if inputs.run_mode == RunMode::PerformLayout {
                self.mirror_children(id, output.size.width, inputs.parent_size.width);
                self.place_table_parts(id);
            }
            return output;
        }
        // The columns with a percentage: they make the table as wide as they need to get their
        // share (CSS Tables 3, 3.3, "percentage columns").
        let percent = self.percent_columns(id, inputs);
        let auto_width = self.node(id).style.size.width.is_auto();
        if inputs.known_dimensions.width.is_none() && auto_width {
            if let AvailableSpace::Definite(available) = inputs.available_space.width {
                let measure = |tree: &mut Self, space| {
                    compute_grid_layout(
                        tree,
                        id,
                        LayoutInput {
                            run_mode: RunMode::ComputeSize,
                            sizing_mode: SizingMode::InherentSize,
                            axis: RequestedAxis::Horizontal,
                            known_dimensions: Size::NONE,
                            available_space: Size {
                                width: space,
                                height: AvailableSpace::MaxContent,
                            },
                            ..inputs
                        },
                    )
                    .size
                    .width
                };
                let min = measure(self, AvailableSpace::MinContent);
                let max = measure(self, AvailableSpace::MaxContent);
                let max = percent.as_ref().map_or(max, |percent| max.max(percent.max_width));
                inputs.known_dimensions.width = Some(shrink_to_fit(min, max, available));
            } else if let (Some(percent), AvailableSpace::MaxContent) =
                (&percent, inputs.available_space.width)
            {
                // (the width that the table asks for when its container is sized by it)
                inputs.known_dimensions.width = Some(percent.max_width);
            }
        }
        // A table is never narrower than its columns need (CSS 2.1 17.5.2.2: the used width is
        // the larger of the `width` and the minimum width of the table): a `width` that is too
        // small leaves the columns at their minimum.
        if inputs.known_dimensions.width.is_none() && !auto_width {
            let measure = |tree: &mut Self, inputs: LayoutInput| {
                compute_grid_layout(
                    tree,
                    id,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        axis: RequestedAxis::Horizontal,
                        ..inputs
                    },
                )
                .size
                .width
            };
            let specified = measure(self, inputs);
            let min = measure(
                self,
                LayoutInput {
                    // (the sizes of the style are the ones that are being limited)
                    sizing_mode: SizingMode::ContentSize,
                    known_dimensions: Size::NONE,
                    available_space: Size {
                        width: AvailableSpace::MinContent,
                        height: AvailableSpace::MaxContent,
                    },
                    ..inputs
                },
            );
            // (a minimum that is wider than the container is not trusted: the percentages in the
            // table may have been taken of the container)
            let fits = inputs.parent_size.width.is_none_or(|container| min <= container);
            if min > specified && fits {
                inputs.known_dimensions.width = Some(min);
            } else if percent.is_some() {
                inputs.known_dimensions.width = Some(specified);
            }
        }
        // the percentages are of the width without the spacing and the borders
        if let Some(percent) = &percent {
            let width = inputs.known_dimensions.width;
            self.set_percent_tracks(id, width.map(|width| (width - percent.extras).max(0.0)));
        }
        let output = compute_grid_layout(self, id, inputs);
        if inputs.run_mode == RunMode::PerformLayout {
            self.mirror_children(id, output.size.width, inputs.parent_size.width);
            self.place_table_parts(id);
        }
        output
    }

    /// Whether the table has `table-layout: fixed` and a width: without a width the table is
    /// laid out by its contents (CSS 2.1, 17.5.2.1).
    fn is_fixed_layout(&self, id: NodeId) -> bool {
        use crate::style::properties::specified::TableLayout;
        let node = self.node(id);
        node.table.as_ref().is_some_and(|structure| structure.rows > 0 && structure.columns > 0)
            && !node.style.size.width.is_auto()
            && self
                .computed_values
                .get(node.entity)
                .is_some_and(|style| style.box_.table_layout == TableLayout::Fixed)
    }

    /// The layout of a table with `table-layout: fixed`: the widths of the columns are those of
    /// the `<col>` elements and of the cells of the first row, and the columns that have none
    /// share what is left of the width of the table. The contents of the cells do not matter.
    /// https://www.w3.org/TR/CSS22/tables.html#fixed-table-layout
    fn compute_fixed_table_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let mut inputs = inputs;
        let (lengths, percents) = match self.node(id).table.as_ref() {
            Some(structure) => (structure.fixed_lengths.clone(), structure.fixed_percents.clone()),
            None => return compute_grid_layout(self, id, inputs),
        };
        let count = lengths.len();
        // the spacing between the columns and at the edges, and the borders and padding
        let spacing = self.node(id).style.gap.width.into_raw().value().max(0.0);
        let extras = spacing * (count + 1) as f32 + self.inline_extras(id).width;
        // the width of the table that the style gives
        let specified = match inputs.known_dimensions.width {
            Some(width) => width,
            None => {
                compute_grid_layout(
                    self,
                    id,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        axis: RequestedAxis::Horizontal,
                        ..inputs
                    },
                )
                .size
                .width
            }
        };
        // the table is as wide as its columns with a width (and the spacing) need at least
        let given: f32 = lengths.iter().flatten().sum();
        let width = if inputs.known_dimensions.width.is_some() {
            specified
        } else {
            specified.max(given + extras)
        };
        let space = (width - extras).max(0.0);
        // the widths of the columns
        let mut widths: Vec<Option<f32>> = (0..count)
            .map(|index| match (lengths[index], percents[index]) {
                (Some(length), _) => Some(length),
                (None, Some(percent)) => Some(percent * space),
                _ => None,
            })
            .collect();
        let used: f32 = widths.iter().flatten().sum();
        let free = space - used;
        let auto = widths.iter().filter(|width| width.is_none()).count();
        if auto > 0 {
            // the columns without a width share what is left equally
            let each = free.max(0.0) / auto as f32;
            for width in widths.iter_mut().filter(|width| width.is_none()) {
                *width = Some(each);
            }
        } else if free > 0.0 {
            // the extra width is distributed over the columns, in proportion to their widths
            if used > 0.0 {
                for width in widths.iter_mut().flatten() {
                    *width += *width / used * free;
                }
            } else {
                for width in widths.iter_mut().flatten() {
                    *width += free / count as f32;
                }
            }
        }
        let node = self.node_mut(id);
        for (index, width) in widths.into_iter().enumerate() {
            // (the first track is the empty one at the edge)
            if let Some(track) = node.style.grid_template_columns.get_mut(index + 1) {
                *track = length(width.unwrap_or(0.0));
            }
        }
        inputs.known_dimensions.width = Some(width);
        compute_grid_layout(self, id, inputs)
    }

    /// The columns of a table that have a percentage: with the width that the table needs for
    /// them to get their share, and the width of the table that is not the width of its columns
    /// (the spacing, borders and padding). `None` for a table without percentage columns.
    fn percent_columns(&mut self, id: NodeId, inputs: LayoutInput) -> Option<PercentColumns> {
        let (percents, lengths, cells) = match self.node(id).table.as_ref() {
            Some(structure) if structure.rows > 0 && structure.columns > 0 => (
                structure.column_percents.clone(),
                structure.column_lengths.clone(),
                structure.column_cells.clone(),
            ),
            _ => return None,
        };
        if !percents.iter().any(|percent| percent.is_some_and(|percent| percent > 0.0)) {
            return None;
        }
        // the natural widths of the columns: as wide as the widest cell of the column wants
        // (what the grid takes as the contribution of the cells that are in one column)
        let mut widths = vec![];
        for (index, column) in cells.iter().enumerate() {
            let mut width = lengths.get(index).copied().flatten().unwrap_or(0.0);
            for &cell in column {
                let size = self.compute_child_layout(
                    cell,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        sizing_mode: SizingMode::InherentSize,
                        axis: RequestedAxis::Horizontal,
                        known_dimensions: Size::NONE,
                        known_dimensions_are_definite: Size { width: false, height: false },
                        parent_size: Size { width: None, height: None },
                        available_space: Size {
                            width: AvailableSpace::MaxContent,
                            height: AvailableSpace::MaxContent,
                        },
                        vertical_margins_are_collapsible: inputs.vertical_margins_are_collapsible,
                    },
                );
                width = width.max(size.size.width);
            }
            widths.push(width);
        }
        // the width of the table that is not the width of its columns: the spacing, which is
        // between the columns and at the edges, and the borders and padding of the table
        let node_style = &self.node(id).style;
        let spacing = node_style
            .gap
            .width
            .into_raw()
            .value()
            .max(0.0);
        let extras = spacing * (percents.len() + 1) as f32 + self.inline_extras(id).width;
        let total: f32 = percents.iter().flatten().sum();
        let others: f32 = widths.iter().zip(&percents).filter(|(_, percent)| percent.is_none()).map(|(width, _)| width).sum();
        let mut need = widths.iter().sum::<f32>();
        for (width, percent) in widths.iter().zip(&percents) {
            if let Some(percent) = percent.filter(|&percent| percent > 0.0) {
                need = need.max(width / percent);
            }
        }
        if total < 1.0 {
            need = need.max(others / (1.0 - total));
        }
        Some(PercentColumns { extras, max_width: need + extras })
    }

    /// Sets the widths of the columns with a percentage: as much as the percentage of `width`
    /// (the width of the columns together), or, without a width, the percentage that the grid
    /// resolves itself.
    fn set_percent_tracks(&mut self, id: NodeId, width: Option<f32>) {
        let Some(percents) = self.node(id).table.as_ref().map(|structure| structure.column_percents.clone()) else {
            return;
        };
        let node = self.node_mut(id);
        for (index, percent) in percents.iter().enumerate() {
            let Some(percent) = *percent else { continue };
            let max = match width {
                Some(width) => MaxTrackSizingFunction::length(percent * width),
                None => MaxTrackSizingFunction::percent(percent),
            };
            // (the first track is the empty one at the edge)
            if let Some(track) = node.style.grid_template_columns.get_mut(index + 1) {
                *track = minmax(MinTrackSizingFunction::min_content(), max);
            }
        }
    }

    /// Finds how far the content of a cell is moved down by `vertical-align`: the cell is as high
    /// as its row, and its content is at the top, in the middle or at the bottom of the cell.
    /// (`baseline` and the other values of the inline boxes count as `top`; the baselines of the
    /// cells in a row are not aligned.) Not in vertical text, where the block axis is horizontal.
    fn align_cell_content(&mut self, cell: NodeId) {
        use crate::style::properties::computed::VerticalAlign;
        self.node_mut(cell).content_shift = 0.0;
        let entity = self.node(cell).entity;
        let align = self
            .computed_values
            .get(entity)
            .map_or(VerticalAlign::Baseline, |style| style.box_.vertical_align);
        if !matches!(align, VerticalAlign::Middle | VerticalAlign::Bottom)
            || self.node(cell).flow.is_vertical()
        {
            return;
        }
        let layout = self.node(cell).layout;
        // The height of the cell for its content alone: the one that its own `height` and its
        // row would give is not what the content needs.
        let saved = self.node(cell).style.clone();
        {
            let node = self.node_mut(cell);
            node.style.size.height = auto();
            node.style.min_size.height = auto();
            node.style.max_size.height = auto();
            node.cache = taffy::tree::Cache::new();
        }
        let natural = self
            .compute_child_layout(
                cell,
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Vertical,
                    known_dimensions: Size { width: Some(layout.size.width), height: None },
                    known_dimensions_are_definite: Size { width: true, height: false },
                    parent_size: Size { width: Some(layout.size.width), height: None },
                    available_space: Size {
                        width: AvailableSpace::Definite(layout.size.width),
                        height: AvailableSpace::MaxContent,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            )
            .size
            .height;
        {
            let node = self.node_mut(cell);
            node.style = saved;
            node.cache = taffy::tree::Cache::new();
        }
        let extra = (layout.size.height - natural).max(0.0);
        self.node_mut(cell).content_shift = match align {
            VerticalAlign::Middle => extra / 2.0,
            _ => extra,
        };
    }

    /// Gives the rows and row groups the boxes of their grid tracks and moves the cells into
    /// them.
    fn place_table_parts(&mut self, table: NodeId) {
        let Some(structure) = self.node_mut(table).table.take() else {
            return;
        };
        for &(cell, _) in &structure.cells {
            self.align_cell_content(cell);
        }
        let tracks = self.node(table).grid_tracks.clone().unwrap_or_default();
        let first_row_track = structure.captions + usize::from(structure.leading_row_track);
        // the rows and columns of the table are between the empty tracks at the edges
        let row_track = |row: usize| tracks.rows.get(first_row_track + row).copied();
        let left = tracks.columns.get(1).map_or(0.0, |track| track.start);
        let right = tracks
            .columns
            .get(structure.columns)
            .map_or(left, |track| track.end);
        let row_top = |row: usize| match row_track(row) {
            Some(track) => track.start,
            // the end of the last row
            None => row
                .checked_sub(1)
                .and_then(row_track)
                .map_or(0.0, |track| track.end),
        };
        let row_bottom = |end: usize| {
            end.checked_sub(1)
                .and_then(row_track)
                .map_or(row_top(end), |track| track.end)
        };

        // the location of the rows and groups relative to the table
        let mut locations: Vec<(NodeId, Point<f32>)> = vec![];
        for part in &structure.parts {
            let top = row_top(part.start);
            let bottom = if part.end > part.start {
                row_bottom(part.end)
            } else {
                top
            };
            let location = Point { x: left, y: top };
            let parent_location = part
                .parent
                .and_then(|parent| locations.iter().find(|(id, _)| *id == parent))
                .map_or(Point { x: 0.0, y: 0.0 }, |&(_, location)| location);
            locations.push((part.node, location));
            self.node_mut(part.node).layout = Layout {
                location: Point {
                    x: location.x - parent_location.x,
                    y: location.y - parent_location.y,
                },
                size: Size {
                    width: right - left,
                    height: bottom - top,
                },
                ..Layout::new()
            };
        }
        for &(cell, parent) in &structure.cells {
            let Some(parent_location) = parent
                .and_then(|parent| locations.iter().find(|(id, _)| *id == parent))
                .map(|&(_, location)| location)
            else {
                continue;
            };
            let layout = &mut self.node_mut(cell).layout;
            layout.location.x -= parent_location.x;
            layout.location.y -= parent_location.y;
        }
        self.node_mut(table).table = Some(structure);
    }
}

/// What the percentage columns of a table ask of its width.
struct PercentColumns {
    /// The width of the table that is not the width of its columns.
    extras: f32,
    /// The width of the table when its columns get their share and the others the rest.
    max_width: f32,
}

/// The size of a column from the widths of its cells.
#[derive(Debug, Clone, Copy, Default)]
struct MinMaxTrack {
    length: Option<f32>,
    percent: Option<f32>,
}

impl MinMaxTrack {
    /// A column with a width is as wide as its widest cell (with that width), but at least as
    /// wide as its contents.
    fn track(&self) -> GridTemplateComponent<String> {
        let min = MinTrackSizingFunction::min_content();
        match (self.percent, self.length) {
            (Some(percent), _) => minmax(min, MaxTrackSizingFunction::percent(percent)),
            (None, Some(length)) => minmax(min, MaxTrackSizingFunction::length(length)),
            (None, None) => auto(),
        }
    }
}
