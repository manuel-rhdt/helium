use crate::graph::{shortest_path, PageEdge, PageGraph};
use crate::kernel::{Kernel, PageFill, Solution};
use std::fmt::Debug;

/// The cost of a page, and how the costs of the pages make the cost of a pagination, which a
/// solver minimises.
///
/// A cost is a type of its own, and costs are compared as a whole: a struct of fields in the order
/// of their priority, compared field by field (`#[derive(PartialOrd)]`), says that a violation of
/// a rule is worse than any amount of the cost below it without a number that stands for "very
/// large".
pub trait Objective {
    type Cost: Clone + Debug + PartialOrd;

    /// The cost of a pagination without pages.
    fn zero(&self) -> Self::Cost;

    /// The cost of a page, or `None` if it cannot be used. `is_last` is whether it is the last
    /// page of the document.
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> Option<Self::Cost>;

    /// The cost of a pagination that has the pages so far, with the cost `so_far`, and one page
    /// more, whose cost is `page` and whose number is `index` (the first page has the number 0).
    /// It must not get better when `so_far` or `page` get worse: the sum of costs that are not
    /// negative and the maximum do. The maximum minimises the worst page, as balancing columns
    /// does; then a pagination is not found by its prefix alone, so an objective that takes it
    /// is followed by one that breaks the ties (see [`balance`]).
    fn combine(&self, so_far: &Self::Cost, page: &Self::Cost, index: usize) -> Self::Cost;
}

/// A policy that chooses the pages. It must only return solutions that the kernel accepts:
/// it can only choose among [`Kernel::candidates`].
pub trait Solver {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution;
}

/// The cost of a pagination under CSS: what does not fit, a break that is avoided, orphans and
/// widows, and how empty the pages are, in this order of priority. Where one of them differs, the
/// ones below it do not matter.
#[derive(Debug, Copy, Clone, Default, PartialEq, PartialOrd)]
pub struct CssCost {
    /// Pages that an unbreakable item that is taller than the page has to itself.
    pub overflows: u32,
    /// Pages that end at a place where a break should be avoided (`break-inside: avoid`, or
    /// `break-before` or `break-after: avoid`). A break is only avoided if the content fits
    /// otherwise, but content that does not fit on a page is broken anyway.
    pub avoided_breaks: u32,
    /// Pages that have fewer lines of a paragraph at their end than `orphans` or at their start
    /// than `widows`. Violating these is preferred to violating an `avoid` value.
    pub orphans_and_widows: u32,
    /// The sum over the pages of the cube of the room that is left on a page that does not end
    /// the document or at a forced break, in units of 1/[`SLACK_UNIT`](Self::SLACK_UNIT) of a
    /// pixel (see [`CssCost::slack_of`]).
    pub slack: u128,
}

impl CssCost {
    /// The room that is left on a page is counted in whole units of 1/64 px. Positions are
    /// single precision numbers, and the room on a page is a difference of two of them: two ways
    /// to paginate that have the same pages, except that the pages are in another order, would
    /// otherwise differ in their cost by the rounding of those numbers, and a rounding error would
    /// decide between them, not the rule of the solver for paginations of the same cost. In whole
    /// units they cost the same.
    pub const SLACK_UNIT: f64 = 64.0;

    /// The cost of a page that has `room` left: the cube of the room.
    pub fn slack_of(room: f64) -> u128 {
        let units = (room.max(0.0) * Self::SLACK_UNIT).round() as u128;
        units * units * units
    }

    pub fn plus(&self, other: &CssCost) -> CssCost {
        CssCost {
            overflows: self.overflows + other.overflows,
            avoided_breaks: self.avoided_breaks + other.avoided_breaks,
            orphans_and_widows: self.orphans_and_widows + other.orphans_and_widows,
            slack: self.slack + other.slack,
        }
    }
}

/// The objective of CSS: pages are filled, and `avoid`, orphans and widows are respected as
/// far as possible.
#[derive(Debug, Copy, Clone, Default)]
pub struct CssObjective;

impl Objective for CssObjective {
    type Cost = CssCost;

    fn zero(&self) -> CssCost {
        CssCost::default()
    }

    fn page_cost(&self, fill: &PageFill, is_last: bool) -> Option<CssCost> {
        if fill.overflow {
            return Some(CssCost {
                overflows: 1,
                ..CssCost::default()
            });
        }
        // A forced break is always taken, but how the page in front of it is filled still
        // matters for the widows of its last paragraph, and it does not matter how empty it is.
        // The last page is allowed to be partially filled.
        let slack = if fill.end_kind.is_forced() || is_last {
            0
        } else {
            CssCost::slack_of(fill.spec.block_size as f64 - fill.used as f64)
        };
        Some(CssCost {
            overflows: 0,
            avoided_breaks: u32::from(fill.end_kind == crate::BreakKind::Avoid),
            orphans_and_widows: u32::from(fill.orphan_violation || fill.widow_violation),
            slack,
        })
    }

    fn combine(&self, so_far: &CssCost, page: &CssCost, _index: usize) -> CssCost {
        so_far.plus(page)
    }
}

/// The best pagination for an objective: the shortest path through the pages that can start at
/// each place, which is found once for each of them ([`shortest_path`] in the graph of the
/// pages, [`PageGraph`]).
///
/// Where the cost of two paths is the same, the one with the earlier start of the last page
/// wins, like in the least-weight subsequence of the line breaker
/// ([`crate::paragraph::compute_breakpoints`]).
#[derive(Debug, Copy, Clone, Default)]
pub struct ExactSolver<O> {
    pub objective: O,
}

