//! The collapsing border model of tables (`border-collapse: collapse`).
//!
//! The borders of adjacent cells (and of the rows, row groups and the table next to them) are
//! resolved into one border per edge of the table grid: the widest border wins; between borders
//! of the same width, the one of a cell wins over the one of a row, which wins over the one of a
//! row group and then of the table; between borders of the same kind, the one further to the
//! top and to the left (to the right in right-to-left tables) wins. Only the styles `none` and
//! `solid` are supported, so the rules about styles reduce to these.
//!
//! For the layout, each cell has half of the width of the borders on its edges as its border
//! (the widest one if an edge of a spanning cell borders several others) and the table half of
//! its outer borders; there is no `border-spacing` and the table has no padding.
//!
//! Each border is painted once, with its full width centered on the grid line, by one of the
//! cells next to it: a cell paints its top and left (start) borders, and its bottom and right
//! borders where there is no cell next to them. The table, its rows and groups paint none.
//! Painting the halves of a border by both cells would leave a hairline seam between them in
//! many PDF viewers.
//!
//! https://www.w3.org/TR/CSS22/tables.html#collapsing-borders

use taffy::style::LengthPercentage as TLengthPercentage;
use taffy::tree::NodeId;

use crate::layout::components::{CollapsedEdge, EdgeValues};
use crate::layout::inline_size::decoration_colors;
use crate::style::properties::computed::{PixelLength as Px, RGBA};
use crate::style::properties::specified::{BorderCollapse, Direction};

use super::tree::HeliumTree;

/// A cell placed in the table grid: `(cell, row, column, rows, columns)`, with the column in
/// logical order (the first column is at the right in a right-to-left table).
pub(super) type Placement = (NodeId, usize, usize, usize, usize);

/// A row or row group with the rows it spans (`start..end`).
pub(super) struct Part {
    pub node: NodeId,
    pub is_row: bool,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Origin {
    Table,
    RowGroup,
    Row,
    Cell,
}

#[derive(Clone, Copy, Default)]
struct Border {
    width: f32,
    color: Option<RGBA>,
}

/// Collects the borders that meet on an edge and picks the winner. The candidates are added in
/// the order of preference among borders of the same width and origin.
#[derive(Default)]
struct Candidates {
    best: Option<(f32, Origin, RGBA)>,
}

impl Candidates {
    fn add(&mut self, (width, color): (f32, RGBA), origin: Origin) {
        let better = match self.best {
            None => true,
            Some((best_width, best_origin, _)) => {
                width > best_width || (width == best_width && origin > best_origin)
            }
        };
        if better {
            self.best = Some((width, origin, color));
        }
    }

