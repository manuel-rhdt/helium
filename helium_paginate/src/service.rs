use crate::flow::{BoxRef, BreakKind, Item, NodeRef, PageTable, ParId, Problem};
use crate::token::Cut;
use std::rc::Rc;

/// A line of a paragraph of the flow.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ParLine {
    /// The height of the line.
    pub extent: f64,
    /// The cut behind the line, where a page that ends after the line is continued. Meaningless
    /// for the last line of the paragraph.
    pub end: Cut,
}

/// Answers the questions of the kernel about sizes.
///
/// Extents are `f64` so that a service that has the positions of a galley can answer with their
/// differences, which add up exactly. There are no width classes: everything is asked for the
/// inline size of the page that it is on, and a service that caches does so by that size.
pub trait LayoutService {
    /// The extent of a `Box` or a `Gap` on a page.
    fn extent(&mut self, node: NodeRef, page_inline: f32) -> f64;

    /// The lines of a paragraph of the flow on a page, from `from` (`None`: from the start of
    /// the paragraph) to its end. The paragraphs are rigid: the lines are what the paragraph
    /// has when it is broken for the page. The same arguments give the same lines.
    fn lines(&mut self, par: ParId, from: Option<Cut>, page_inline: f32) -> Rc<[ParLine]>;

    // The questions about a subtree. They are for a box whose content is laid out at a width that
    // is not the width of any box around it: the columns of a multi-column container, or the
    // table that continues on a page of another width. The box `root` is laid out on its own, as
    // the root of a formatting context, with the *content box* of `root` `inline` wide: its own
    // `width` (and `min-width` and `max-width`) do not apply, its padding and border are added
    // around the width, and percentages in its own padding refer to `inline`. Everything inside
    // it is laid out by the usual rules for that content width. The same arguments give the same
    // answers, and asking does not change what the other methods answer.

    /// The block size of the content box of `root` when its content box is `inline` wide.
    fn content_extent(&mut self, root: BoxRef, inline: f32) -> f64;

    /// The extent of a `Box` or a `Gap` that is inside `root`, when the content box of `root` is
    /// `inline` wide. `extent` answers for the same boxes on a page.
    fn extent_in(&mut self, root: BoxRef, node: NodeRef, inline: f32) -> f64;

    /// The lines of a paragraph that is inside `root`, when the content box of `root` is
    /// `inline` wide: the lines for the width and the indent that the paragraph is given there.
    /// `lines` answers for the same paragraphs on a page.
    fn lines_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]>;
}

/// A layout service that is backed by a galley: the document laid out once, at one width. The
/// extents are what the layout of the galley has computed, whatever the width of the page. The
/// cut of a paragraph is the number of the line that follows it.
///
/// The answers for a subtree are the same whatever the width: the service has one layout only.
#[derive(Debug, Default, Clone)]
pub struct GalleyService {
    nodes: Vec<f64>,
    paragraphs: Vec<Vec<f64>>,
    /// The extent of the content of every box that can be a subtree root.
    boxes: Vec<f64>,
}

impl GalleyService {
    pub fn add_node(&mut self, extent: f64) -> NodeRef {
        self.nodes.push(extent);
        NodeRef(self.nodes.len() as u32 - 1)
    }

    /// A box that can be asked about as the root of a subtree, with the extent of its content.
    pub fn add_box(&mut self, content_extent: f64) -> BoxRef {
        self.boxes.push(content_extent);
        BoxRef(self.boxes.len() as u32 - 1)
    }

    pub fn add_paragraph(&mut self, line_extents: Vec<f64>) -> ParId {
        self.paragraphs.push(line_extents);
        ParId(self.paragraphs.len() as u32 - 1)
    }
}

impl LayoutService for GalleyService {
    fn extent(&mut self, node: NodeRef, _page_inline: f32) -> f64 {
        self.nodes[node.0 as usize]
    }

    fn lines(&mut self, par: ParId, from: Option<Cut>, _page_inline: f32) -> Rc<[ParLine]> {
        let first = from.map_or(0, |cut| cut.0 as usize);
        self.paragraphs[par.0 as usize]
            .iter()
            .enumerate()
            .skip(first)
            .map(|(line, &extent)| ParLine {
                extent,
                end: Cut(line as u32 + 1),
            })
            .collect()
    }

    fn content_extent(&mut self, root: BoxRef, _inline: f32) -> f64 {
        self.boxes[root.0 as usize]
    }

    fn extent_in(&mut self, _root: BoxRef, node: NodeRef, inline: f32) -> f64 {
        self.extent(node, inline)
    }

    fn lines_in(&mut self, _root: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]> {
        self.lines(par, from, inline)
    }
}

/// Builds a flow together with the service for it.
#[derive(Debug, Default, Clone)]
pub struct GalleyFlow {
    items: Vec<Item>,
    service: GalleyService,
}

impl GalleyFlow {
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of items so far.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// A box that cannot be split.
    pub fn content(&mut self, extent: f64) -> &mut Self {
        let node = self.service.add_node(extent);
        self.items.push(Item::Box { node });
        self
    }

    pub fn gap(&mut self, extent: f64) -> &mut Self {
        let node = self.service.add_node(extent);
        self.items.push(Item::Gap { node });
        self
    }

    pub fn break_(&mut self, kind: BreakKind) -> &mut Self {
        self.items.push(Item::Break { kind });
        self
    }

    pub fn paragraph(
        &mut self,
        line_extents: &[f64],
        orphans: u32,
        widows: u32,
        avoid_inside: bool,
    ) -> &mut Self {
        let id = self.service.add_paragraph(line_extents.to_vec());
        self.items.push(Item::Paragraph {
            id,
            orphans,
            widows,
            avoid_inside,
        });
        self
    }

    pub fn finish(self, pages: PageTable) -> (Problem, GalleyService) {
        (Problem::new(pages, self.items), self.service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_galley_answers_for_a_subtree_with_what_it_has_whatever_the_width() {
        let mut flow = GalleyFlow::new();
        flow.content(30.0).paragraph(&[10.0, 12.0, 14.0], 2, 2, false);
        let (_, mut service) = flow.finish(PageTable::uniform(Default::default()));
        let root = service.add_box(36.0);
        for inline in [100.0, 250.0] {
            assert_eq!(service.content_extent(root, inline), 36.0);
            assert_eq!(service.extent_in(root, NodeRef(0), inline), 30.0);
            let lines = service.lines_in(root, ParId(0), Some(Cut(1)), inline);
            assert_eq!(lines.iter().map(|line| line.extent).collect::<Vec<_>>(), [12.0, 14.0]);
            assert_eq!(lines[0].end, Cut(2));
            assert_eq!(lines, service.lines(ParId(0), Some(Cut(1)), inline));
        }
        // a second box has an extent of its own
        let other = service.add_box(5.0);
        assert_ne!(other, root);
        assert_eq!(service.content_extent(other, 100.0), 5.0);
        assert_eq!(service.content_extent(root, 100.0), 36.0);
    }
}
