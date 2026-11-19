//! A subtree laid out for a width of its own.
//!
//! The service lays the whole document out for the width of a page ([`super::service`]). A box
//! whose content area is not the width of a page (a column, a table that continues on a page of
//! another width) needs the layout of its subtree for the width that it has. This module lays out
//! one box of the tree as the root of a formatting context with a *definite content-box width*.
//! The box is laid out on its own: Taffy is called for the box itself, not for the root of the
//! document, and the inputs are the ones that a block container gives a child that is as wide as
//! the box (known inline size, the border box of the width that was asked for).
//!
//! What a layout at another width may change, and what is done about it:
//!
//! - **The cache of Taffy.** The key of an entry is the known size, the available space and the
//!   size of the parent. It does not say which children or which style the layout was computed
//!   with, and the layouts of the children of a box are not part of its entry. A layout at
//!   another width finds the entries of the nodes below the box that were made for the width of
//!   a page, so the cache of all the nodes of the subtree is cleared in front of it: the layout
//!   is computed from nothing, and the paragraphs have fresh `measured` widths. The nodes of the
//!   subtree then hold entries that agree with the layouts below them. The boxes *around* the
//!   subtree do not: their entries still say that their children were laid out as they were in
//!   a galley, and the layouts below the subtree are now the ones of this width. The cache of
//!   every ancestor is cleared behind the layout, so that a later layout of the page computes
//!   them again (the other children of an ancestor keep their entries and are found there).
//! - **The `inline-block` boxes of a paragraph** are sized once, for the line width of the first
//!   layout of the paragraph, and `apply_atom_widths` adds their widths to the inline context.
//!   Laid out for another width, they would change what every later layout of the paragraph
//!   sees. The inline context that a paragraph had before its boxes were applied is kept
//!   ([`super::tree::Node::atoms_before`]). For the layout of the subtree the paragraph gets a
//!   copy of it, so that its boxes are sized for this width, and the lines of the paragraph at
//!   this width are made from that copy (the result of `layout_subtree`). Behind the layout the
//!   inline context, the sizes of the boxes and the lines that were cached for the paragraph are
//!   the ones from before, and the cache of the subtree is cleared once more.
//!
//! What this does not do: it does not look at a window. A window is taken off the tree first, so
//! the subtree is the whole of the box. Out-of-flow boxes in the subtree whose containing block
//! is outside of it are not placed.

use super::tree::{HeliumTree, calc};
use crate::layout::inline::InlineFormattingContext;
use helium_paginate::FxHashMap as HashMap;
use std::rc::Rc;
use taffy::geometry::{Line, Size};
use taffy::style::AvailableSpace;
use taffy::tree::{Layout, LayoutInput, NodeId, RunMode, SizingMode};
use taffy::{LayoutPartialTree, RequestedAxis, ResolveOrZero};

/// What a paragraph with `inline-block` boxes had before a layout of a subtree.
struct Kept {
    id: NodeId,
    /// Whether the boxes had been applied.
    done: bool,
    ifc: InlineFormattingContext,
    before: Option<Box<(InlineFormattingContext, f32, Option<f32>)>>,
    /// The lines that were cached for the paragraph, by the entity.
    lines: Vec<((u32, Option<super::tree::LineCut>, u32), Rc<Vec<crate::layout::inline::MeasuredLine>>)>,
}

/// A float laid out on its own at the width of a page ([`HeliumTree::layout_float`]).
#[derive(Debug, Clone, Copy)]
pub(super) struct FloatBox {
    /// The margins of the float, with the percentages resolved against the width of the page.
    pub margin: taffy::geometry::Rect<f32>,
    /// The size of the border box.
    pub size: Size<f32>,
}

impl FloatBox {
    /// The extent of the margin box in the block direction, which is what the float takes on a
    /// page: the margins are not collapsed with anything.
    pub fn outer_height(&self) -> f32 {
        self.margin.top + self.size.height + self.margin.bottom
    }
}

impl HeliumTree<'_, '_> {
    /// Lays out a float, which is not part of the flow, as the root of a layout of its own on a
    /// page whose content area is `page_inline` wide: like a block that fills the width of the page
    /// (`width` and the margins apply as usual). The layout of the nodes of the float is the one
    /// for this width afterwards.
    ///
    /// The float is laid out from nothing, so the paragraphs in it have fresh `measured` widths
    /// where a page has another width than the one before. (The `inline-block` boxes in its lines
    /// are sized once, for the first width, like the ones of every other box.)
    pub(super) fn layout_float(&mut self, id: NodeId, page_inline: f32, icb_height: f32) -> FloatBox {
        self.clear_window();
        let nodes = self.nodes_below(id);
        self.clear_caches(&nodes);
        for &below in &nodes {
            self.node_mut(below).measured = None;
        }
        taffy::compute_root_layout(
            self,
            id,
            Size {
                width: AvailableSpace::Definite(page_inline),
                height: AvailableSpace::Definite(icb_height),
            },
        );
        let layout = &self.node(id).layout;
        FloatBox { margin: layout.margin, size: layout.size }
    }

