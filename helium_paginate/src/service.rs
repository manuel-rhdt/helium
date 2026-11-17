use crate::columns::{plan_fragment, ColumnFill};
use crate::flow::{BoxRef, BreakKind, BreakLevel, Item, NodeRef, PageTable, ParId, Problem};
use crate::token::{ContainerCut, Cut};
use std::rc::Rc;

/// A line of a paragraph of the flow.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ParLine {
    /// The height of the line.
    pub extent: f64,
    /// The cut behind the line, where a page that ends after the line is continued. Meaningless
    /// for the last line of the paragraph, except in [`LayoutService::lines_to`], where the last
    /// line ends at the cut that was asked for.
    pub end: Cut,
    /// The demerits of the line when the paragraph is broken as the breaker counts them (the
    /// optimum has the least of them), or 0 for a service that does not count them. A page that
    /// has some of the lines of a breaking has their demerits.
    pub demerits: f64,
    /// The badness of the line (TeX's, 0 for a line whose spaces are as they are, and 100 for
    /// a line that stretches or shrinks them as far as their stretch or shrink), or 0 for a
    /// service that does not count it. A breaking with a line that is worse than the tolerance is
    /// not a way that a document that asks for tolerable lines takes.
    pub badness: f64,
}

/// A way for a fragment of a container to end: what a page has of the container.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Fragment {
    /// Where the next fragment starts, or `None` if the container ends in this one.
    pub end: Option<ContainerCut>,
    /// The extent of the fragment on the page: its columns, and the padding and the border at
    /// the top (of the first fragment) and at the bottom (of the last).
    pub used: f64,
    /// The kind of the break at the end of the fragment: forced at a forced break of the page
    /// level, else allowed or to be avoided.
    pub end_kind: BreakKind,
    /// Some column of the fragment ends where it should not, or has fewer lines of a paragraph
    /// than `orphans` or `widows` ask for.
    pub orphan_violation: bool,
    pub widow_violation: bool,
    pub avoid_violation: bool,
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

    /// The ways to break a paragraph of the flow on a page, from `from` (`None`: from the start of
    /// the paragraph) to its end: the breaking that [`lines`](Self::lines) gives, and then, if
    /// `extra` is not 0, breakings with `extra` lines fewer and more than it has at the least
    /// (the number of lines that the paragraph has when it is broken for the page, and the next
    /// ones; a number of lines that no breaking has is left out). Each has the least demerits that
    /// a breaking with its number of lines can have, and they are in the order of their number of
    /// lines, after the first. The same arguments give the same breakings.
    ///
    /// A service that has only one way to break a paragraph, as the default has, gives only that.
    fn variants(&mut self, par: ParId, from: Option<Cut>, page_inline: f32, extra: u32) -> Vec<Rc<[ParLine]>> {
        let _ = extra;
        vec![self.lines(par, from, page_inline)]
    }

    /// The lines of the part of a paragraph from `from` to the cut `to`, which can be any place
    /// where the paragraph can be broken, not only a place where a line of [`lines`](Self::lines)
    /// ends. The last line ends at `to`, and it is a line like the others (it is not the last
    /// line of the paragraph), and the lines before it are broken to fit it. A service whose
    /// paragraphs can only be broken as [`lines`](Self::lines) has them returns the lines up to
    /// the line that ends at `to`, and panics if there is none.
    ///
    /// Only the place of `to` is used, and what comes with it (the state of the breaking, if the
    /// service has one) is not: the last of the lines that are returned has the cut as the
    /// service makes it. This is how a page break at any place of a paragraph is a break that can
    /// be made: see [`Kernel::fill_at`](crate::Kernel::fill_at).
    fn lines_to(&mut self, par: ParId, from: Option<Cut>, to: Cut, page_inline: f32) -> Rc<[ParLine]> {
        let lines = self.lines(par, from, page_inline);
        match lines.iter().position(|line| line.end == to) {
            Some(index) => Rc::from(&lines[..=index]),
            None => panic!("this layout service cannot break a paragraph at {to:?}"),
        }
    }

    /// The extent that a float takes on a page that is `page_inline` wide: its margin box, laid
    /// out on its own at the width of the page. Margins are not collapsed with anything.
    ///
    /// A service that has no floats is not asked.
    fn float_extent(&mut self, float: BoxRef, page_inline: f32) -> f64 {
        let _ = (float, page_inline);
        panic!("the layout service has no floats")
    }

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

    /// [`variants`](Self::variants) for a paragraph that is inside `root`: the breakings for the
    /// width and the indent that the paragraph is given there.
    fn variants_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32, extra: u32) -> Vec<Rc<[ParLine]>> {
        let _ = extra;
        vec![self.lines_in(root, par, from, inline)]
    }

    /// [`lines_to`](Self::lines_to) for a paragraph that is inside `root`: the lines for the width
    /// and the indent that the paragraph is given there.
    fn lines_to_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, to: Cut, inline: f32) -> Rc<[ParLine]> {
        let lines = self.lines_in(root, par, from, inline);
        match lines.iter().position(|line| line.end == to) {
            Some(index) => Rc::from(&lines[..=index]),
            None => panic!("this layout service cannot break a paragraph at {to:?}"),
        }
    }

    /// The ways that a fragment of a container that starts at `from` (`None`: at its start) can
    /// end on a page that is `page_inline` wide, when `available` is what is left of the page.
    /// The fragments are listed in the order of growing content. A fragment that has an `end` goes
    /// on on the next page, and a fragment without one is the end of the container. If
    /// nothing fits, the fragment is the least that can be placed, and it is taller than
    /// `available`. The same arguments give the same fragments.
    ///
    /// A service that has no containers is not asked.
    fn fragment(
        &mut self,
        container: BoxRef,
        from: Option<ContainerCut>,
        page_inline: f32,
        available: f64,
    ) -> Rc<[Fragment]> {
        let _ = (container, from, page_inline, available);
        panic!("the layout service has no containers")
    }
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
    containers: Vec<GalleyContainer>,
    /// The extent of every float.
    floats: Vec<f64>,
}

