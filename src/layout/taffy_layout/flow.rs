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
//! A multi-column container is one item of the flow of its page, `Item::Container`: the kernel asks
//! the layout service for the fragment of it that fits on a page, and the page can end inside it.
//! (Where it has a height of its own or is inside another container, it is one box.) Its content is a flow of its own ([`Inner`]), which the
//! columns are chosen from. The items of both are numbered from 0 (the spans of the boxes in the
//! content of the container are in the numbers of its flow), the extents and the paragraphs have
//! one numbering for all of them.

use super::tree::{Content, UmbruchTree, node_is_out_of_flow};
use super::window::Span;
use crate::style::properties::computed::Break;
use crate::style::properties::specified::Display;
use crate::style::properties::computed::{Float, FloatReference};
use umbruch_paginate::{BoxRef, BreakKind, BreakLevel, FloatSide, Item, NodeRef, ParId};
use std::rc::Rc;
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
    /// The floats of the page that are called for in the content (`float-reference: page`): the
    /// boxes, where they are placed, and the item that each is called for in front of.
    pub hoisted: Vec<(NodeId, FloatSide, u32)>,
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
    /// The header rows of the tables, which the pages that go on inside of a table repeat.
    pub repeats: Vec<umbruch_paginate::Repeat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A box that is not split.
    Whole,
    /// A multi-column container, which is split between its columns and over pages: one item of
    /// the flow, the fragments of which the layout service finds.
    Container,
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
fn pin_to_first_page(tree: &mut UmbruchTree, root: NodeId) {
    fn collect(tree: &UmbruchTree, id: NodeId, positioned: bool, found: &mut Vec<(NodeId, NodeId)>) {
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

/// `single_page`: the document is one page whatever it says (a document that is cropped), so
/// there are no forced page breaks.
pub(super) fn extract(
    tree: &mut UmbruchTree,
    root: NodeId,
    rtl: bool,
    paged: bool,
    single_page: bool,
    page_size: (f32, f32),
) -> Flow {
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
            repeats: vec![],
        },
        level: BreakLevel::Page,
        paged,
        single_page,
        page_size,
        hoist: false,
        hoisted: vec![],
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
    tree: &'t mut UmbruchTree<'a, 'w>,
    flow: Flow,
    /// What the pages are that the flow is made for: the pages of the document, or the columns
    /// of a multi-column container.
    level: BreakLevel,
    /// Whether the document has pages that have a size. A container with columns is split over
    /// pages only then.
    paged: bool,
    /// Whether the document is one page, in which no page break is forced.
    single_page: bool,
    /// The width and the height of the content area of the first page: what decides which table
    /// rows may be taller than a page.
    page_size: (f32, f32),
    /// Whether the floats of the content of the container that is made into a flow are placed on
    /// the page: the container is split over pages, so the page that it is on is known.
    hoist: bool,
    /// The floats of the page that the container has so far.
    hoisted: Vec<(NodeId, FloatSide, u32)>,
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

    /// The children of a box that are part of the text: the floats are not (see `float_side`).
    fn in_flow_children(&self, id: NodeId) -> Vec<NodeId> {
        self.tree
            .node(id)
            .children
            .iter()
            .copied()
            .filter(|&child| self.in_flow(child) && self.float_side(child).is_none())
            .collect()
    }

    /// Where a box is placed on its page if it is a float: `float: top` or `float: bottom` takes a
    /// box that is between the blocks of a block container out of the text, and the paginator
    /// places it on a page like TeX does. A float is not one where the document has no pages or
    /// has one page that has the size of the content. There it is a block like the ones around
    /// it. In the content of a container with columns the float is placed in a column.
    fn float_side(&self, id: NodeId) -> Option<FloatSide> {
        // (the columns of a container are columns whether the document has pages or not)
        let floating = match self.level {
            BreakLevel::Page => self.paged && !self.single_page,
            BreakLevel::Column => true,
        };
        if !floating || !self.in_flow(id) {
            return None;
        }
        let node = self.tree.node(id);
        if node.orthogonal || node.flow.is_vertical() {
            return None;
        }
        match self.tree.computed_values.get(node.entity)?.page.float {
            Float::None => None,
            Float::Top => Some(FloatSide::Top),
            Float::Bottom => Some(FloatSide::Bottom),
        }
    }

    /// Whether a float that is in the content of a container with columns belongs to the page
    /// instead of its column (`float-reference: page`), if the page is known.
    fn floats_on_page(&self, id: NodeId) -> bool {
        self.hoist
            && self.level == BreakLevel::Column
            && self
                .tree
                .computed_values
                .get(self.tree.node(id).entity)
                .is_some_and(|style| style.page.float_reference == FloatReference::Page)
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
        // A container with columns is one item in the flow that it is in: its content is a flow of
        // its own, which `emit_columns` makes. It is split over pages if it is in the flow of a
        // page and its height is the one of its content. Inside of another container it is one box.
        if node.columns.is_some() {
            let min_height = self.tree.node(id).style.min_size.height;
            let no_min_height = min_height.is_auto() || {
                let raw = min_height.into_raw();
                raw.tag() == taffy::CompactLength::LENGTH_TAG && raw.value() == 0.0
            };
            return if self.paged && self.level == BreakLevel::Page && no_min_height { Kind::Container } else { Kind::Whole };
        }
        // A grid or flex container is split over pages like a table row is: its items are cut between the
        // rows of the grid, and inside of the ones that are taller than half a page.
        if self.grid_may_split(id) {
            return Kind::Container;
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

    /// Whether a grid container is split over pages: it is in the flow of a page and its height is
    /// the one of its content.
    fn grid_may_split(&self, id: NodeId) -> bool {
        if !self.paged || self.level != BreakLevel::Page || self.single_page || id == self.flow.root {
            return false;
        }
        let node = self.tree.node(id);
        if node.content != Content::Blocks || !matches!(self.display(id), Some(Display::Grid | Display::Flex)) {
            return false;
        }
        let min_height = node.style.min_size.height;
        let no_min_height = min_height.is_auto() || {
            let raw = min_height.into_raw();
            raw.tag() == taffy::CompactLength::LENGTH_TAG && raw.value() == 0.0
        };
        no_min_height && !self.in_flow_children(id).is_empty()
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
        // A change of the page name forces a page break. (Page names do not change the pages of a
        // container with columns: the page that continues it is not one of another name.)
        if self.level == BreakLevel::Page {
            if let (Some(a), Some(b)) = (self.page_name(before), self.page_name(after)) {
                breaks.forced_page |= a != b;
            }
        }
        if self.single_page {
            // (also in a container with columns, where it would end the fragment on its page)
            breaks.forced_page = false;
            breaks.side = None;
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
            // (a forced page break in a container with columns ends the fragment of it on its page:
            // it is a break to the side that it asks for)
            let side = if self.level == BreakLevel::Column { breaks.side } else { None };
            (forced(side), there)
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
            Kind::Container => {
                let item = self.push(Item::Container {
                    node: BoxRef(usize::from(id) as u32),
                    avoid_inside: avoid.page,
                });
                self.flow.leaves[item as usize] = Some(id);
                if let Some(columns) = self.tree.node_mut(id).columns.as_mut() {
                    columns.splittable = true;
                }
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
        // the header rows of a table are repeated where a page goes on inside of the table
        if kind == Kind::Table && self.level == BreakLevel::Page {
            if let Some(header) = self.tree.repeated_header(id) {
                if let Some(header_end) = self.tree.node(header).span.as_ref().map(|span| span.end) {
                    let node = self.extent(Anchor::Top(header), Anchor::Bottom(header));
                    self.flow.repeats.push(umbruch_paginate::Repeat {
                        from: header_end as usize,
                        to: end as usize,
                        node,
                        footer: false,
                    });
                }
            }
            // the footer rows of a table are repeated behind the last row of the pages in front
            // of the one that has them
            if let Some(footer) = self.tree.repeated_footer(id) {
                if let Some(footer_start) = self.tree.node(footer).span.as_ref().map(|span| span.start) {
                    let node = self.extent(Anchor::Top(footer), Anchor::Bottom(footer));
                    self.flow.repeats.push(umbruch_paginate::Repeat {
                        from: start as usize,
                        to: footer_start as usize,
                        node,
                        footer: true,
                    });
                }
            }
        }
    }

    /// The children of a block container: the breaks and gaps between them, and their items.
    /// `end` is where the box in front of the first one ends, and where the last one does.
    fn emit_blocks(&mut self, id: NodeId, avoid: Avoid, end: &mut Anchor, kids: &mut Vec<NodeId>) {
        let mut previous: Option<NodeId> = None;
        for &child in &self.tree.node(id).children.clone() {
            if let Some(side) = self.float_side(child) {
                // A float is called for here: the item says where, and the box is not part of the
                // box tree of the text any more (the paginator places it on a page, which is
                // laid out on its own).
                self.tree.node_mut(id).children.retain(|&other| other != child);
                if self.floats_on_page(child) {
                    // called for here, placed by the page: the item is where the container's
                    // fragment calls for it
                    self.hoisted.push((child, side, self.flow.items.len() as u32));
                } else {
                    self.push(Item::Float { node: BoxRef(usize::from(child) as u32), side });
                }
                continue;
            }
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
        self.hoist = self.tree.node(id).columns.as_ref().is_some_and(|columns| columns.splittable);
        self.hoisted.clear();
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
        self.hoist = false;
        let hoisted = std::mem::take(&mut self.hoisted);
        let items = std::mem::replace(&mut self.flow.items, items);
        let leaves = std::mem::replace(&mut self.flow.leaves, leaves);
        let span = Span { start: 0, end: items.len() as u32, kids, split, anchored: false };
        if let Some(columns) = self.tree.node_mut(id).columns.as_mut() {
            columns.inner = Some(Rc::new(Inner { items, leaves, span, hoisted }));
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
                if self.row_may_split(table, kid, avoid) {
                    self.emit_row(kid.node, owner, avoid);
                } else {
                    self.emit_whole(kid.node, owner);
                }
            } else {
                self.emit_group(table, kid, avoid);
            }
            *end = Anchor::Bottom(kid.node);
            *previous = Some(kid.node);
            last_rows = Some(kid.rows.clone());
            kids.push(kid.node);
        }
    }

    /// Whether a row of a table may be split over pages: it is in the flow of the page, no cell
    /// spans over a break in front of it or behind it, and it may be taller than a page (the
    /// estimate is made from the text in its cells: it is not taller than a page, if the row is a
    /// container, the fragment that the layout service finds is the whole row).
    fn row_may_split(&self, table: NodeId, kid: &super::table::TableKid, _avoid: Avoid) -> bool {
        if !self.paged || self.level != BreakLevel::Page || self.single_page {
            return false;
        }
        if self.display(kid.node) != Some(Display::TableRow) || kid.rows.is_empty() {
            return false;
        }
        let spans = |row: usize| self.tree.table_row_blocked(table, row);
        if kid.rows.start > 0 && spans(kid.rows.start - 1) || spans(kid.rows.end - 1) {
            return false;
        }
        let node = self.tree.node(kid.node);
        if node.orthogonal || node.flow.is_vertical() || !node.style.size.height.is_auto() {
            return false;
        }
        let cells = node.children.len().max(1) as f32;
        let (width, height) = self.page_size;
        // (the cells share the width: a lower estimate of what the row needs)
        let cell_width = (width / cells).max(1.0);
        node.children.iter().any(|&cell| self.estimated_height(cell, cell_width) >= height * 0.25)
    }

    /// An estimate of the height of the text in a box that is `width` wide.
    fn estimated_height(&self, id: NodeId, width: f32) -> f32 {
        let node = self.tree.node(id);
        match node.content {
            Content::Inline => {
                let Some(ifc) = self.tree.inline_ctxs.get(node.entity) else {
                    return 0.0;
                };
                let advance = ifc.break_points.last().map_or(0.0, |point| point.advance_before.0);
                let font_size = self
                    .tree
                    .computed_values
                    .get(node.entity)
                    .map_or(16.0, |style| style.font.font_size.0);
                (advance / width.max(1.0)).ceil().max(1.0) * font_size * 1.2
            }
            Content::Blocks => node.children.iter().map(|&child| self.estimated_height(child, width)).sum(),
            _ => 0.0,
        }
    }

    /// A row that may be split over pages: a container item, which the layout service finds the
    /// fragments of.
    fn emit_row(&mut self, id: NodeId, parent: NodeId, avoid: Avoid) {
        self.flow.parents[usize::from(id)] = Some(parent);
        let start = self.flow.items.len() as u32;
        let inside = self
            .tree
            .computed_values
            .get(self.tree.node(id).entity)
            .map(|style| style.page.break_inside);
        let item = self.push(Item::Container {
            node: BoxRef(usize::from(id) as u32),
            avoid_inside: avoid.page || inside.is_some_and(|inside| inside.avoids_page()),
        });
        self.flow.leaves[item as usize] = Some(id);
        self.tree.node_mut(id).span = Some(Span {
            start,
            end: start + 1,
            kids: vec![],
            split: false,
            anchored: false,
        });
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
