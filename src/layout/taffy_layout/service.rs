//! The layout service: answers the questions of the pagination kernel with Taffy.
//!
//! For a width of a page it lays out the whole box tree once, as a galley, and reads the extents
//! of the boxes and gaps of the flow from it (see [`super::flow`]). The lines of a paragraph are
//! what the line breaker makes of the paragraph for the width that the galley gave it, and of
//! the rest of it for the width of a page that continues it. Both are cached by the width that
//! was asked for. There are no width classes.
//!
//! The pages themselves are not laid out from the galleys. Once the pages are chosen, every page
//! is laid out for its own width and height ([`super::window`]).

use super::flow::{Anchor, Flow};
use super::tree::{HeliumTree, lines_from};
use crate::style::properties::computed::PixelLength;
use helium_paginate::{Cut, LayoutService, NodeRef, ParId, ParLine};
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

pub(super) struct TreeService<'t, 'a, 'w> {
    pub tree: &'t mut HeliumTree<'a, 'w>,
    pub flow: &'t Flow,
    /// The height of the initial containing block of a galley.
    pub icb_height: f32,
    galleys: HashMap<u32, Rc<Galley>>,
    lines: HashMap<(u32, Option<u32>, u32), Rc<[ParLine]>>,
}

impl<'t, 'a, 'w> TreeService<'t, 'a, 'w> {
    pub fn new(tree: &'t mut HeliumTree<'a, 'w>, flow: &'t Flow, icb_height: f32) -> Self {
        TreeService {
            tree,
            flow,
            icb_height,
            galleys: HashMap::new(),
            lines: HashMap::new(),
        }
    }

    /// The number of widths that the document was laid out for.
    pub fn galleys(&self) -> usize {
        self.galleys.len()
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

/// The top of every box of the flow in the document, as the galley is laid out.
fn place(tree: &HeliumTree, id: NodeId, parent: f64, is_root: bool, tops: &mut Vec<f64>) {
    let node = tree.node(id);
    // (the root is where the content area of the page starts)
    let top = if is_root { 0.0 } else { parent + f64::from(node.layout.location.y) };
    tops[usize::from(id)] = top;
    if let Some(span) = &node.span {
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
        let (line_width, indent) = galley.lines.get(&node).copied().unwrap_or_else(|| {
            let layout = &self.tree.node(node).layout;
            let edges = layout.border.horizontal_axis_sum() + layout.padding.horizontal_axis_sum();
            ((layout.size.width - edges).max(0.0), 0.0)
        });
        let tree = &mut *self.tree;
        let entity = tree.node(node).entity;
        let lines: Rc<[ParLine]> = match tree.inline_ctxs.get(entity) {
            Some(ifc) => lines_from(
                &mut tree.lines,
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
            .collect(),
            None => Rc::from(vec![]),
        };
        self.lines.insert(key, lines.clone());
        lines
    }
}