    fn border(&self) -> Border {
        match self.best {
            Some((width, _, color)) if width > 0.0 => Border {
                width,
                color: Some(color),
            },
            _ => Border::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl HeliumTree<'_, '_> {
    /// Whether the table has `border-collapse: collapse`.
    pub(super) fn has_collapsed_borders(&self, table: NodeId) -> bool {
        self.computed_values
            .get(self.node(table).entity)
            .is_some_and(|style| style.text.border_collapse == BorderCollapse::Collapse)
    }

    /// The width and color of the border of a box on one side.
    fn border_of(&self, id: NodeId, side: Side) -> (f32, RGBA) {
        let Some(style) = self.computed_values.get(self.node(id).entity) else {
            return (0.0, RGBA::transparent());
        };
        let colors = decoration_colors(style).border_color;
        let position = &style.position;
        match side {
            Side::Top => (position.border_top_width.0, colors.top),
            Side::Bottom => (position.border_bottom_width.0, colors.bottom),
            Side::Left => (position.border_left_width.0, colors.left),
            Side::Right => (position.border_right_width.0, colors.right),
        }
    }

    /// Resolves the borders of a table with collapsing borders, gives its cells and the table
    /// their (half) border widths for the layout and stores the borders to paint in the nodes.
    pub(super) fn collapse_borders(
        &mut self,
        table: NodeId,
        placements: &[Placement],
        parts: &[Part],
        rows: usize,
        columns: usize,
    ) {
        if rows == 0 || columns == 0 {
            return;
        }
        let rtl = self
            .computed_values
            .get(self.node(table).entity)
            .is_some_and(|style| style.text.direction == Direction::Rtl);
        // the cells with physical columns (from the left)
        let cells: Vec<Placement> = placements
            .iter()
            .map(|&(cell, row, column, row_span, column_span)| {
                let column = if rtl {
                    columns - column - column_span
                } else {
                    column
                };
                (cell, row, column, row_span, column_span)
            })
            .collect();
        let mut owner = vec![vec![None; columns]; rows];
        for (index, &(_, row, column, row_span, column_span)) in cells.iter().enumerate() {
            for slot_row in &mut owner[row..row + row_span] {
                for slot in &mut slot_row[column..column + column_span] {
                    *slot = Some(index);
                }
            }
        }
        let owner_at = |row: usize, column: usize| owner.get(row).and_then(|r| r[column]);
        let row_of = |index: usize| {
            parts
                .iter()
                .find(|part| part.is_row && part.start == index)
                .map(|part| part.node)
        };
        let group_of = |index: usize| {
            parts
                .iter()
                .find(|part| !part.is_row && part.start <= index && index < part.end)
        };

        // the horizontal borders above row `r` (`horizontal[r][c]`, `r` up to `rows`) and the
        // vertical borders left of column `c` (`vertical[r][c]`, `c` up to `columns`)
        let mut horizontal = vec![vec![Border::default(); columns]; rows + 1];
        for (r, borders) in horizontal.iter_mut().enumerate() {
            for (c, border) in borders.iter_mut().enumerate() {
                let above = r.checked_sub(1).and_then(|r| owner_at(r, c));
                let below = (r < rows).then(|| owner_at(r, c)).flatten();
                if above.is_some() && above == below {
                    // inside a cell that spans rows
                    continue;
                }
                let mut candidates = Candidates::default();
                if let Some(index) = above {
                    candidates.add(self.border_of(cells[index].0, Side::Bottom), Origin::Cell);
                }
                if let Some(index) = below {
                    candidates.add(self.border_of(cells[index].0, Side::Top), Origin::Cell);
                }
                if let Some(row) = r.checked_sub(1).and_then(row_of) {
                    candidates.add(self.border_of(row, Side::Bottom), Origin::Row);
                }
                if let Some(row) = (r < rows).then(|| row_of(r)).flatten() {
                    candidates.add(self.border_of(row, Side::Top), Origin::Row);
                }
                if let Some(group) = r.checked_sub(1).and_then(group_of) {
                    if group.end == r {
                        candidates.add(self.border_of(group.node, Side::Bottom), Origin::RowGroup);
                    }
                }
                if let Some(group) = (r < rows).then(|| group_of(r)).flatten() {
                    if group.start == r {
                        candidates.add(self.border_of(group.node, Side::Top), Origin::RowGroup);
                    }
                }
                if r == 0 {
                    candidates.add(self.border_of(table, Side::Top), Origin::Table);
                }
                if r == rows {
                    candidates.add(self.border_of(table, Side::Bottom), Origin::Table);
                }
                *border = candidates.border();
            }
        }
        let mut vertical = vec![vec![Border::default(); columns + 1]; rows];
        for (r, borders) in vertical.iter_mut().enumerate() {
            for (c, border) in borders.iter_mut().enumerate() {
                let left = c.checked_sub(1).and_then(|c| owner_at(r, c));
                let right = (c < columns).then(|| owner_at(r, c)).flatten();
                if left.is_some() && left == right {
                    continue;
                }
                // the border further to the start wins between borders of the same kind
                let mut sides = [
                    left.map(|index| (index, Side::Right)),
                    right.map(|index| (index, Side::Left)),
                ];
                if rtl {
                    sides.reverse();
                }
                let mut candidates = Candidates::default();
                for (index, side) in sides.into_iter().flatten() {
                    candidates.add(self.border_of(cells[index].0, side), Origin::Cell);
                }
                let outer = if c == 0 {
                    Some(Side::Left)
                } else if c == columns {
                    Some(Side::Right)
                } else {
                    None
                };
                if let Some(side) = outer {
                    if let Some(row) = row_of(r) {
                        candidates.add(self.border_of(row, side), Origin::Row);
                    }
                    if let Some(group) = group_of(r) {
                        candidates.add(self.border_of(group.node, side), Origin::RowGroup);
                    }
                    candidates.add(self.border_of(table, side), Origin::Table);
                }
                *border = candidates.border();
            }
        }

        let widest = |borders: &mut dyn Iterator<Item = Border>| {
            borders.fold(Border::default(), |widest, border| {
                if border.width > widest.width {
                    border
                } else {
                    widest
                }
            })
        };
        // half of the widest of the borders meeting at a corner of the grid, by which the
        // borders across them are extended
        let horizontal_at = |r: usize, c: usize| {
            let left = c.checked_sub(1).map_or(0.0, |c| horizontal[r][c].width);
            let right = horizontal[r].get(c).map_or(0.0, |border| border.width);
            left.max(right) / 2.0
        };
        let vertical_at = |r: usize, c: usize| {
            let above = r.checked_sub(1).map_or(0.0, |r| vertical[r][c].width);
            let below = vertical.get(r).map_or(0.0, |borders| borders[c].width);
            above.max(below) / 2.0
        };
        let edge = |border: Border, paint: bool, extend_start: f32, extend_end: f32| CollapsedEdge {
            width: Px(border.width),
            color: border.color.unwrap_or(RGBA::transparent()),
            paint: paint && border.width > 0.0,
            extend_start: Px(extend_start),
            extend_end: Px(extend_end),
        };

        for &(cell, row, column, row_span, column_span) in &cells {
            let (bottom_row, right_column) = (row + row_span, column + column_span);
            let top = widest(&mut horizontal[row][column..right_column].iter().copied());
            let bottom = widest(&mut horizontal[bottom_row][column..right_column].iter().copied());
            let left = widest(&mut vertical[row..bottom_row].iter().map(|r| r[column]));
            let right = widest(&mut vertical[row..bottom_row].iter().map(|r| r[right_column]));
            let paint_bottom = bottom_row == rows || owner_at(bottom_row, column).is_none();
            let paint_right = right_column == columns || owner_at(row, right_column).is_none();
            let edges = EdgeValues {
                top: edge(
                    top,
                    true,
                    vertical_at(row, column),
                    vertical_at(row, right_column),
                ),
                bottom: edge(
                    bottom,
                    paint_bottom,
                    vertical_at(bottom_row, column),
                    vertical_at(bottom_row, right_column),
                ),
                left: edge(
                    left,
                    true,
                    horizontal_at(row, column),
                    horizontal_at(bottom_row, column),
                ),
                right: edge(
                    right,
                    paint_right,
                    horizontal_at(row, right_column),
                    horizontal_at(bottom_row, right_column),
                ),
            };
            let node = self.node_mut(cell);
            node.collapsed = Some(edges);
            node.style.border = taffy::geometry::Rect {
                top: TLengthPercentage::length(top.width / 2.0),
                bottom: TLengthPercentage::length(bottom.width / 2.0),
                left: TLengthPercentage::length(left.width / 2.0),
                right: TLengthPercentage::length(right.width / 2.0),
            };
        }

        let outer_width = |borders: &mut dyn Iterator<Item = Border>| widest(borders).width / 2.0;
        let table_border = taffy::geometry::Rect {
            top: TLengthPercentage::length(outer_width(&mut horizontal[0].iter().copied())),
            bottom: TLengthPercentage::length(outer_width(&mut horizontal[rows].iter().copied())),
            left: TLengthPercentage::length(outer_width(&mut vertical.iter().map(|r| r[0]))),
            right: TLengthPercentage::length(outer_width(
                &mut vertical.iter().map(|r| r[columns]),
            )),
        };
        let nothing = Some(EdgeValues::all(CollapsedEdge::default()));
        let node = self.node_mut(table);
        node.style.border = table_border;
        node.style.padding = taffy::geometry::Rect {
            top: TLengthPercentage::length(0.0),
            bottom: TLengthPercentage::length(0.0),
            left: TLengthPercentage::length(0.0),
            right: TLengthPercentage::length(0.0),
        };
        node.style.gap = taffy::geometry::Size {
            width: TLengthPercentage::length(0.0),
            height: TLengthPercentage::length(0.0),
        };
        node.collapsed = nothing;
        for part in parts {
            self.node_mut(part.node).collapsed = nothing;
        }
    }
}