impl<O: Objective> ExactSolver<O> {
    pub fn new(objective: O) -> Self {
        ExactSolver { objective }
    }
}

impl<O: Objective> Solver for ExactSolver<O> {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution {
        let first = kernel.first_page();
        let fragment = kernel.is_fragment();
        let objective = &self.objective;
        let found = {
            let mut graph = PageGraph::new(kernel);
            shortest_path(
                &mut graph,
                objective.zero(),
                &mut |edge: &PageEdge| objective.page_cost(&edge.fill, edge.last),
                &mut |so_far, page, edge| objective.combine(so_far, page, edge.index),
            )
        };
        let mut pages = found.as_ref().map_or_else(Vec::new, |path| path.nodes.clone());
        if pages.is_empty() {
            pages.push(first);
        }
        // the solution of a fragment ends with the place where its last page ends
        if fragment {
            if let Some(path) = &found {
                pages.push(path.end);
            }
        }
        Solution { pages }
    }
}

/// The cost of a column of balanced columns: what it breaks and how tall it is, in the order of
/// their priority. A column is only as tall as it takes to respect `avoid` and orphans and
/// widows if it is possible, and a column that cannot is as tall as the content needs.
///
/// The fields are numbers, not counts, because the second step of balancing weighs the columns.
#[derive(Debug, Copy, Clone, Default, PartialEq, PartialOrd)]
pub struct ColumnCost {
    /// 1 for a column that is taller than what it can hold.
    pub overflow: f64,
    /// 1 for a column that ends where it should not (`avoid`).
    pub avoided_break: f64,
    /// 1 for a column with fewer lines of a paragraph at the end than `orphans` or at the start
    /// than `widows`.
    pub orphans_and_widows: f64,
    pub height: f64,
}

impl ColumnCost {
    pub fn of(fill: &PageFill) -> ColumnCost {
        ColumnCost {
            overflow: f64::from(u8::from(fill.overflow)),
            avoided_break: f64::from(u8::from(fill.end_kind == crate::BreakKind::Avoid)),
            orphans_and_widows: f64::from(u8::from(fill.orphan_violation || fill.widow_violation)),
            height: fill.used as f64,
        }
    }

    /// The worse of two costs.
    pub fn max(self, other: ColumnCost) -> ColumnCost {
        if other > self {
            other
        } else {
            self
        }
    }

    fn plus_weighted(&self, other: &ColumnCost, weight: f64) -> ColumnCost {
        ColumnCost {
            overflow: self.overflow + other.overflow * weight,
            avoided_break: self.avoided_break + other.avoided_break * weight,
            orphans_and_widows: self.orphans_and_widows + other.orphans_and_widows * weight,
            height: self.height + other.height * weight,
        }
    }
}

/// The first step of balancing columns: the least cost of the worst column. A pagination is not
/// found by its prefix alone, so the result is one of the paginations with the least worst
/// column, and [`FullestFirst`] chooses among them.
#[derive(Debug, Copy, Clone, Default)]
pub struct TallestColumn;

impl Objective for TallestColumn {
    type Cost = ColumnCost;

    fn zero(&self) -> ColumnCost {
        ColumnCost::default()
    }

    fn page_cost(&self, fill: &PageFill, _is_last: bool) -> Option<ColumnCost> {
        Some(ColumnCost::of(fill))
    }

    fn combine(&self, so_far: &ColumnCost, page: &ColumnCost, _index: usize) -> ColumnCost {
        so_far.max(*page)
    }
}

/// The second step of balancing columns: among the paginations in which no column costs more
/// than `bound` (the least worst column of the first step), the one with the least sum of the
/// costs of the columns, each weighted by its number plus one: with the same tallest column, the
/// content is in the first columns. It is a sum, so a prefix of a pagination decides.
#[derive(Debug, Copy, Clone)]
pub struct FullestFirst {
    pub bound: ColumnCost,
}

impl Objective for FullestFirst {
    type Cost = ColumnCost;

    fn zero(&self) -> ColumnCost {
        ColumnCost::default()
    }

    fn page_cost(&self, fill: &PageFill, _is_last: bool) -> Option<ColumnCost> {
        let cost = ColumnCost::of(fill);
        (cost <= self.bound).then_some(cost)
    }

    fn combine(&self, so_far: &ColumnCost, page: &ColumnCost, index: usize) -> ColumnCost {
        so_far.plus_weighted(page, (index + 1) as f64)
    }
}

/// The columns of a container with the least height of the tallest one, and among those the
/// fullest first columns ([`TallestColumn`], [`FullestFirst`]). The problem should have a limit
/// on its number of pages: the number of columns. The choice among equal ones is the one of the
/// exact solver, so the result is the same every time.
pub fn balance(kernel: &mut Kernel<'_>) -> Solution {
    let first = ExactSolver::new(TallestColumn).solve(kernel);
    let Ok(pages) = kernel.realize(&first) else {
        return first;
    };
    let bound = pages
        .iter()
        .map(ColumnCost::of)
        .fold(ColumnCost::default(), ColumnCost::max);
    ExactSolver::new(FullestFirst { bound }).solve(kernel)
}
