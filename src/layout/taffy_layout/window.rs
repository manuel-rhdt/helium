//! The window: the part of the document that Taffy is asked to lay out for one page.
//!
//! The paginator drives, and Taffy answers (see `docs/pagination.md`). Taffy knows nothing of
//! pages, so [`HeliumTree`] shows it a *window* of the tree instead of the whole of it:
//!
//! - the children of a box are the ones that have items of the flow in the window;
//! - a box that is cut has no padding, no border and no margin at the cut edge, and no minimum
//!   height (the height of the fragment is what is in it);
//! - the margins that adjoin the edge of the page are discarded at the end of the page and at the
//!   start of a page that follows an unforced break. At a forced break they are replaced by the
//!   collapsed margin that the break has (the gap of the flow);
//! - a paragraph that is cut has the lines that the paginator has chosen for the page (the
//!   measure function does not run the line breaker for the whole of it).
//!
//! The nodes whose style or children differ from the whole document must not share the cache of
//! Taffy with the same nodes laid out whole: the cache is keyed by the sizes that Taffy asks
//! for, and nothing in the key says which window it was computed in. These nodes bypass the
//! cache (`Node::windowed`).

use super::tree::{Content, HeliumTree, Node, node_is_out_of_flow};
use helium_paginate::Cut;
use std::collections::HashMap;
use taffy::tree::NodeId;
use taffy::{LengthPercentage, LengthPercentageAuto};

/// Where a box is in the flow of the document: the items of the flow that are made from it.
#[derive(Debug, Clone)]
pub(super) struct Span {
    /// The first item and the item behind the last one.
    pub start: u32,
    pub end: u32,
    /// The boxes of the flow in it, in order: the children of a block container, the parts of a
    /// table. Empty for boxes that are not split.
    pub kids: Vec<NodeId>,
    /// Whether the box is split between its kids.
    pub split: bool,
    /// Whether the box is out of the flow, or not shown: the items of the box in front of it
    /// are `start..end`, and it is part of the window of the page that has the item at `start`.
    pub anchored: bool,
}

/// What the window of a page asks for.
#[derive(Debug, Clone)]
pub(super) struct Window {
    /// The items of the page: `start..end`.
    pub start: u32,
    pub end: u32,
    /// The paragraph that the page starts in, and where it continues.
    pub start_cut: Option<(NodeId, Cut)>,
    /// The paragraph that the page ends in, and where the next page continues.
    pub end_cut: Option<(NodeId, Cut)>,
    /// What happens to the margins in front of the content of the page.
    pub leading: Leading,
    /// Whether the page ends where the document does, so that nothing cuts the margins at its
    /// end.
    pub at_end: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Leading {
    /// The start of the document: margins are what the style sheet says.
    Natural,
    /// After an unforced break: the margins are discarded.
    Discard,
    /// After a forced break: the margin of the first box of the page is the gap of the flow.
    Gap(f32),
}

/// How the style of a box is changed in a window.
#[derive(Debug, Default, Clone, Copy)]
struct Changes {
    /// No padding and no border at the top.
    top_edge: bool,
    bottom_edge: bool,
    margin_top: Option<f32>,
    margin_bottom: Option<f32>,
    min_height: bool,
}

impl Window {
    /// Whether a box that spans the items `start..end` has items in front of the window.
    pub fn open_at_start(&self, span: &Span) -> bool {
        span.start < self.start && span.end > self.start
    }

    /// Whether a box has items behind the window.
    pub fn open_at_end(&self, span: &Span) -> bool {
        span.start < self.end && span.end > self.end
    }

    /// The paragraph that the page starts in or ends in, and the cuts.
    pub fn cut_of(&self, id: NodeId) -> Option<(Option<Cut>, Option<Cut>)> {
        let from = self.start_cut.filter(|&(node, _)| node == id).map(|(_, cut)| cut);
        let to = self.end_cut.filter(|&(node, _)| node == id).map(|(_, cut)| cut);
        (from.is_some() || to.is_some()).then_some((from, to))
    }

    /// The kids of a box that have items in the window, as a part of the list of its children.
    pub fn children<'n>(&self, nodes: &[Node], node: &'n Node) -> &'n [NodeId] {
        let children = &node.children;
        if node.span.as_ref().is_none_or(|span| !span.split) {
            return children;
        }
        let span_of = |id: &NodeId| nodes[usize::from(*id)].span.as_ref();
        let first = children.partition_point(|id| span_of(id).is_some_and(|span| span.end <= self.start));
        let last = children.partition_point(|id| span_of(id).is_none_or(|span| span.start < self.end));
        &children[first..last.max(first)]
    }
}

fn is_zero(value: LengthPercentage) -> bool {
    let raw = value.into_raw();
    raw.tag() == taffy::CompactLength::LENGTH_TAG && raw.value() == 0.0
}

impl HeliumTree<'_, '_> {
    /// The style that Taffy lays a box out with: the style of the box, changed for the window.
    pub(super) fn active_style(&self, id: NodeId) -> &taffy::Style {
        let node = self.node(id);
        node.window_style.as_ref().unwrap_or(&node.style)
    }

    /// The children of a box that Taffy sees: the ones in the window.
    pub(super) fn window_children(&self, id: NodeId) -> &[NodeId] {
        let node = self.node(id);
        match &self.window {
            None => &node.children,
            Some(window) => {
                // a paragraph that the page continues has no out-of-flow boxes of its own
                if node.content == Content::Inline && window.start_cut.is_some_and(|(cut, _)| cut == id) {
                    return &[];
                }
                window.children(&self.nodes, node)
            }
        }
    }

