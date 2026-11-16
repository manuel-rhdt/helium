//! The layout service: answers the questions of the pagination kernel with Taffy.
//!
//! For a width of a page it lays out the whole box tree once, as a galley, and reads the extents
//! of the boxes and gaps of the flow from it (see [`super::flow`]). The lines of a paragraph are
//! what the line breaker makes of the paragraph for the width that the galley gave it, and of
//! the rest of it for the width of a page that continues it. Both are cached by the width that
//! was asked for. There are no width classes.
//!
//! A box that is laid out for a width of its own (a column, a table on a page of another width) is
//! asked for as a subtree ([`super::subtree`]): the box is laid out alone with a definite width
//! of its content box, and the extents and the lines inside it are read from that layout. The
//! answers are cached by the box and the width.
//!
//! The pages themselves are not laid out from the galleys. Once the pages are chosen, every page
//! is laid out for its own width and height ([`super::window`]).

use super::flow::{Anchor, Flow};
use super::tree::{HeliumTree, lines_from};
use crate::layout::inline::InlineFormattingContext;
use crate::style::properties::computed::PixelLength;
use helium_paginate::{BoxRef, Cut, Item, LayoutService, NodeRef, ParId, ParLine};
use std::collections::HashMap;
use std::rc::Rc;
use taffy::geometry::Size;
use taffy::style::AvailableSpace;
use taffy::tree::NodeId;

/// The document laid out for one width of a page.
struct Galley {
    /// The extents of the boxes and gaps of the flow.
    extents: Vec<f64>,
    /// The width and the indent that the lines of every paragraph were broken for.
    lines: HashMap<NodeId, (f32, f32)>,
}

/// A box laid out for one width of its content box.
struct Subtree {
    /// The block size of the content box of the root.
    content_extent: f64,
    /// The extents of the boxes and gaps of the flow inside the box.
    extents: HashMap<u32, f64>,
    /// The width and the indent that the lines of the paragraphs inside the box were broken for.
    lines: HashMap<NodeId, (f32, f32)>,
    /// The inline contexts of the paragraphs with `inline-block` boxes inside the box, as the
    /// boxes were sized for this width.
    inline_contexts: HashMap<NodeId, InlineFormattingContext>,
}

pub(super) struct TreeService<'t, 'a, 'w> {
    pub tree: &'t mut HeliumTree<'a, 'w>,
    pub flow: &'t Flow,
    /// The height of the initial containing block of a galley.
    pub icb_height: f32,
    galleys: HashMap<u32, Rc<Galley>>,
    lines: HashMap<(u32, Option<u32>, u32), Rc<[ParLine]>>,
    /// By the root and the content width that were asked for.
    subtrees: HashMap<(u32, u32), Rc<Subtree>>,
    subtree_lines: HashMap<(u32, u32, Option<u32>, u32), Rc<[ParLine]>>,
}

