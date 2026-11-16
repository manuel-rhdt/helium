//! The flow of the document: the boxes, gaps, breaks and paragraphs that the paginator chooses
//! the pages from (see `docs/pagination.md`).
//!
//! The flow is made from the box tree, once. It says what can be split and where, and nothing
//! about sizes: the layout service has them ([`super::service`]).
//!
//! - A block container becomes the box of its top edge (padding and border), its kids, which are
//!   separated by breaks and gaps, and the box of its bottom edge.
//! - A paragraph becomes its two edges and a paragraph item between them.
//! - Everything else is one box that cannot be split: replaced boxes, flex and grid containers,
//!   table rows, boxes with a height, orthogonal flows.
//! - A table is split between its rows (and row groups and captions), like a block container.
//!
//! Every box that is part of the flow knows its items ([`Span`]), which are what the window of a
//! page is made from.
//!
//! A multi-column container is one box of the flow of its page. Its content is a flow of its own
//! ([`Inner`]), which the columns are chosen from. The items of both are numbered from 0 (the
//! spans of the boxes in the content of the container are in the numbers of its flow), the
//! extents and the paragraphs have one numbering for all of them.

use super::tree::{Content, HeliumTree, node_is_out_of_flow};
use super::window::Span;
use crate::style::properties::computed::Break;
use crate::style::properties::specified::Display;
use helium_paginate::{BreakKind, BreakLevel, Item, NodeRef, ParId};
use taffy::tree::NodeId;

/// A place in a box that the layout of a galley knows the position of.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum Anchor {
    /// The top of the border box.
    Top(NodeId),
    /// The top of the content box, behind the padding and the border.
    ContentTop(NodeId),
    ContentBottom(NodeId),
    /// The bottom of the border box.
    Bottom(NodeId),
}

/// The flow of the content of a multi-column container: the items that its columns are chosen
/// from.
#[derive(Debug, Clone)]
pub(super) struct Inner {
    pub items: Vec<Item>,
    /// For every item, the box or the paragraph of it if it is one.
    pub leaves: Vec<Option<NodeId>>,
    /// Where the items of the content are: the span that the container has in its own flow. Its
    /// kids are the boxes of the content, and it is split between them if the content is made of
    /// blocks.
    pub span: Span,
}

/// The flow of a document.
#[derive(Debug)]
pub(super) struct Flow {
    pub items: Vec<Item>,
    /// The extent of the boxes and gaps of the flow, by `NodeRef`: from one anchor to the other.
    pub extents: Vec<(Anchor, Anchor)>,
    /// The box of every paragraph, by `ParId`.
    pub paragraphs: Vec<NodeId>,
    /// For every item, the box or the paragraph of it if it is one.
    pub leaves: Vec<Option<NodeId>>,
    pub root: NodeId,
    /// The box that a box is in, by the index of the node.
    pub parents: Vec<Option<NodeId>>,
    /// Whether the pages are bound on the right.
    pub rtl: bool,
    /// The number of extents that are the ones of the flow of the page: the extents of the flows
    /// of the multi-column containers come behind them.
    pub page_extents: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A box that is not split.
    Whole,
    Paragraph,
    Block,
    Table,
}

/// What `break-before` or `break-after` values of the boxes at a break add up to.
#[derive(Debug, Default, Clone, Copy)]
struct Breaks {
    forced_page: bool,
    forced_column: bool,
    avoid_page: bool,
    avoid_column: bool,
    /// The side that a forced break asks for.
    side: Option<Side>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// The kinds of break that `break-inside` of the boxes around a box asks to avoid.
#[derive(Debug, Default, Clone, Copy)]
struct Avoid {
    page: bool,
    column: bool,
}

impl Avoid {
    /// Whether the breaks of the level inside the box are to be avoided.
    fn at(self, level: BreakLevel) -> bool {
        match level {
            BreakLevel::Page => self.page,
            BreakLevel::Column => self.column,
        }
    }
}

impl Breaks {
    fn add(&mut self, value: Break, rtl: bool) {
        self.forced_page |= value.forces_page();
        self.forced_column |= value.forces_column();
        self.avoid_page |= value.avoids_page();
        self.avoid_column |= value.avoids_column();
        let side = match value {
            Break::Left => Some(Side::Left),
            Break::Right => Some(Side::Right),
            // the recto page is the right one in a document that is set from left to right
            Break::Recto => Some(if rtl { Side::Left } else { Side::Right }),
            Break::Verso => Some(if rtl { Side::Right } else { Side::Left }),
            _ => None,
        };
        self.side = self.side.or(side);
    }

