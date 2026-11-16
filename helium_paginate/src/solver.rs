use crate::flow::PageKind;
use crate::kernel::{Kernel, PageFill, PageStart, Solution};
use crate::token::BreakToken;
use std::collections::{BTreeMap, HashMap};

/// The cost of a page, and how the costs of the pages make the cost of a pagination, which a
/// solver minimises.
pub trait Objective {
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> f64;

    /// The cost of a pagination that has the pages so far, with the cost `so_far`, and one page
    /// more, whose cost is `page` and whose number is `index` (the first page has the number 0).
    /// It must not decrease when `so_far` or `page` do not. The sum, which is the default, adds
    /// up the costs of the pages. The maximum minimises the worst page, as balancing columns does;
    /// then a pagination is not found by its prefix alone, so an objective that takes it
    /// is followed by one that breaks the ties (see [`balance`]).
    fn combine(&self, so_far: f64, page: f64, index: usize) -> f64 {
        let _ = index;
        so_far + page
    }
}

/// A policy that chooses the pages. It must only return solutions that the kernel accepts:
/// it can only choose among [`Kernel::candidates`].
pub trait Solver {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution;
}

/// The objective of CSS: pages are filled, and `avoid`, orphans and widows are respected as
/// far as possible.
#[derive(Debug, Copy, Clone, Default)]
pub struct CssObjective;

impl CssObjective {
    /// The cost of a page break at a place where a break should be avoided (`break-inside:
    /// avoid` or `break-before`/`break-after: avoid`). It is larger than the cost of any page
    /// that is merely not full, so a break is only avoided if the content fits otherwise, but
    /// finite so that content that does not fit on a page is broken anyway.
    pub const AVOID_BREAK_COST: f64 = 1e12;
    /// The cost of a page that ends at a forced break; it outweighs all other costs.
    pub const FORCED_BREAK_COST: f64 = -1e15;
    /// The cost of a page that has fewer lines of a paragraph at its end than `orphans` or at its
    /// start than `widows`. Violating these is preferred to violating an `avoid` value.
    pub const ORPHANS_WIDOWS_COST: f64 = 1e11;
    /// The cost of a page that an unbreakable item that is taller than the page has to itself.
    pub const OVERFLOW_COST: f64 = 1e30;
}

impl Objective for CssObjective {
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> f64 {
        if fill.overflow {
            return Self::OVERFLOW_COST;
        }
        // A forced break is always taken, but how the page in front of it is filled still
        // matters for the widows of its last paragraph.
        let mut cost = if fill.end_kind.is_forced() {
            Self::FORCED_BREAK_COST
        } else if is_last {
            // the last page is allowed to be partially filled
            0.0
        } else {
            (fill.spec.block_size as f64 - fill.used as f64).powi(3)
        };
        if fill.end_kind == crate::BreakKind::Avoid {
            cost += Self::AVOID_BREAK_COST;
        }
        if fill.orphan_violation || fill.widow_violation {
            cost += Self::ORPHANS_WIDOWS_COST;
        }
        cost
    }
}

/// The best pagination for an objective: the shortest path through the pages that can start at
/// each place, which is found once for each of them.
///
/// Where the cost of two paths is the same, the one with the earlier start of the last page
/// wins, like in the least-weight subsequence of the line breaker
/// ([`crate::paragraph::compute_breakpoints`]).
///
/// The state of the search is the place where a page starts and the kind of the page: the
/// geometry of a page, and with it everything that follows, depends on nothing else about the
/// path that led to it.
#[derive(Debug, Copy, Clone, Default)]
pub struct ExactSolver<O> {
    pub objective: O,
}

impl<O: Objective> ExactSolver<O> {
    pub fn new(objective: O) -> Self {
        ExactSolver { objective }
    }
}

/// The place where a page starts, its kind, and its number if the number of pages is limited.
type State = (BreakToken, PageKind, usize);

struct Node {
    cost: f64,
    previous: Option<State>,
    start: PageStart,
}

