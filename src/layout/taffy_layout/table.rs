//! Tables, laid out as CSS grids.
//!
//! Taffy has no table layout. A table is turned into a grid instead:
//!
//! * the cells become the grid items, each placed at the row and column it has in the table
//!   (with its `colspan` and `rowspan`);
//! * there is one track per row and column; the `width` of the cells becomes the size of their
//!   column and the `height` of the rows and cells the size of their row;
//! * `border-spacing` becomes the gap between the tracks, and an empty track at each edge puts
//!   the spacing between the outermost cells and the border of the table as well;
//! * captions are grid items that span all columns in rows of their own above the cells.
//!
//! Rows and row groups are no Taffy nodes. After the grid is laid out, they get the boxes of the
//! tracks they span (see [`HeliumTree::place_table_parts`]), and the cells are moved to be
//! relative to the row they are in, so the rest of the layout sees an ordinary box tree.
//!
//! Not supported yet: `vertical-align` in cells (their contents are at the top), columns
//! (`<col>`), `table-layout: fixed`, `border-collapse` and `caption-side`. Grid tracks share
//! extra width equally instead of in proportion to their contents.
//!
//! https://www.w3.org/TR/CSS22/tables.html

use taffy::compute_grid_layout;
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
        for &(cell, row, column, rows, columns) in &placements {
            let extras = self.inline_extras(cell);
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
        if inputs.known_dimensions.width.is_none() && self.node(id).style.size.width.is_auto() {
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
                inputs.known_dimensions.width = Some(shrink_to_fit(min, max, available));
            }
        }
        let output = compute_grid_layout(self, id, inputs);
        if inputs.run_mode == RunMode::PerformLayout {
            self.place_table_parts(id);
        }
        output
    }

    /// Gives the rows and row groups the boxes of their grid tracks and moves the cells into
    /// them.
    fn place_table_parts(&mut self, table: NodeId) {
        let Some(structure) = self.node_mut(table).table.take() else {
            return;
        };
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