    fn merge(&mut self, other: Breaks) {
        self.forced_page |= other.forced_page;
        self.forced_column |= other.forced_column;
        self.avoid_page |= other.avoid_page;
        self.avoid_column |= other.avoid_column;
        self.side = self.side.or(other.side);
    }
}

/// The boxes that are positioned in the initial containing block by their `top` or `bottom`
/// are on the first page: they become children of the root, in front of its other children.
/// (A box with a static position is on the page of the content in front of it.)
fn pin_to_first_page(tree: &mut HeliumTree, root: NodeId) {
    fn collect(tree: &HeliumTree, id: NodeId, positioned: bool, found: &mut Vec<(NodeId, NodeId)>) {
        for &child in &tree.node(id).children {
            let style = &tree.node(child).style;
            let is_positioned = style.position != taffy::Position::Static;
            let pinned = style.position == taffy::Position::Absolute
                && !positioned
                && !(style.inset.top.is_auto() && style.inset.bottom.is_auto());
            if pinned {
                found.push((id, child));
            }
            collect(tree, child, positioned || is_positioned, found);
        }
    }
    let mut found = vec![];
    collect(tree, root, false, &mut found);
    let mut pinned = vec![];
    for (parent, child) in found {
        tree.node_mut(parent).children.retain(|&other| other != child);
        pinned.push(child);
    }
    let children = &mut tree.node_mut(root).children;
    pinned.extend(children.iter().copied());
    *children = pinned;
}

pub(super) fn extract(tree: &mut HeliumTree, root: NodeId, rtl: bool) -> Flow {
    pin_to_first_page(tree, root);
    let nodes = tree.nodes.len();
    let mut extractor = Extractor {
        tree,
        flow: Flow {
            items: vec![],
            extents: vec![],
            paragraphs: vec![],
            leaves: vec![],
            root,
            parents: vec![None; nodes],
            rtl,
            page_extents: 0,
        },
        level: BreakLevel::Page,
    };
    extractor.emit(root, None, Avoid::default());
    extractor.flow.page_extents = extractor.flow.extents.len();
    // the content of the containers with columns, which are no part of the flow of the page
    for index in 0..nodes {
        let id = NodeId::from(index);
        if extractor.tree.node(id).columns.is_some() {
            extractor.emit_columns(id);
        }
    }
    // the parents of the boxes that are not in a flow are needed to find the boxes around a box
    for index in 0..nodes {
        let id = NodeId::from(index);
        let node = extractor.tree.node(id);
        let kids: Vec<NodeId> = node.children.iter().chain(node.atoms.iter()).copied().collect();
        for kid in kids {
            let parent = &mut extractor.flow.parents[usize::from(kid)];
            if parent.is_none() && kid != root {
                *parent = Some(id);
            }
        }
    }
    extractor.flow
}

struct Extractor<'t, 'a, 'w> {
    tree: &'t mut HeliumTree<'a, 'w>,
    flow: Flow,
    /// What the pages are that the flow is made for: the pages of the document, or the columns
    /// of a multi-column container.
    level: BreakLevel,
}

