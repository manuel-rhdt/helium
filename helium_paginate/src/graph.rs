//! Pagination as a search in a graph.
//!
//! The ways to paginate a document are the paths from the start of the document to its end in a
//! graph. A node is a place where a page can start, with everything about the path that led there
//! that the pages that follow depend on (the kind of the page, its number if the number of pages
//! is limited), and nothing else. An edge is a page, from the place where it starts to a place
//! where it can end. The graph is a DAG, and it is never built as a whole: the edges of a node
//! are made when the search gets to it ([`Graph::edges`]).
//!
//! The edges carry facts (the [`PageFill`] of a page), and the cost of a path is made from the
//! facts by an [`Objective`](crate::Objective), so that the graph and the costs can be changed
//! independently.
//!
//! [`shortest_path`] finds the cheapest path by taking the nodes in the order of the flow, so
//! that by the time a node is taken its cost is final. It is the exact search of the least-weight
//! subsequence problem, and it is the only one that is written for a graph; the policies that
//! choose among the edges of a node (greedy, beam search, a learned one) are written against
//! the same trait.

use crate::kernel::{Kernel, PageFill, PageStart};
use crate::token::BreakToken;
use crate::PageKind;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// A graph that is searched for the cheapest path from its start to one of its ends.
pub trait Graph {
    /// A place in the graph: all that is needed to make the edges that leave it.
    type Node: Clone;
    /// A step from a node to the next: the facts that its cost is made from.
    type Edge;
    /// What identifies a node in a search. Nodes with the same key have the same future, so
    /// only the cheapest way to one of them is kept.
    type Key: Copy + Eq + Hash;
    /// The order in which the nodes are taken: every edge goes to a node of a greater rank than
    /// the node it leaves.
    type Rank: Ord;

    fn start(&mut self) -> Self::Node;
    fn key(&mut self, node: &Self::Node) -> Self::Key;
    fn rank(&mut self, key: Self::Key) -> Self::Rank;
    /// Whether a path can end at the node.
    fn is_end(&mut self, node: &Self::Node) -> bool;
    /// The edges that leave a node, with the node that each of them goes to. They are in a fixed
    /// order, which decides between paths of the same cost that are found first.
    fn edges(&mut self, node: &Self::Node) -> Vec<(Self::Edge, Self::Node)>;
}

/// The nodes of a path from the start to the node where it ends, the node where it ends, and the
/// edges that it takes.
#[derive(Debug, Clone)]
pub struct Path<N, E> {
    /// The nodes that an edge leaves, from the start on. The end is not among them.
    pub nodes: Vec<N>,
    /// The edges: `edges[i]` leaves `nodes[i]`.
    pub edges: Vec<E>,
    pub end: N,
}

struct Entry<G: Graph, C> {
    cost: C,
    previous: Option<G::Key>,
    node: G::Node,
    /// The edge that led to the node.
    edge: Option<G::Edge>,
}

