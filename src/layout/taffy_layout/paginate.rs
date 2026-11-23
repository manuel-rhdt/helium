//! The pages of the document: the geometry of the page that starts at every place of the flow,
//! the choice of the pages, and the windows that the pages are laid out from.
//!
//! The kernel and the solvers of `helium_paginate` do the choosing. This module tells them what
//! a page looks like at a place (its name and its kind decide the margins and the
//! size), and turns what they choose into a window for every
//! page (see [`super::window`]).

use super::flow::Flow;
use taffy::tree::NodeId;
use super::tree::HeliumTree;
use super::window::{Leading, Window};
use crate::layout::components::EdgeValues;
use crate::layout::pages::{PageContext, PageMargins};
use crate::style::properties::computed::PixelLength;
use helium_paginate::{BreakLevel, Item, PageKind, PageSpec, PageTable, Position};
use std::collections::HashMap;
use std::sync::Arc;

/// What decides the geometry of a page, other than its kind: the name of the first content on
/// it.
type Class = Option<Arc<str>>;

/// The class of the page that starts at every place of the flow where one can start.
pub(super) struct Classes {
    classes: Vec<Class>,
    /// By the item that the page starts in front of.
    at_item: Vec<usize>,
    /// By the paragraph or container item that the page starts in.
    in_paragraph: HashMap<usize, usize>,
}

impl Classes {
    pub fn new(tree: &HeliumTree, flow: &Flow) -> Self {
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
        let mut at_item = Vec::with_capacity(len + 1);
        for item in 0..=len {
            let leaf = next_leaf[item].and_then(|leaf| flow.leaves[leaf].map(|node| (leaf, node)));
            let name = leaf.and_then(|(_, node)| name_of(node));
            at_item.push(id_of(name));
        }
        let mut in_paragraph = HashMap::new();
        for (item, entry) in flow.items.iter().enumerate() {
            if let (Item::Paragraph { .. } | Item::Container { .. }, Some(node)) = (entry, flow.leaves[item]) {
                in_paragraph.insert(item, id_of(name_of(node)));
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
            name: class.clone(),
            rtl: self.margins.rtl,
        }
    }

    /// The margins of a page, and its width and height.
    pub fn of_page(&self, page: &PageContext) -> (EdgeValues<PixelLength>, (PixelLength, PixelLength)) {
        let edges = self.margins.pages.get(&crate::layout::pages::normalize(page, &self.margins.names));
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
    }
}