impl<O: Objective> Solver for ExactSolver<O> {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution {
        let first = kernel.first_page();
        let state_of = |kernel: &Kernel<'_>, start: &PageStart| {
            let (token, kind) = kernel.state(start);
            (token, kind, kernel.slot(start))
        };
        let mut nodes: HashMap<State, Node> = HashMap::new();
        let first_state = state_of(kernel, &first);
        nodes.insert(
            first_state,
            Node {
                cost: 0.0,
                previous: None,
                start: first,
            },
        );
        // The states are taken in the order of the flow: all pages that end at a place start at
        // an earlier one, so by the time a state is taken from the queue its cost is final.
        let order = |kernel: &Kernel<'_>, state: State| {
            (kernel.position(state.0).order(), state.1 as u8, state.2)
        };
        let mut queue = BTreeMap::from([(order(kernel, first_state), first_state)]);
        // the ways to the end of the document
        let mut ends: Vec<State> = vec![];
        while let Some((_, state)) = queue.pop_first() {
            if kernel.is_final(&nodes[&state].start) {
                ends.push(state);
                continue;
            }
            let start = nodes[&state].start;
            let cost_so_far = nodes[&state].cost;
            for fill in kernel.candidates(&start) {
                let next = kernel.next_page(&start, &fill);
                let page = self.objective.page_cost(&fill, next.is_none());
                let cost = self.objective.combine(cost_so_far, page, start.index);
                if !cost.is_finite() {
                    continue;
                }
                let next = next.unwrap_or(PageStart {
                    at: fill.end,
                    index: start.index + 1,
                });
                let next_state = state_of(kernel, &next);
                let better = nodes.get(&next_state).is_none_or(|node| cost < node.cost);
                if better {
                    nodes.insert(
                        next_state,
                        Node {
                            cost,
                            previous: Some(state),
                            start: next,
                        },
                    );
                    queue.insert(order(kernel, next_state), next_state);
                }
            }
        }
        // The pages of a way to the end, from the last to the first.
        let path = |end: State| {
            let mut pages = vec![];
            let mut at = nodes[&end].previous;
            while let Some(state) = at {
                pages.push(nodes[&state].start);
                at = nodes[&state].previous;
            }
            pages
        };
        // Among the cheapest ways, the one with the earlier start of the last page wins, and
        // where those are the same, the one with the earlier start of the page before it, and
        // so on, as in the least-weight subsequence. The kind of the page is a state of its own,
        // so equal ways with other kinds are found, too, and one of them has to be chosen.
        let cheapest = ends
            .iter()
            .map(|state| nodes[state].cost)
            .fold(f64::INFINITY, f64::min);
        let key = |kernel: &Kernel<'_>, state: State| -> Vec<(usize, u64)> {
            path(state)
                .iter()
                .map(|start| kernel.position(start.at).order())
                .collect()
        };
        // (the place where the last page ends is the same for all of them unless the problem is
        // the fragment of a container, where it is one more thing to decide by)
        let best = ends
            .iter()
            .copied()
            .filter(|state| nodes[state].cost == cheapest)
            .min_by_key(|&state| (key(kernel, state), kernel.position(state.0).order(), state.1 as u8, state.2));
        let mut pages = best.map_or_else(Vec::new, path);
        pages.reverse();
        if pages.is_empty() {
            pages.push(first);
        }
        // the solution of a fragment ends with the place where its last page ends
        if let Some(state) = best.filter(|_| kernel.is_fragment()) {
            pages.push(nodes[&state].start);
        }
        Solution { pages }
    }
}

/// The objective of balanced columns: the least height of the tallest column, and then the first
/// columns as full as they can be.
///
/// It is used in two steps ([`balance`]). The first minimises the maximum of the heights of the
/// columns (`bound` is `None`). The second finds, among the paginations in which no column is
/// taller than the least maximum, the one with the least sum of the heights of the columns,
/// each weighted by its number plus one: with the same tallest column, the content is in the
/// first columns. Both are exact (the second is a sum, so a prefix of a pagination decides),
/// and the choice among equal ones is the one of the exact solver, so the result is the same
/// every time.
///
/// The height of a column is its content plus a cost for every rule that it breaks, which is
/// larger than any content: a column is only as tall as it takes to respect `avoid` and orphans
/// and widows if it is possible, and a column that cannot is as tall as the content needs.
#[derive(Debug, Copy, Clone, Default)]
pub struct BalanceObjective {
    /// The height of the tallest column of the first step, in the second.
    pub bound: Option<f64>,
}

impl BalanceObjective {
    /// The cost of a column that ends where it should not (`avoid`), as a height.
    pub const AVOID_BREAK_COST: f64 = 1e9;
    /// The cost of a column with fewer lines of a paragraph at the end than `orphans` or at the
    /// start than `widows`.
    pub const ORPHANS_WIDOWS_COST: f64 = 1e8;
    /// The cost of a column that is taller than what it can hold.
    pub const OVERFLOW_COST: f64 = 1e12;

    /// The height of a column, with what its breaks cost.
    pub fn height(fill: &PageFill) -> f64 {
        let mut height = fill.used as f64;
        if fill.overflow {
            height += Self::OVERFLOW_COST;
        }
        if fill.end_kind == crate::BreakKind::Avoid {
            height += Self::AVOID_BREAK_COST;
        }
        if fill.orphan_violation || fill.widow_violation {
            height += Self::ORPHANS_WIDOWS_COST;
        }
        height
    }
}

impl Objective for BalanceObjective {
    fn page_cost(&self, fill: &PageFill, _is_last: bool) -> f64 {
        let height = Self::height(fill);
        match self.bound {
            Some(bound) if height > bound => f64::INFINITY,
            _ => height,
        }
    }

    fn combine(&self, so_far: f64, page: f64, index: usize) -> f64 {
        match self.bound {
            None => so_far.max(page),
            Some(_) => so_far + page * (index + 1) as f64,
        }
    }
}

/// The columns of a container with the least height of the tallest one, and among those the
/// fullest first columns ([`BalanceObjective`]). The problem should have a limit on its number
/// of pages: the number of columns.
pub fn balance(kernel: &mut Kernel<'_>) -> Solution {
    let first = ExactSolver::new(BalanceObjective { bound: None }).solve(kernel);
    let Ok(pages) = kernel.realize(&first) else {
        return first;
    };
    let objective = BalanceObjective { bound: None };
    let bound = pages
        .iter()
        .map(|fill| objective.page_cost(fill, false))
        .fold(0.0, f64::max);
    ExactSolver::new(BalanceObjective { bound: Some(bound) }).solve(kernel)
}