    /// Whether a box has a padding or a border at its top or its bottom.
    fn has_edge(&self, id: NodeId, top: bool) -> bool {
        let style = &self.node(id).style;
        if top {
            !is_zero(style.padding.top) || !is_zero(style.border.top)
        } else {
            !is_zero(style.padding.bottom) || !is_zero(style.border.bottom)
        }
    }

    /// The first (last) kid of a box that is in the window.
    fn edge_kid(&self, id: NodeId, window: &Window, first: bool) -> Option<NodeId> {
        let node = self.node(id);
        if node.content != Content::Blocks || !node.span.as_ref().is_some_and(|span| span.split) {
            return None;
        }
        let children = window.children(&self.nodes, node);
        let in_flow = |&&child: &&NodeId| {
            self.node(child).span.as_ref().is_some_and(|span| !span.anchored)
                && !node_is_out_of_flow(self, child)
        };
        if first {
            children.iter().find(in_flow).copied()
        } else {
            children.iter().rev().find(in_flow).copied()
        }
    }

    /// Takes the window off the tree.
    pub(super) fn clear_window(&mut self) {
        self.window = None;
        for id in std::mem::take(&mut self.windowed) {
            let node = self.node_mut(id);
            node.window_style = None;
            node.windowed = false;
        }
    }

    /// Shows Taffy a window of the tree.
    pub(super) fn set_window(&mut self, window: Window, root: NodeId) {
        self.clear_window();
        let mut changes: HashMap<NodeId, Changes> = HashMap::new();

        // The boxes at the start of the window: the ones that are cut, and the ones that the
        // margins of the start of the page collapse through. With a gap, the first box that is
        // not cut has the gap as its margin and nothing below it adds to it.
        let mut given = false;
        let mut current = Some(root);
        while let Some(id) = current {
            let open = self.node(id).span.as_ref().is_some_and(|span| window.open_at_start(span));
            let change = changes.entry(id).or_default();
            let table = self.node(id).content == Content::Table;
            if open {
                change.top_edge = !table;
                change.margin_top = Some(0.0);
                change.min_height = !table;
            } else {
                match window.leading {
                    Leading::Natural => {}
                    Leading::Discard => change.margin_top = Some(0.0),
                    Leading::Gap(gap) => {
                        change.margin_top = Some(if given { 0.0 } else { gap });
                        given = true;
                    }
                }
            }
            if !open && self.has_edge(id, true) {
                break;
            }
            current = self.edge_kid(id, &window, true);
        }

        // the boxes at the end of the window
        let mut current = Some(root);
        while let Some(id) = current {
            let open = self.node(id).span.as_ref().is_some_and(|span| window.open_at_end(span));
            let change = changes.entry(id).or_default();
            let table = self.node(id).content == Content::Table;
            if open {
                change.bottom_edge = !table;
                change.margin_bottom = Some(0.0);
                change.min_height = !table;
            } else if !window.at_end {
                change.margin_bottom = Some(0.0);
            }
            if !open && self.has_edge(id, false) {
                break;
            }
            current = self.edge_kid(id, &window, false);
        }

        let mut windowed = vec![];
        for (id, change) in changes {
            let mut style = self.node(id).style.clone();
            if change.top_edge {
                style.padding.top = LengthPercentage::length(0.0);
                style.border.top = LengthPercentage::length(0.0);
            }
            if change.bottom_edge {
                style.padding.bottom = LengthPercentage::length(0.0);
                style.border.bottom = LengthPercentage::length(0.0);
            }
            if let Some(margin) = change.margin_top {
                style.margin.top = LengthPercentageAuto::length(margin);
            }
            if let Some(margin) = change.margin_bottom {
                style.margin.bottom = LengthPercentageAuto::length(margin);
            }
            if change.min_height {
                style.min_size.height = LengthPercentageAuto::auto();
            }
            let node = self.node_mut(id);
            node.window_style = Some(style);
            node.windowed = true;
            windowed.push(id);
        }
        // a paragraph that is cut has a window of its own, and so has its box
        for (id, _) in window.start_cut.iter().chain(window.end_cut.iter()) {
            let node = self.node_mut(*id);
            node.windowed = true;
            if !windowed.contains(id) {
                windowed.push(*id);
            }
        }
        self.windowed = windowed;
        self.window = Some(window);
    }
}

/// The part of a table or a row group that is in the window, as the top and the bottom of the
/// fragment relative to the top of the box. `full` is the height of the box as a whole.
pub(super) fn table_fragment(nodes: &[Node], window: &Window, node: &Node, full: f32) -> (f32, f32) {
    let Some(span) = node.span.as_ref() else {
        return (0.0, full);
    };
    let in_window = |id: &NodeId| {
        nodes[usize::from(*id)]
            .span
            .as_ref()
            .is_some_and(|span| span.start < window.end && span.end > window.start)
    };
    let mut kids = span.kids.iter().filter(|id| in_window(id));
    let (Some(first), last) = (kids.next(), kids.last()) else {
        return (0.0, full);
    };
    let first = &nodes[usize::from(*first)];
    let last = last.map_or(first, |last| &nodes[usize::from(*last)]);
    // the part of a kid that is a row group is cut, too
    let part = |kid: &Node| {
        if kid.span.as_ref().is_some_and(|span| !span.split && !span.kids.is_empty()) {
            table_fragment(nodes, window, kid, kid.layout.size.height)
        } else {
            (0.0, kid.layout.size.height)
        }
    };
    let top = if window.open_at_start(span) { first.layout.location.y + part(first).0 } else { 0.0 };
    let bottom = if window.open_at_end(span) {
        last.layout.location.y + part(last).1
    } else {
        full
    };
    (top, bottom)
}
