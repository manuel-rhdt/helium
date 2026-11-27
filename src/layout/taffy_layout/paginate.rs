//! The pages of the document: the geometry of the page that starts at every place of the flow,
//! the choice of the pages, and the windows that the pages are laid out from.
//!
//! The kernel and the solvers of `umbruch_paginate` do the choosing. This module tells them what
//! a page looks like at a place (its name and its kind decide the margins and the
//! size), and turns what they choose into a window for every
//! page (see [`super::window`]).

use super::flow::Flow;
use taffy::tree::NodeId;
use super::tree::UmbruchTree;
use super::window::{Leading, Window};
use crate::layout::components::EdgeValues;
use crate::layout::pages::{PageContext, PageMargins};
use crate::style::properties::computed::PixelLength;
use umbruch_paginate::{BreakLevel, Item, PageKind, PageSpec, PageTable, Position};
use std::collections::HashMap;
use std::sync::Arc;

/// What decides the geometry of a page, other than its kind: the name of the first content on
/// it, and the page groups that it starts (of the names that `:nth(1 of name)` selectors mention).
type Class = (Option<Arc<str>>, Vec<Arc<str>>);

/// A page group: the pages of an element with a `page` name and a forced break (CSS GCPM 3, 3.2).
#[derive(Debug, Clone)]
pub(super) struct GroupElement {
    pub name: Arc<str>,
    /// The items of the flow that the element is made of.
    pub start: usize,
    pub end: usize,
}

/// The elements that make page groups, for the names that the selectors of the style sheet use.
///
/// The `page` property is applied to the element: a name that an element has because it inherits
/// it (`auto` is the name of the parent) makes no group. (The computed value does not say it
/// was inherited, so an element inside another one of the same name makes no group either.)
pub(super) fn group_elements(tree: &UmbruchTree, flow: &Flow, keys: &crate::layout::pages::PageKeys) -> Vec<GroupElement> {
    use crate::style::properties::specified::Break;
    let forced = |value: Break| matches!(value, Break::Always | Break::Page | Break::Left | Break::Right | Break::Recto | Break::Verso);
    let mut elements = vec![];
    for index in 0..tree.nodes.len() {
        let node = tree.node(NodeId::from(index));
        let (Some(span), Some(style)) = (&node.span, tree.computed_values.get(node.entity)) else { continue };
        let Some(name) = style.page.page.0.clone() else { continue };
        if !keys.nth.contains_key(&Some(name.clone())) {
            continue;
        }
        let parent_name = flow.parents[index]
            .and_then(|parent| tree.computed_values.get(tree.node(parent).entity))
            .and_then(|style| style.page.page.0.clone());
        if parent_name.as_ref() == Some(&name) {
            continue;
        }
        if forced(style.page.break_before) || forced(style.page.break_after) {
            elements.push(GroupElement { name, start: span.start as usize, end: span.end as usize });
        }
    }
    elements
}

/// The item that a position is in or in front of.
pub(super) fn item_of(position: Position) -> usize {
    match position {
        Position::Item(item) | Position::InParagraph { item, .. } | Position::InContainer { item, .. } => item,
    }
}

/// For every page, its number in the page groups that it is part of, by the name of the group
/// (the innermost group if there are several of a name). `pages` are the index of every page and
/// the first and last position of it. A page is part of a group if it has an item of it.
pub(super) fn group_positions(
    elements: &[GroupElement],
    pages: &[(usize, Position, Position)],
) -> Vec<Vec<(Arc<str>, usize)>> {
    let mut first_page: Vec<Option<usize>> = vec![None; elements.len()];
    pages
        .iter()
        .map(|&(page, from, to)| {
            let start = item_of(from);
            let end = match to {
                Position::Item(item) => item,
                other => item_of(other) + 1,
            }
            .max(start + 1);
            let mut groups: Vec<(Arc<str>, usize, usize)> = vec![];
            for (number, element) in elements.iter().enumerate() {
                if start < element.end && end > element.start {
                    let first = *first_page[number].get_or_insert(page);
                    let position = page - first + 1;
                    // (the innermost is the one that starts last)
                    match groups.iter_mut().find(|(name, _, _)| *name == element.name) {
                        Some(known) if known.2 <= element.start => *known = (element.name.clone(), position, element.start),
                        Some(_) => {}
                        None => groups.push((element.name.clone(), position, element.start)),
                    }
                }
            }
            groups.sort_by(|a, b| a.0.cmp(&b.0));
            groups.into_iter().map(|(name, position, _)| (name, position)).collect()
        })
        .collect()
}