impl<'t, 'a, 'w> TreeService<'t, 'a, 'w> {
    pub fn new(tree: &'t mut HeliumTree<'a, 'w>, flow: &'t Flow, icb_height: f32) -> Self {
        TreeService {
            tree,
            flow,
            icb_height,
            galleys: HashMap::new(),
            lines: HashMap::new(),
            subtrees: HashMap::new(),
            subtree_lines: HashMap::new(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// The box of an entity, if it can be asked about as a subtree: a box of the flow.
    pub fn box_of(&self, entity: specs::Entity) -> Option<BoxRef> {
        let found = self.tree.nodes.iter().position(|node| node.entity == entity && node.span.is_some());
        found.map(|index| BoxRef(index as u32))
    }

    /// The number of subtrees that were laid out.
    #[allow(dead_code)]
    pub fn subtrees(&self) -> usize {
        self.subtrees.len()
    }

    /// The number of widths that the document was laid out for.
    pub fn galleys(&self) -> usize {
        self.galleys.len()
    }

    /// The box `root` laid out with its content box `inline` wide.
    fn subtree(&mut self, root: BoxRef, inline: f32) -> Rc<Subtree> {
        let key = (root.0, inline.to_bits());
        if let Some(subtree) = self.subtrees.get(&key) {
            return subtree.clone();
        }
        let root_id = NodeId::from(root.0 as usize);
        let inline_contexts = self.tree.layout_subtree(root_id, inline, self.icb_height, &self.flow.parents);
        let tree = &*self.tree;
        let mut tops = vec![f64::NAN; tree.nodes.len()];
        place(tree, root_id, 0.0, true, &mut tops);
        let content_extent = y(tree, &tops, Anchor::ContentBottom(root_id))
            - y(tree, &tops, Anchor::ContentTop(root_id));
        // the items of the box: its own, or the ones of the content of a container with columns
        let items: &[Item] = match tree.node(root_id).columns.as_ref().and_then(|columns| columns.inner.as_ref()) {
            Some(inner) => &inner.items,
            None => {
                let span = tree.node(root_id).span.as_ref().expect("the root is part of the flow");
                &self.flow.items[span.start as usize..span.end as usize]
            }
        };
        let mut extents = HashMap::new();
        let mut lines = HashMap::new();
        for item in items {
            match *item {
                Item::Box { node } | Item::Gap { node } => {
                    let (from, to) = self.flow.extents[node.0 as usize];
                    let extent = y(tree, &tops, to) - y(tree, &tops, from);
                    // (a gap at the edge of the box may be made from boxes around it)
                    if extent.is_finite() {
                        extents.insert(node.0, extent);
                    }
                }
                Item::Paragraph { id, .. } => {
                    let node = self.flow.paragraphs[id.0 as usize];
                    if tops[usize::from(node)].is_nan() {
                        continue;
                    }
                    lines.insert(node, paragraph_width(tree, node));
                }
                Item::Break { .. } => {}
            }
        }
        let subtree = Rc::new(Subtree {
            content_extent,
            extents,
            lines,
            inline_contexts,
        });
        self.subtrees.insert(key, subtree.clone());
        subtree
    }

    /// The lines of a paragraph for a line width and an indent. A paragraph with `inline-block`
    /// boxes may be given the inline context that its boxes were sized for.
    fn paragraph_lines(
        &mut self,
        node: NodeId,
        from: Option<Cut>,
        (line_width, indent): (f32, f32),
        inline_context: Option<&InlineFormattingContext>,
    ) -> Rc<[ParLine]> {
        let tree = &mut *self.tree;
        let entity = tree.node(node).entity;
        let ifc = inline_context.or_else(|| tree.inline_ctxs.get(entity));
        let Some(ifc) = ifc else {
            return Rc::from(vec![]);
        };
        // (the cache of the box tree does not know which boxes the context was sized for)
        let mut own_cache = HashMap::new();
        let cache = if inline_context.is_some() { &mut own_cache } else { &mut tree.lines };
        lines_from(
            cache,
            ifc,
            entity.id(),
            from.map(|cut| cut.0 as usize),
            PixelLength(line_width),
            PixelLength(indent),
            tree.text_runs,
            tree.intrinsic_sizes,
            tree.aligns,
        )
        .iter()
        .map(|line| ParLine {
            extent: f64::from(line.height.0),
            end: Cut(line.to_break as u32),
        })
        .collect()
    }

    fn galley(&mut self, inline: f32) -> Rc<Galley> {
        let key = inline.to_bits();
        if let Some(galley) = self.galleys.get(&key) {
            return galley.clone();
        }
        let tree = &mut *self.tree;
        tree.clear_window();
        let root = self.flow.root;
        taffy::compute_root_layout(
            tree,
            root,
            Size {
                width: AvailableSpace::Definite(inline),
                height: AvailableSpace::Definite(self.icb_height),
            },
        );
        let mut tops = vec![0.0f64; tree.nodes.len()];
        place(tree, root, 0.0, true, &mut tops);
        let extents = self
            .flow
            .extents
            .iter()
            .take(self.flow.page_extents)
            .map(|&(from, to)| y(tree, &tops, to) - y(tree, &tops, from))
            .collect();
        let lines = self
            .flow
            .paragraphs
            .iter()
            .filter_map(|&node| Some((node, tree.node(node).measured?)))
            .collect();
        let galley = Rc::new(Galley { extents, lines });
        self.galleys.insert(key, galley.clone());
        galley
    }
}

/// The width and the indent that the lines of a paragraph were broken for in the layout that the
/// tree has: the ones that the measure function was given, or else the width of its content box.
fn paragraph_width(tree: &HeliumTree, node: NodeId) -> (f32, f32) {
    tree.node(node).measured.unwrap_or_else(|| {
        let layout = &tree.node(node).layout;
        let edges = layout.border.horizontal_axis_sum() + layout.padding.horizontal_axis_sum();
        ((layout.size.width - edges).max(0.0), 0.0)
    })
}

/// The top of every box of the flow in the document, as the galley is laid out.
fn place(tree: &HeliumTree, id: NodeId, parent: f64, is_root: bool, tops: &mut Vec<f64>) {
    let node = tree.node(id);
    // (the root is where the content area of the page starts)
    let top = if is_root { 0.0 } else { parent + f64::from(node.layout.location.y) };
    tops[usize::from(id)] = top;
    // the boxes in a container with columns are in its own flow, if it is the root
    let inner = node.columns.as_ref().filter(|_| is_root).and_then(|columns| columns.inner.as_ref());
    if let Some(span) = inner.map(|inner| &inner.span).or(node.span.as_ref()) {
        for &kid in &span.kids {
            place(tree, kid, top, false, tops);
        }
    }
}

fn y(tree: &HeliumTree, tops: &[f64], anchor: Anchor) -> f64 {
    let top = |id: NodeId| tops[usize::from(id)];
    let height = |id: NodeId| f64::from(tree.node(id).layout.size.height);
    let is_table = |id: NodeId| tree.node(id).content == super::tree::Content::Table;
    match anchor {
        Anchor::Top(id) => top(id),
        Anchor::Bottom(id) => top(id) + height(id),
        Anchor::ContentTop(id) if is_table(id) => {
            let first = tree.node(id).span.as_ref().and_then(|span| span.kids.first()).copied();
            first.map_or(top(id), top)
        }
        Anchor::ContentBottom(id) if is_table(id) => {
            let last = tree.node(id).span.as_ref().and_then(|span| span.kids.last()).copied();
            last.map_or(top(id) + height(id), |last| top(last) + height(last))
        }
        Anchor::ContentTop(id) => {
            let layout = &tree.node(id).layout;
            top(id) + f64::from(layout.border.top + layout.padding.top)
        }
        Anchor::ContentBottom(id) => {
            let layout = &tree.node(id).layout;
            top(id) + height(id) - f64::from(layout.border.bottom + layout.padding.bottom)
        }
    }
}

impl LayoutService for TreeService<'_, '_, '_> {
    fn extent(&mut self, node: NodeRef, page_inline: f32) -> f64 {
        self.galley(page_inline).extents[node.0 as usize]
    }

    fn lines(&mut self, par: ParId, from: Option<Cut>, page_inline: f32) -> Rc<[ParLine]> {
        let key = (par.0, from.map(|cut| cut.0), page_inline.to_bits());
        if let Some(lines) = self.lines.get(&key) {
            return lines.clone();
        }
        let galley = self.galley(page_inline);
        let node = self.flow.paragraphs[par.0 as usize];
        let width = galley.lines.get(&node).copied().unwrap_or_else(|| paragraph_width(self.tree, node));
        let lines = self.paragraph_lines(node, from, width, None);
        self.lines.insert(key, lines.clone());
        lines
    }

    fn content_extent(&mut self, root: BoxRef, inline: f32) -> f64 {
        self.subtree(root, inline).content_extent
    }

    fn extent_in(&mut self, root: BoxRef, node: NodeRef, inline: f32) -> f64 {
        let subtree = self.subtree(root, inline);
        *subtree.extents.get(&node.0).expect("the box or the gap is inside the subtree")
    }

    fn lines_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]> {
        let key = (root.0, par.0, from.map(|cut| cut.0), inline.to_bits());
        if let Some(lines) = self.subtree_lines.get(&key) {
            return lines.clone();
        }
        let subtree = self.subtree(root, inline);
        let node = self.flow.paragraphs[par.0 as usize];
        let width = *subtree.lines.get(&node).expect("the paragraph is inside the subtree");
        let lines = self.paragraph_lines(node, from, width, subtree.inline_contexts.get(&node));
        self.subtree_lines.insert(key, lines.clone());
        lines
    }
}
