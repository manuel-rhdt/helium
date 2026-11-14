//! Page groups: the boxes of a document that are laid out for pages with other margins.
//!
//! The layout of a document is done once, for the content area of the plain `@page` rules, and
//! the paginator then cuts it into pages. A page that has other margins (a title page without
//! margins, `@page :first`, or a named page) therefore cannot get another width from the
//! paginator. Where the content of such a page is delimited by forced page breaks, it can be
//! laid out separately instead: the in-flow children of the box that flows over the pages (the
//! `body`, or the only box it contains) are cut into *page groups* at the forced breaks, and the
//! boxes of a group that needs another width are put into a box of that width.
//!
//! A group is laid out at the width of its first page, also if it takes more pages. A page of
//! a document that has only one kind of width needs no group, and the layout is not changed.
//!
//! The box around a group gets the difference to the margins of the layout as its margins: it
//! is moved to the left margin of the page of the group and is as wide as the page of the group
//! (a margin is negative where the page has smaller margins than the layout). The paginator then
//! does not shift the pages of the group any more.

use super::tree::{Content, HeliumTree, node_is_out_of_flow};
use crate::layout::components::PageGroupBox;
use crate::layout::pages::PageMargins;
use specs::{Entities, WriteStorage};
use std::sync::Arc;
use taffy::{Display, LengthPercentageAuto};
use taffy::prelude::NodeId;

/// The in-flow children of a box.
fn in_flow_children(tree: &HeliumTree, id: NodeId) -> Vec<NodeId> {
    tree.node(id)
        .children
        .iter()
        .copied()
        .filter(|&child| !node_is_out_of_flow(tree, child))
        .collect()
}

fn page_name(tree: &HeliumTree, id: NodeId) -> Option<Arc<str>> {
    tree.computed_values
        .get(tree.node(id).entity)
        .and_then(|style| style.page.page.0.clone())
}

/// Whether the box or its last box has a forced `break-after` (the value of the last child
/// propagates to its parent).
fn forced_break_after(tree: &HeliumTree, mut id: NodeId) -> bool {
    loop {
        if tree
            .computed_values
            .get(tree.node(id).entity)
            .is_some_and(|style| style.page.break_after.is_forced())
        {
            return true;
        }
        match in_flow_children(tree, id).last() {
            Some(&last) if tree.node(id).content == Content::Blocks => id = last,
            _ => return false,
        }
    }
}

/// Whether the box or its first box has a forced `break-before`.
fn forced_break_before(tree: &HeliumTree, mut id: NodeId) -> bool {
    loop {
        if tree
            .computed_values
            .get(tree.node(id).entity)
            .is_some_and(|style| style.page.break_before.is_forced())
        {
            return true;
        }
        match in_flow_children(tree, id).first() {
            Some(&first) if tree.node(id).content == Content::Blocks => id = first,
            _ => return false,
        }
    }
}

/// Puts the boxes of the page groups that need another width than the layout into boxes of that
/// width. `root` is the root of the tree.
pub(super) fn wrap_page_groups(
    tree: &mut HeliumTree,
    root: NodeId,
    margins: &PageMargins,
    entities: &Entities,
    group_boxes: &mut WriteStorage<PageGroupBox>,
) {
    let base_width = margins.base.left + margins.base.right;
    if margins.rtl || margins.pages.values().all(|edges| edges.left + edges.right == base_width) {
        return;
    }

    // the box that flows over the pages: the first box with more than one in-flow child
    let mut flow = root;
    loop {
        let children = in_flow_children(tree, flow);
        match children.as_slice() {
            [only] if tree.node(*only).content == Content::Blocks => flow = *only,
            _ => break,
        }
    }
    let children = in_flow_children(tree, flow);
    if children.is_empty() {
        return;
    }

    // cut the children into groups at the forced breaks, and at a change of the page name
    let mut groups: Vec<Vec<NodeId>> = vec![];
    for &child in &children {
        let starts_group = match groups.last().and_then(|group| group.last()) {
            None => true,
            Some(&previous) => {
                forced_break_after(tree, previous)
                    || forced_break_before(tree, child)
                    || page_name(tree, previous) != page_name(tree, child)
            }
        };
        if starts_group {
            groups.push(vec![child]);
        } else {
            groups.last_mut().unwrap().push(child);
        }
    }

    // (the first member of the group, the group, the box around it) for the groups that need
    // another width
    let mut wrapped: Vec<(NodeId, Vec<NodeId>, NodeId)> = vec![];
    let only_group = groups.len() == 1;
    for (number, group) in groups.into_iter().enumerate() {
        let name = page_name(tree, group[0]);
        let first_in_group = name.is_some();
        let edges = margins.edges_of_page_start(number == 0, name.clone(), first_in_group);
        let difference = base_width - (edges.left + edges.right);
        if difference.0.abs() < 0.01 && (edges.left - margins.base.left).0.abs() < 0.01 {
            continue;
        }
        if only_group && name.is_none() {
            // The whole document would be laid out for the margins of its first page.
            log::warn!(
                "the margins of `@page :first` change the width of the content only if the \
                 content of the first page is followed by a forced page break (the width of the \
                 first page is not used)"
            );
            continue;
        }
        let entity = entities.create();
        group_boxes.insert(entity, PageGroupBox { name }).unwrap();
        let id = tree.new_node(entity, Content::Blocks);
        let node = tree.node_mut(id);
        node.style.display = Display::Block;
        node.style.margin.left = LengthPercentageAuto::length((edges.left - margins.base.left).0);
        node.style.margin.right = LengthPercentageAuto::length((edges.right - margins.base.right).0);
        node.children = group.clone();
        wrapped.push((group[0], group, id));
    }
    if wrapped.is_empty() {
        return;
    }

    let mut new_children = vec![];
    for child in tree.node(flow).children.clone() {
        if let Some((_, _, id)) = wrapped.iter().find(|(first, ..)| *first == child) {
            new_children.push(*id);
        } else if wrapped.iter().any(|(_, group, _)| group.contains(&child)) {
            // moved into the box around its group
        } else {
            new_children.push(child);
        }
    }
    tree.node_mut(flow).children = new_children;
}
