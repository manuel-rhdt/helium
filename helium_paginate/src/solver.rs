use crate::kernel::{Kernel, PageFill, PageStart, Solution};
use crate::token::BreakToken;
use std::collections::{BTreeSet, HashMap};

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
/// The state of the search is the place where a page starts. The geometry of a page cannot
/// depend on its kind yet (see [`PageTable::spec`](crate::PageTable::spec)); once it can, the
/// state is the place and the kind.
#[derive(Debug, Copy, Clone, Default)]
pub struct ExactSolver<O> {
    pub objective: O,
}

impl<O: Objective> ExactSolver<O> {
    pub fn new(objective: O) -> Self {
        ExactSolver { objective }
    }
}

struct Node {
    cost: f64,
    previous: Option<BreakToken>,
    start: PageStart,
}

impl<O: Objective> Solver for ExactSolver<O> {
    fn solve(&self, kernel: &mut Kernel<'_>) -> Solution {
        let first = kernel.first_page();
        let mut nodes: HashMap<BreakToken, Node> = HashMap::new();
        nodes.insert(
            first.at,
            Node {
                cost: 0.0,
                previous: None,
                start: first,
            },
        );
        // The tokens are numbered in the order of the flow: all pages that end at a token start
        // at an earlier token, so by the time a token is taken from the queue its cost is final.
        let mut queue = BTreeSet::from([first.at]);
        let mut end = None;
        while let Some(at) = queue.pop_first() {
            if kernel.is_end(at) {
                end = Some(at);
                continue;
            }
            let start = nodes[&at].start;
            let cost_so_far = nodes[&at].cost;
            for fill in kernel.candidates(&start) {
                let next = kernel.next_page(&start, &fill);
                let cost = self.objective.page_cost(&fill, next.is_none()) + cost_so_far;
                if !cost.is_finite() {
                    continue;
                }
                let better = nodes.get(&fill.end).is_none_or(|node| cost < node.cost);
                if better {
                    let start = next.unwrap_or(PageStart {
                        at: fill.end,
                        index: start.index + 1,
                    });
                    nodes.insert(
                        fill.end,
                        Node {
                            cost,
                            previous: Some(at),
                            start,
                        },
                    );
                    queue.insert(fill.end);
                }
            }
        }
        let mut pages = vec![];
        let mut at = end;
        while let Some(token) = at {
            let node = &nodes[&token];
            at = node.previous;
            if let Some(previous) = node.previous {
                pages.push(previous);
            }
        }
        pages.reverse();
        if pages.is_empty() {
            pages.push(first.at);
        }
        Solution { pages }
    }
}