impl Extractor<'_, '_, '_> {
    fn push(&mut self, item: Item) -> u32 {
        self.flow.items.push(item);
        self.flow.leaves.push(None);
        self.flow.items.len() as u32 - 1
    }

    fn extent(&mut self, from: Anchor, to: Anchor) -> NodeRef {
        self.flow.extents.push((from, to));
        NodeRef(self.flow.extents.len() as u32 - 1)
    }

    fn box_item(&mut self, from: Anchor, to: Anchor) -> u32 {
        let node = self.extent(from, to);
        self.push(Item::Box { node })
    }

    fn gap_item(&mut self, from: Anchor, to: Anchor) {
        let node = self.extent(from, to);
        self.push(Item::Gap { node });
    }

    /// Whether a box has a size of its own in the block direction, which a box that is split
    /// would not have.
    fn has_block_size(&self, id: NodeId) -> bool {
        let style = &self.tree.node(id).style;
        !style.size.height.is_auto()
            || !style.max_size.height.is_auto()
            || style.aspect_ratio.is_some()
    }

    fn in_flow(&self, id: NodeId) -> bool {
        !node_is_out_of_flow(self.tree, id) && self.tree.node(id).style.display != taffy::Display::None
    }

    fn in_flow_children(&self, id: NodeId) -> Vec<NodeId> {
        self.tree
            .node(id)
            .children
            .iter()
            .copied()
            .filter(|&child| self.in_flow(child))
            .collect()
    }

    fn display(&self, id: NodeId) -> Option<Display> {
        self.tree
            .computed_values
            .get(self.tree.node(id).entity)
            .map(|style| style.box_.display)
    }

    fn classify(&self, id: NodeId) -> Kind {
        let node = self.tree.node(id);
        if node.orthogonal || node.flow.is_vertical() || self.has_block_size(id) {
            return Kind::Whole;
        }
        // (a container with columns is one box in the flow of its page)
        if node.columns.is_some() && self.level == BreakLevel::Page {
            return Kind::Whole;
        }
        match node.content {
            Content::Replaced => Kind::Whole,
            Content::Inline => {
                let lines = self.tree.inline_ctxs.get(node.entity).is_some_and(|ifc| ifc.break_points.len() >= 2);
                if lines { Kind::Paragraph } else { Kind::Whole }
            }
            // a table is not split between columns
            Content::Table => {
                if self.level == BreakLevel::Page && self.tree.table_kids(id).is_some() {
                    Kind::Table
                } else {
                    Kind::Whole
                }
            }
            Content::Blocks => {
                let display = self.display(id);
                let monolithic = display.is_some_and(|display| {
                    display.is_flex_or_grid()
                        || matches!(
                            display,
                            Display::TableRow | Display::TableCell | Display::TableCaption
                        )
                });
                if monolithic || self.in_flow_children(id).is_empty() {
                    Kind::Whole
                } else {
                    Kind::Block
                }
            }
        }
    }

    /// The last (first) in-flow box in a box, which the `break-after` (`break-before`) of its
    /// parent propagates to.
    /// https://www.w3.org/TR/css-break-3/#break-propagation
    fn edge_kid(&self, id: NodeId, last: bool) -> Option<NodeId> {
        let node = self.tree.node(id);
        // breaks inside a container with columns are breaks between its columns
        if node.columns.is_some() {
            return None;
        }
        let kids: Vec<NodeId> = match node.content {
            Content::Blocks => self.in_flow_children(id),
            Content::Table => node.table.iter().flat_map(|table| table.boxes.iter().copied()).collect(),
            _ => return None,
        };
        if last { kids.last().copied() } else { kids.first().copied() }
    }

    fn breaks_of(&self, mut id: NodeId, after: bool) -> Breaks {
        let mut breaks = Breaks::default();
        loop {
            if let Some(style) = self.tree.computed_values.get(self.tree.node(id).entity) {
                let value = if after { style.page.break_after } else { style.page.break_before };
                breaks.add(value, self.flow.rtl);
            }
            match self.edge_kid(id, after) {
                Some(kid) => id = kid,
                None => return breaks,
            }
        }
    }

    fn page_name(&self, id: NodeId) -> Option<Option<std::sync::Arc<str>>> {
        self.tree
            .computed_values
            .get(self.tree.node(id).entity)
            .map(|style| style.page.page.0.clone())
    }

    /// The kind of the break between two boxes that follow each other, and the level of the
    /// value that decides it. A value of the other level than the one of the flow is no break
    /// there, and the kernel knows it from the level.
    fn break_kind(&self, before: NodeId, after: NodeId, avoid: Avoid) -> (BreakKind, BreakLevel) {
        let mut breaks = self.breaks_of(before, true);
        breaks.merge(self.breaks_of(after, false));
        // a change of the page name forces a page break
        if let (Some(a), Some(b)) = (self.page_name(before), self.page_name(after)) {
            breaks.forced_page |= a != b;
        }
        breaks.avoid_page |= avoid.page;
        breaks.avoid_column |= avoid.column;
        let forced = |side: Option<Side>| match side {
            Some(Side::Left) => BreakKind::ForcedLeft,
            Some(Side::Right) => BreakKind::ForcedRight,
            None => BreakKind::Forced,
        };
        let (forced_here, forced_there, avoid_here, avoid_there, there) = match self.level {
            BreakLevel::Page => {
                (breaks.forced_page, breaks.forced_column, breaks.avoid_page, breaks.avoid_column, BreakLevel::Column)
            }
            BreakLevel::Column => {
                (breaks.forced_column, breaks.forced_page, breaks.avoid_column, breaks.avoid_page, BreakLevel::Page)
            }
        };
        if forced_here {
            // (a side is a property of a page)
            let side = if self.level == BreakLevel::Page { breaks.side } else { None };
            (forced(side), self.level)
        } else if avoid_here {
            (BreakKind::Avoid, self.level)
        } else if forced_there {
            (BreakKind::Forced, there)
        } else if avoid_there {
            (BreakKind::Avoid, there)
        } else {
            (BreakKind::Allowed, self.level)
        }
    }

    fn push_break(&mut self, before: NodeId, after: NodeId, avoid: Avoid) {
        let (kind, level) = self.break_kind(before, after, avoid);
        self.push(Item::Break { kind, level });
    }

    /// Emits the items of a box and sets its span.
    fn emit(&mut self, id: NodeId, parent: Option<NodeId>, avoid: Avoid) {
        self.flow.parents[usize::from(id)] = parent;
        let start = self.flow.items.len() as u32;
        let kind = self.classify(id);
        let inside = self
            .tree
            .computed_values
            .get(self.tree.node(id).entity)
            .map(|style| style.page.break_inside);
        let avoid = Avoid {
            page: avoid.page || inside.is_some_and(|inside| inside.avoids_page()),
            column: avoid.column || inside.is_some_and(|inside| inside.avoids_column()),
        };
        let mut kids = vec![];
        match kind {
            Kind::Whole => {
                let item = self.box_item(Anchor::Top(id), Anchor::Bottom(id));
                self.flow.leaves[item as usize] = Some(id);
            }
            Kind::Paragraph => {
                let page = self.tree.computed_values.get(self.tree.node(id).entity).map(|style| &style.page);
                let (orphans, widows) = page.map_or((2, 2), |page| (page.orphans, page.widows));
                self.box_item(Anchor::Top(id), Anchor::ContentTop(id));
                let paragraph = ParId(self.flow.paragraphs.len() as u32);
                self.flow.paragraphs.push(id);
                let item = self.push(Item::Paragraph {
                    id: paragraph,
                    orphans: orphans as u32,
                    widows: widows as u32,
                    avoid_inside: avoid.at(self.level),
                });
                self.flow.leaves[item as usize] = Some(id);
                self.box_item(Anchor::ContentBottom(id), Anchor::Bottom(id));
            }
            Kind::Block | Kind::Table => {
                let split = kind == Kind::Block;
                self.box_item(Anchor::Top(id), Anchor::ContentTop(id));
                let mut end = Anchor::ContentTop(id);
                let mut previous: Option<NodeId> = None;
                if split {
                    self.emit_blocks(id, avoid, &mut end, &mut kids);
                } else {
                    let table_kids = self.tree.table_kids(id).unwrap_or_default();
                    self.emit_table_kids(id, id, &table_kids, avoid, &mut end, &mut previous, &mut kids);
                    // the out-of-flow children of the table are anchored to its start
                    for &child in &self.tree.node(id).children.clone() {
                        if !self.in_flow(child) {
                            let anchor = start;
                            self.tree.node_mut(child).span = Some(Span {
                                start: anchor,
                                end: anchor + 1,
                                kids: vec![],
                                split: false,
                                anchored: true,
                            });
                        }
                    }
                }
                self.gap_item(end, Anchor::ContentBottom(id));
                self.box_item(Anchor::ContentBottom(id), Anchor::Bottom(id));
            }
        }
        let end = self.flow.items.len() as u32;
        self.tree.node_mut(id).span = Some(Span {
            start,
            end,
            kids,
            split: kind == Kind::Block,
            anchored: false,
        });
    }

    /// The children of a block container: the breaks and gaps between them, and their items.
    /// `end` is where the box in front of the first one ends, and where the last one does.
    fn emit_blocks(&mut self, id: NodeId, avoid: Avoid, end: &mut Anchor, kids: &mut Vec<NodeId>) {
        let mut previous: Option<NodeId> = None;
        for &child in &self.tree.node(id).children.clone() {
            if !self.in_flow(child) {
                // out of the flow: shown on the page of the item in front of it
                let anchor = (self.flow.items.len() as u32).saturating_sub(1);
                self.tree.node_mut(child).span = Some(Span {
                    start: anchor,
                    end: anchor + 1,
                    kids: vec![],
                    split: false,
                    anchored: true,
                });
                continue;
            }
            if let Some(before) = previous {
                self.push_break(before, child, avoid);
            }
            self.gap_item(*end, Anchor::Top(child));
            self.emit(child, Some(id), avoid);
            *end = Anchor::Bottom(child);
            previous = Some(child);
            kids.push(child);
        }
    }

    /// The content of a multi-column container, as a flow of its own.
    fn emit_columns(&mut self, id: NodeId) {
        let items = std::mem::take(&mut self.flow.items);
        let leaves = std::mem::take(&mut self.flow.leaves);
        self.level = BreakLevel::Column;
        let mut kids = vec![];
        let split = self.tree.node(id).content == Content::Blocks;
        if split {
            let mut end = Anchor::ContentTop(id);
            self.emit_blocks(id, Avoid::default(), &mut end, &mut kids);
        } else {
            // the content is one paragraph, which is the container itself
            let page = self.tree.computed_values.get(self.tree.node(id).entity).map(|style| &style.page);
            let (orphans, widows) = page.map_or((2, 2), |page| (page.orphans, page.widows));
            let paragraph = ParId(self.flow.paragraphs.len() as u32);
            self.flow.paragraphs.push(id);
            let item = self.push(Item::Paragraph {
                id: paragraph,
                orphans: orphans as u32,
                widows: widows as u32,
                avoid_inside: false,
            });
            self.flow.leaves[item as usize] = Some(id);
        }
        self.level = BreakLevel::Page;
        let items = std::mem::replace(&mut self.flow.items, items);
        let leaves = std::mem::replace(&mut self.flow.leaves, leaves);
        let span = Span { start: 0, end: items.len() as u32, kids, split, anchored: false };
        if let Some(columns) = self.tree.node_mut(id).columns.as_mut() {
            columns.inner = Some(Inner { items, leaves, span });
        }
    }

    /// The boxes of a table. Breaks are only possible where no cell spans over them.
    fn emit_table_kids(
        &mut self,
        table: NodeId,
        owner: NodeId,
        boxes: &[super::table::TableKid],
        avoid: Avoid,
        end: &mut Anchor,
        previous: &mut Option<NodeId>,
        kids: &mut Vec<NodeId>,
    ) {
        let mut last_rows: Option<std::ops::Range<usize>> = None;
        for kid in boxes {
            if let Some(before) = *previous {
                // a row group and its rows are kids of the same table: break between the boxes
                // unless a cell spans over the break
                let blocked = last_rows
                    .as_ref()
                    .is_some_and(|rows| rows.end > 0 && self.tree.table_row_blocked(table, rows.end - 1));
                if !blocked {
                    self.push_break(before, kid.node, avoid);
                } else {
                    // no break: the boxes are in one piece
                    let _ = before;
                }
            }
            self.gap_item(*end, Anchor::Top(kid.node));
            if kid.kids.is_empty() {
                self.emit_whole(kid.node, owner);
            } else {
                self.emit_group(table, kid, avoid);
            }
            *end = Anchor::Bottom(kid.node);
            *previous = Some(kid.node);
            last_rows = Some(kid.rows.clone());
            kids.push(kid.node);
        }
    }

    /// A box of a table that is not split: a caption or a row.
    fn emit_whole(&mut self, id: NodeId, parent: NodeId) {
        self.flow.parents[usize::from(id)] = Some(parent);
        let start = self.flow.items.len() as u32;
        let item = self.box_item(Anchor::Top(id), Anchor::Bottom(id));
        self.flow.leaves[item as usize] = Some(id);
        self.tree.node_mut(id).span = Some(Span {
            start,
            end: start + 1,
            kids: vec![],
            split: false,
            anchored: false,
        });
    }

    /// A row group: its rows are split like the boxes of a table.
    fn emit_group(&mut self, table: NodeId, group: &super::table::TableKid, avoid: Avoid) {
        let id = group.node;
        self.flow.parents[usize::from(id)] = Some(table);
        let start = self.flow.items.len() as u32;
        self.box_item(Anchor::Top(id), Anchor::ContentTop(id));
        let mut end = Anchor::ContentTop(id);
        let mut previous = None;
        let mut kids = vec![];
        self.emit_table_kids(table, id, &group.kids, avoid, &mut end, &mut previous, &mut kids);
        self.gap_item(end, Anchor::ContentBottom(id));
        self.box_item(Anchor::ContentBottom(id), Anchor::Bottom(id));
        let end_item = self.flow.items.len() as u32;
        self.tree.node_mut(id).span = Some(Span {
            start,
            end: end_item,
            kids,
            split: false,
            anchored: false,
        });
    }
}