/// The class of the page that starts at every place of the flow where one can start.
pub(super) struct Classes {
    classes: Vec<Class>,
    /// By the item that the page starts in front of.
    at_item: Vec<usize>,
    /// By the paragraph or container item that the page starts in.
    in_paragraph: HashMap<usize, usize>,
}

impl Classes {
    pub fn new(tree: &UmbruchTree, flow: &Flow, groups: &[GroupElement]) -> Self {
        let len = flow.items.len();
        let name_of = |node| {
            tree.computed_values
                .get(tree.node(node).entity)
                .and_then(|style| style.page.page.0.clone())
        };
        // the first leaf at or behind an item and the last one in front of it
        let mut next_leaf: Vec<Option<usize>> = vec![None; len + 1];
        for item in (0..len).rev() {
            next_leaf[item] = if flow.leaves[item].is_some() { Some(item) } else { next_leaf[item + 1] };
        }
        let mut classes: Vec<Class> = vec![];
        let mut id_of = |class: Class| match classes.iter().position(|other| *other == class) {
            Some(id) => id,
            None => {
                classes.push(class);
                classes.len() - 1
            }
        };
        // The groups that a page starts that starts in front of an item: the groups of the
        // elements whose first content is the first content of the page. (The page can start in
        // front of the edges of boxes that are before the element, like the root's.)
        let mut starts: Vec<Vec<Arc<str>>> = vec![vec![]; len + 1];
        for element in groups {
            if next_leaf.get(element.start).copied().flatten().is_none() {
                continue;
            }
            let mut item = element.start;
            loop {
                if !starts[item].contains(&element.name) {
                    starts[item].push(element.name.clone());
                }
                if item == 0 || flow.leaves[item - 1].is_some() {
                    break;
                }
                item -= 1;
            }
        }
        for names in &mut starts {
            names.sort();
        }
        let mut at_item = Vec::with_capacity(len + 1);
        for item in 0..=len {
            let leaf = next_leaf[item].and_then(|leaf| flow.leaves[leaf].map(|node| (leaf, node)));
            let name = leaf.and_then(|(_, node)| name_of(node));
            at_item.push(id_of((name, starts[item].clone())));
        }
        let mut in_paragraph = HashMap::new();
        for (item, entry) in flow.items.iter().enumerate() {
            if let (Item::Paragraph { .. } | Item::Container { .. }, Some(node)) = (entry, flow.leaves[item]) {
                in_paragraph.insert(item, id_of((name_of(node), vec![])));
            }
        }
        Classes { classes, at_item, in_paragraph }
    }

    pub fn of(&self, position: Position) -> &Class {
        let id = match position {
            Position::Item(item) => self.at_item[item],
            Position::InParagraph { item, .. } | Position::InContainer { item, .. } => self.in_paragraph[&item],
        };
        &self.classes[id]
    }
}

/// The geometry of the pages of a document.
pub(super) struct Geometry<'m> {
    pub margins: &'m PageMargins,
    /// The size of the plain page and its margins.
    pub size: (PixelLength, PixelLength),
    pub margin: EdgeValues<PixelLength>,
}