/// A container of a galley: a multi-column container with a flow of its own.
#[derive(Debug, Clone)]
struct GalleyContainer {
    items: Vec<Item>,
    columns: usize,
    fill: ColumnFill,
    /// The padding and the border at the top and the bottom.
    edges: (f64, f64),
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

    /// A container. Its content is made of the items of this service. The box that is returned is
    /// asked for by the item of the container.
    pub fn add_container(&mut self, items: Vec<Item>, columns: usize, fill: ColumnFill, edges: (f64, f64)) -> BoxRef {
        self.containers.push(GalleyContainer { items, columns, fill, edges });
        BoxRef(self.containers.len() as u32 - 1)
    }

    /// A float with the extent that it takes on a page.
    pub fn add_float(&mut self, extent: f64) -> BoxRef {
        self.floats.push(extent);
        BoxRef(self.floats.len() as u32 - 1)
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
                demerits: 0.0,
                badness: 0.0,
                extent,
                end: Cut(line as u32 + 1),
            })
            .collect()
    }

    fn float_extent(&mut self, float: BoxRef, _page_inline: f32) -> f64 {
        self.floats[float.0 as usize]
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

    fn fragment(
        &mut self,
        container: BoxRef,
        from: Option<ContainerCut>,
        page_inline: f32,
        available: f64,
    ) -> Rc<[Fragment]> {
        let GalleyContainer { items, columns, fill, edges } = self.containers[container.0 as usize].clone();
        let (top, bottom) = edges;
        let top = if from.is_none() { top } else { 0.0 };
        // (the bottom edge is only known to be there once the container ends)
        let plan = plan_fragment(self, &items, columns, page_inline, from, (available - top) as f32, bottom as f32, fill, crate::Variants::default());
        let used = plan.height as f64 + top + if plan.end.is_none() { bottom } else { 0.0 };
        Rc::from(vec![Fragment {
            end: plan.end,
            used,
            end_kind: plan.end_kind,
            orphan_violation: plan.orphan_violation,
            widow_violation: plan.widow_violation,
            avoid_violation: plan.avoid_violation,
        }])
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

    /// A break between pages.
    pub fn break_(&mut self, kind: BreakKind) -> &mut Self {
        self.break_at(kind, BreakLevel::Page)
    }

    pub fn break_at(&mut self, kind: BreakKind, level: BreakLevel) -> &mut Self {
        self.items.push(Item::Break { kind, level });
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

    /// A float: it takes `extent` on the page that has the place where it is called for.
    pub fn float(&mut self, extent: f64, side: crate::FloatSide) -> &mut Self {
        let node = self.service.add_float(extent);
        self.items.push(Item::Float { node, side });
        self
    }

    /// A container with `columns` columns. Its content is the flow `inner`, which has its own
    /// numbers of boxes and paragraphs. The breaks in it are the ones of its columns, and a break
    /// of the level `Page` in it ends a fragment. `edges` are the padding and the border at the
    /// top and at the bottom.
    pub fn container(&mut self, inner: GalleyFlow, columns: usize, fill: ColumnFill, edges: (f64, f64)) -> &mut Self {
        let nodes = self.service.nodes.len() as u32;
        let paragraphs = self.service.paragraphs.len() as u32;
        self.service.nodes.extend(inner.service.nodes);
        self.service.paragraphs.extend(inner.service.paragraphs);
        let items = inner
            .items
            .into_iter()
            .map(|item| match item {
                Item::Box { node } => Item::Box { node: NodeRef(node.0 + nodes) },
                Item::Gap { node } => Item::Gap { node: NodeRef(node.0 + nodes) },
                Item::Paragraph { id, orphans, widows, avoid_inside } => {
                    Item::Paragraph { id: ParId(id.0 + paragraphs), orphans, widows, avoid_inside }
                }
                Item::Break { .. } => item,
                Item::Container { .. } => panic!("a container is not in a container"),
                Item::Float { .. } => panic!("a float is not in a container"),
            })
            .collect();
        let node = self.service.add_container(items, columns, fill, edges);
        self.items.push(Item::Container { node, avoid_inside: false });
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
