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

use super::tree::{Content, HeliumTree, node_is_out_of_flow};
use super::window::Span;
use crate::style::properties::computed::Break;
use crate::style::properties::specified::Display;
use helium_paginate::{BreakKind, Item, NodeRef, ParId};
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
    forced: bool,
    avoid: bool,
    /// The side that a forced break asks for.
    side: Option<Side>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

impl Breaks {
    fn add(&mut self, value: Break, rtl: bool) {
        self.forced |= value.is_forced();
        self.avoid |= value.is_avoid();
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
        self.forced |= other.forced;
        self.avoid |= other.avoid;
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
        },
    };
    extractor.emit(root, None, false);
    extractor.flow
}

struct Extractor<'t, 'a, 'w> {
    tree: &'t mut HeliumTree<'a, 'w>,
    flow: Flow,
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
        match node.content {
            Content::Replaced => Kind::Whole,
            Content::Inline => {
                let lines = self.tree.inline_ctxs.get(node.entity).is_some_and(|ifc| ifc.break_points.len() >= 2);
                if lines { Kind::Paragraph } else { Kind::Whole }
            }
            Content::Table => {
                if self.tree.table_kids(id).is_some() {
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

    /// The kind of the break between two boxes that follow each other.
    fn break_kind(&self, before: NodeId, after: NodeId, avoid_inside: bool) -> BreakKind {
        let mut breaks = self.breaks_of(before, true);
        breaks.merge(self.breaks_of(after, false));
        // a change of the page name forces a page break
        if let (Some(a), Some(b)) = (self.page_name(before), self.page_name(after)) {
            breaks.forced |= a != b;
        }
        if breaks.forced {
            match breaks.side {
                Some(Side::Left) => BreakKind::ForcedLeft,
                Some(Side::Right) => BreakKind::ForcedRight,
                None => BreakKind::Forced,
            }
        } else if breaks.avoid || avoid_inside {
            BreakKind::Avoid
        } else {
            BreakKind::Allowed
        }
    }

    /// Emits the items of a box and sets its span.
    fn emit(&mut self, id: NodeId, parent: Option<NodeId>, avoid_inside: bool) {
        self.flow.parents[usize::from(id)] = parent;
        let start = self.flow.items.len() as u32;
        let kind = self.classify(id);
        let avoid_inside = avoid_inside
            || self
                .tree
                .computed_values
                .get(self.tree.node(id).entity)
                .is_some_and(|style| style.page.break_inside.is_avoid());
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
                    avoid_inside,
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
                            let kind = self.break_kind(before, child, avoid_inside);
                            self.push(Item::Break { kind });
                        }
                        self.gap_item(end, Anchor::Top(child));
                        self.emit(child, Some(id), avoid_inside);
                        end = Anchor::Bottom(child);
                        previous = Some(child);
                        kids.push(child);
                    }
                } else {
                    let table_kids = self.tree.table_kids(id).unwrap_or_default();
                    self.emit_table_kids(id, id, &table_kids, avoid_inside, &mut end, &mut previous, &mut kids);
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

    /// The boxes of a table. Breaks are only possible where no cell spans over them.
    fn emit_table_kids(
        &mut self,
        table: NodeId,
        owner: NodeId,
        boxes: &[super::table::TableKid],
        avoid_inside: bool,
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
                    let kind = self.break_kind(before, kid.node, avoid_inside);
                    self.push(Item::Break { kind });
                } else {
                    // no break: the boxes are in one piece
                    let _ = before;
                }
            }
            self.gap_item(*end, Anchor::Top(kid.node));
            if kid.kids.is_empty() {
                self.emit_whole(kid.node, owner);
            } else {
                self.emit_group(table, kid, avoid_inside);
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
    fn emit_group(&mut self, table: NodeId, group: &super::table::TableKid, avoid_inside: bool) {
        let id = group.node;
        self.flow.parents[usize::from(id)] = Some(table);
        let start = self.flow.items.len() as u32;
        self.box_item(Anchor::Top(id), Anchor::ContentTop(id));
        let mut end = Anchor::ContentTop(id);
        let mut previous = None;
        let mut kids = vec![];
        self.emit_table_kids(table, id, &group.kids, avoid_inside, &mut end, &mut previous, &mut kids);
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