/// The cheapest path from the start of the graph to one of its ends.
///
/// `weigh` gives the cost of an edge, or `None` if it cannot be taken. `combine` adds the cost of
/// an edge to the cost of the path in front of it. It must not make a cost better when the cost
/// of the path in front or of the edge gets worse (as the sum of non-negative costs and the
/// maximum do), or a prefix of a cheapest path would not be a cheapest path.
///
/// Where two paths cost the same, the one whose last node starts earlier wins, and where those are
/// the same, the one whose node before it starts earlier, and so on.
pub fn shortest_path<G, C>(
    graph: &mut G,
    zero: C,
    weigh: &mut dyn FnMut(&G::Edge) -> Option<C>,
    combine: &mut dyn FnMut(&C, &C, &G::Edge) -> C,
) -> Option<Path<G::Node, G::Edge>>
where
    G: Graph,
    G::Edge: Clone,
    C: Clone + PartialOrd,
{
    let start = graph.start();
    let start_key = graph.key(&start);
    let mut entries: HashMap<G::Key, Entry<G, C>> = HashMap::new();
    let start_rank = graph.rank(start_key);
    entries.insert(
        start_key,
        Entry {
            cost: zero,
            previous: None,
            node: start,
            edge: None,
        },
    );
    // The nodes are taken in the order of the flow: all edges that end at a node leave an earlier
    // one, so by the time a node is taken from the queue its cost is final.
    let mut queue = BTreeMap::from([(start_rank, start_key)]);
    let mut ends: Vec<G::Key> = vec![];
    while let Some((_, key)) = queue.pop_first() {
        let node = entries[&key].node.clone();
        if graph.is_end(&node) {
            ends.push(key);
            continue;
        }
        let cost_so_far = entries[&key].cost.clone();
        for (edge, next) in graph.edges(&node) {
            let Some(page) = weigh(&edge) else {
                continue;
            };
            let cost = combine(&cost_so_far, &page, &edge);
            // (a cost that is not comparable, as NaN is, cannot be better than anything)
            if cost.partial_cmp(&cost).is_none() {
                continue;
            }
            let next_key = graph.key(&next);
            let better = entries.get(&next_key).is_none_or(|entry| cost < entry.cost);
            if better {
                entries.insert(
                    next_key,
                    Entry {
                        cost,
                        previous: Some(key),
                        node: next,
                        edge: Some(edge),
                    },
                );
                let rank = graph.rank(next_key);
                queue.insert(rank, next_key);
            }
        }
    }
    // The nodes of a way to an end, from the last to the first.
    let chain = |end: G::Key| {
        let mut keys = vec![];
        let mut at = entries[&end].previous;
        while let Some(key) = at {
            keys.push(key);
            at = entries[&key].previous;
        }
        keys
    };
    let mut cheapest: Option<&C> = None;
    for key in &ends {
        let cost = &entries[key].cost;
        if cheapest.is_none_or(|best| cost < best) {
            cheapest = Some(cost);
        }
    }
    let cheapest = cheapest?.clone();
    // Among the cheapest ways, the one with the earlier start of the last page wins, and where
    // those are the same, the one with the earlier start of the page before it, and so on.
    let mut best: Option<(Vec<G::Rank>, G::Rank, G::Key)> = None;
    for &end in ends.iter().filter(|end| entries[*end].cost == cheapest) {
        let ranks: Vec<G::Rank> = chain(end).into_iter().map(|key| graph.rank(key)).collect();
        let rank = graph.rank(end);
        let better = match &best {
            None => true,
            Some((best_ranks, best_rank, _)) => (&ranks, &rank) < (best_ranks, best_rank),
        };
        if better {
            best = Some((ranks, rank, end));
        }
    }
    let (_, _, end) = best?;
    let mut nodes: Vec<G::Node> = chain(end).into_iter().map(|key| entries[&key].node.clone()).collect();
    nodes.reverse();
    // the edges are the ones that led to the nodes after the first, and to the end
    let mut edges: Vec<G::Edge> = vec![];
    let mut at = Some(end);
    while let Some(key) = at {
        if let Some(edge) = &entries[&key].edge {
            edges.push(edge.clone());
        }
        at = entries[&key].previous;
    }
    edges.reverse();
    Some(Path {
        nodes,
        edges,
        end: entries[&end].node.clone(),
    })
}

/// A page of the graph of a [`Kernel`]: the facts of the page and what the cost needs to know
/// about its place.
#[derive(Debug, Clone, Copy)]
pub struct PageEdge {
    pub fill: PageFill,
    /// Whether the page is the last one of the document.
    pub last: bool,
    /// The number of the page; the first page has the number 0.
    pub index: usize,
}

/// The graph of the pages of a [`Kernel`]: the nodes are the places where a page starts, with the
/// kind of the page and its number if the number of pages is limited, and the edges are the
/// pages that [`Kernel::candidates`] lists.
pub struct PageGraph<'k, 'a> {
    kernel: &'k mut Kernel<'a>,
}

impl<'k, 'a> PageGraph<'k, 'a> {
    pub fn new(kernel: &'k mut Kernel<'a>) -> Self {
        PageGraph { kernel }
    }
}

/// The place where a page starts, its kind, and its number if the number of pages is limited:
/// the geometry of a page, and with it everything that follows, depends on nothing else about
/// the path that led to it.
pub type PageKey = (BreakToken, PageKind, usize);

impl Graph for PageGraph<'_, '_> {
    type Node = PageStart;
    type Edge = PageEdge;
    type Key = PageKey;
    type Rank = ((usize, u64), u8, usize);

    fn start(&mut self) -> PageStart {
        self.kernel.first_page()
    }

    fn key(&mut self, node: &PageStart) -> PageKey {
        let (token, kind) = self.kernel.state(node);
        (token, kind, self.kernel.slot(node))
    }

    fn rank(&mut self, key: PageKey) -> Self::Rank {
        (self.kernel.position(key.0).order(), key.1 as u8, key.2)
    }

    fn is_end(&mut self, node: &PageStart) -> bool {
        self.kernel.is_final(node)
    }

    fn edges(&mut self, node: &PageStart) -> Vec<(PageEdge, PageStart)> {
        let mut edges = vec![];
        for fill in self.kernel.candidates(node) {
            let next = self.kernel.next_page(node, &fill);
            let last = next.is_none();
            let next = next.unwrap_or(PageStart {
                at: fill.end,
                index: node.index + 1,
            });
            edges.push((
                PageEdge {
                    fill,
                    last,
                    index: node.index,
                },
                next,
            ));
        }
        edges
    }
}