    /// The nodes of the subtree of a box, the box included: its children and the `inline-block`
    /// boxes in its lines, recursively.
    fn nodes_below(&self, root: NodeId) -> Vec<NodeId> {
        let mut found = vec![];
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            found.push(id);
            let node = self.node(id);
            stack.extend(node.children.iter().copied());
            stack.extend(node.atoms.iter().copied());
        }
        found
    }

    fn clear_caches(&mut self, nodes: &[NodeId]) {
        for &id in nodes {
            self.node_mut(id).cache.clear();
        }
    }

    /// Lays out the subtree of `root` with its content box `inline` wide and no window.
    ///
    /// `root` must be part of the flow and not an orthogonal flow, and `parents` are the boxes
    /// that the boxes of the flow are in. The layout of the nodes of the subtree (`layout`,
    /// `measured`, and so on) is the one for this width afterwards, and so is the `layout` of the
    /// root, which has the size of its border box, its padding and its border but no position.
    ///
    /// Returns the inline contexts of the paragraphs with `inline-block` boxes in the subtree, as
    /// the boxes were sized for this width (see the module).
    pub(super) fn layout_subtree(
        &mut self,
        root: NodeId,
        inline: f32,
        icb_height: f32,
        parents: &[Option<NodeId>],
    ) -> HashMap<NodeId, InlineFormattingContext> {
        assert!(
            self.node(root).span.is_some() || self.node(root).columns.is_some(),
            "a subtree is the subtree of a box of the flow or of a container with columns"
        );
        assert!(!self.node(root).orthogonal, "a subtree is not an orthogonal flow");
        self.clear_window();

        let nodes = self.nodes_below(root);
        self.clear_caches(&nodes);
        for &id in &nodes {
            self.node_mut(id).measured = None;
        }

        // the paragraphs whose `inline-block` boxes are sized again for this width
        let mut kept: Vec<Kept> = vec![];
        for &id in &nodes {
            if self.node(id).atoms.is_empty() {
                continue;
            }
            let entity = self.node(id).entity;
            let Some(ifc) = self.inline_ctxs.get(entity).cloned() else {
                continue;
            };
            let done = self.node(id).atoms_done;
            let before = self.node_mut(id).atoms_before.take();
            if let Some(before) = &before {
                if done {
                    if let Some(slot) = self.inline_ctxs.get_mut(entity) {
                        *slot = before.0.clone();
                    }
                }
            }
            self.node_mut(id).atoms_done = false;
            let id_of_entity = entity.id();
            let lines = {
                let mut taken = vec![];
                let keys: Vec<_> = self.lines.keys().filter(|key| key.0 == id_of_entity).copied().collect();
                for key in keys {
                    if let Some(lines) = self.lines.remove(&key) {
                        taken.push((key, lines));
                    }
                }
                taken
            };
            kept.push(Kept { id, done, ifc, before, lines });
        }

        // the box is laid out as a block container as wide as the box that was asked for
        let style = self.active_style(root);
        let padding = style.padding.resolve_or_zero(Some(inline), calc);
        let border = style.border.resolve_or_zero(Some(inline), calc);
        let outer = inline + (padding + border).horizontal_axis_sum();
        let inputs = LayoutInput {
            known_dimensions: Size { width: Some(outer), height: None },
            known_dimensions_are_definite: Size { width: true, height: true },
            parent_size: Size { width: Some(inline), height: None },
            available_space: Size {
                width: AvailableSpace::Definite(outer),
                height: AvailableSpace::Definite(icb_height),
            },
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            run_mode: RunMode::PerformLayout,
            vertical_margins_are_collapsible: Line::FALSE,
        };
        // (a container with columns is laid out as a block)
        let plain = self.plain_root.replace(root);
        let output = self.compute_child_layout(root, inputs);
        self.plain_root = plain;
        self.node_mut(root).layout = Layout {
            size: output.size,
            padding,
            border,
            ..Layout::new()
        };

        // the paragraphs with `inline-block` boxes go back to what they were
        let mut inline_contexts = HashMap::default();
        if !kept.is_empty() {
            for Kept { id, .. } in &kept {
                let entity = self.node(*id).entity;
                if let Some(ifc) = self.inline_ctxs.get(entity) {
                    inline_contexts.insert(*id, ifc.clone());
                }
            }
            self.clear_caches(&nodes);
            for Kept { id, done, ifc, before, lines } in &mut kept {
                let id = *id;
                let entity = self.node(id).entity;
                let id_of_entity = entity.id();
                self.lines.retain(|key, _| key.0 != id_of_entity);
                self.lines.extend(std::mem::take(lines));
                if let Some(slot) = self.inline_ctxs.get_mut(entity) {
                    *slot = ifc.clone();
                }
                let node = self.node_mut(id);
                node.atoms_done = *done;
                node.atoms_before = before.take();
                node.measured = None;
            }
            // the boxes are laid out again for the line width they had before, the ones in the
            // boxes first
            for Kept { id, done, .. } in kept.iter().rev() {
                if !done {
                    continue;
                }
                if let Some((_, line_width, containing_height)) = self.node(*id).atoms_before.as_deref().cloned() {
                    for atom in self.node(*id).atoms.clone() {
                        self.layout_atom(atom, line_width, containing_height);
                    }
                }
            }
        }

        // the boxes around the subtree have layouts of the subtree that they do not know of
        let mut ancestor = parents[usize::from(root)];
        while let Some(id) = ancestor {
            self.node_mut(id).cache.clear();
            ancestor = parents[usize::from(id)];
        }
        inline_contexts
    }
}