impl Geometry<'_> {
    /// The context that decides the style of a page.
    pub fn context(&self, index: usize, class: &Class) -> PageContext {
        PageContext {
            index,
            name: class.0.clone(),
            rtl: self.margins.rtl,
            // a page that starts a group is the first page of it
            groups: class.1.iter().map(|name| (name.clone(), 1)).collect(),
        }
    }

    /// The margins of a page, and its width and height.
    pub fn of_page(&self, page: &PageContext) -> (EdgeValues<PixelLength>, (PixelLength, PixelLength)) {
        let edges = self.margins.pages.get(&crate::layout::pages::normalize(page, &self.margins.keys, crate::layout::pages::Scope::Geometry));
        (
            edges.copied().unwrap_or(self.margin),
            self.margins.size(page, self.size),
        )
    }

    /// The content area of a page.
    pub fn spec(&self, page: &PageContext) -> PageSpec {
        let (edges, size) = self.of_page(page);
        PageSpec {
            inline_size: (size.0 - edges.left - edges.right).0.max(0.0),
            block_size: (size.1 - edges.top - edges.bottom).0.max(0.0),
        }
    }

    /// The table of the geometry of the pages for the kernel.
    pub fn table(&self, classes: Arc<Classes>) -> PageTable {
        let rtl = self.margins.rtl;
        // the page index that stands for a kind of page
        let index_of = |kind: PageKind| match kind {
            PageKind::First => 0,
            PageKind::Left => if rtl { 2 } else { 1 },
            PageKind::Right => if rtl { 1 } else { 2 },
        };
        let kinds = [PageKind::First, PageKind::Left, PageKind::Right];
        let specs: Vec<[PageSpec; 3]> = classes
            .classes
            .iter()
            .map(|class| kinds.map(|kind| self.spec(&self.context(index_of(kind), class))))
            .collect();
        let differ = specs.iter().any(|specs| specs[1] != specs[2]);
        PageTable::new(rtl, move |position, kind| {
            let class = match position {
                Position::Item(item) => classes.at_item[item],
                Position::InParagraph { item, .. } | Position::InContainer { item, .. } => classes.in_paragraph[&item],
            };
            specs[class][kind as usize]
        })
        .left_right_differ(differ)
    }
}

/// The window of the page that starts at `start` and ends at `end`: in the items of the flow of
/// a document, or in the ones of the content of a multi-column container, which `level` and
/// `scope` tell (the pages are its columns).
pub(super) fn window(
    items: &[Item],
    leaves: &[Option<NodeId>],
    level: BreakLevel,
    scope: Option<NodeId>,
    start: Position,
    end: Position,
    leading_gap: impl FnOnce(usize) -> f32,
) -> Window {
    let len = items.len();
    let paragraph = |item: usize| leaves[item].expect("a paragraph has its box");
    let (first, start_cut, leading) = match start {
        Position::Item(0) => (0, None, Leading::Natural),
        Position::Item(item) => {
            // (a forced break of the page level is a column break, too)
            let forced = matches!(
                items[item - 1],
                Item::Break { kind, level: of }
                    if (level == of || (level == BreakLevel::Column && of == BreakLevel::Page)) && kind.is_forced()
            );
            let leading = match items.get(item) {
                Some(Item::Gap { .. }) if forced => Leading::Gap(leading_gap(item)),
                _ => Leading::Discard,
            };
            (item, None, leading)
        }
        Position::InParagraph { item, cut } => (item, Some((paragraph(item), cut)), Leading::Discard),
        Position::InContainer { item, .. } => (item, None, Leading::Discard),
    };
    let (last, end_cut, at_end) = match end {
        Position::Item(item) if item >= len => (len, None, true),
        // (the break is not part of the page)
        Position::Item(item) => (item - 1, None, false),
        Position::InParagraph { item, cut } => (item + 1, Some((paragraph(item), cut)), false),
        Position::InContainer { item, .. } => (item + 1, None, false),
    };
    // (a page of floats ends where it starts)
    let last = last.max(first);
    Window {
        start: first as u32,
        end: last as u32,
        start_cut,
        end_cut,
        leading,
        // (the margins at the end of the content of a column are not part of it)
        at_end: at_end && scope.is_none(),
        scope,
        start_container: match start {
            Position::InContainer { item, .. } => Some(paragraph(item)),
            _ => None,
        },
        end_container: match end {
            Position::InContainer { item, .. } => Some(paragraph(item)),
            _ => None,
        },
        start_offset: match start {
            Position::InContainer { cut, .. } => Some(cut.item),
            _ => None,
        },
        end_offset: match end {
            Position::InContainer { cut, .. } => Some(cut.item),
            _ => None,
        },
    }
}
