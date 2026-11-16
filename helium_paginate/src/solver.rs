use crate::flow::PageKind;
use crate::kernel::{Kernel, PageFill, PageStart, Solution};
use crate::token::BreakToken;
use std::collections::{BTreeMap, HashMap};

/// The cost of a page. A solver minimises the sum of the costs of the pages.
pub trait Objective {
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> f64;
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

type State = (BreakToken, PageKind);

struct Node {
    cost: f64,
    previous: Option<State>,
    start: PageStart,
}

impl<O: Objective> Solver for ExactSolver<O> {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution {
        let first = kernel.first_page();
        let state_of = |kernel: &Kernel<'_>, start: &PageStart| kernel.state(start);
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
        let order = |kernel: &Kernel<'_>, state: State| (kernel.position(state.0).order(), state.1 as u8);
        let mut queue = BTreeMap::from([(order(kernel, first_state), first_state)]);
        // the ways to the end of the document
        let mut ends: Vec<State> = vec![];
        while let Some((_, state)) = queue.pop_first() {
            if kernel.is_end(state.0) {
                ends.push(state);
                continue;
            }
            let start = nodes[&state].start;
            let cost_so_far = nodes[&state].cost;
            for fill in kernel.candidates(&start) {
                let next = kernel.next_page(&start, &fill);
                let cost = self.objective.page_cost(&fill, next.is_none()) + cost_so_far;
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
        let key = |kernel: &Kernel<'_>, state: State| -> Vec<(usize, u32)> {
            path(state)
                .iter()
                .map(|start| kernel.position(start.at).order())
                .collect()
        };
        let best = ends
            .iter()
            .copied()
            .filter(|state| nodes[state].cost == cheapest)
            .min_by_key(|&state| (key(kernel, state), state.1 as u8));
        let mut pages = best.map_or_else(Vec::new, path);
        pages.reverse();
        if pages.is_empty() {
            pages.push(first);
        }
        Solution { pages }
    }
}
