use crate::flow::{BreakKind, Item, NodeRef, PageTable, ParId, Problem};

/// Answers the questions of the kernel about sizes.
///
/// Extents are `f64` so that a service that has the positions of a galley can answer with their
/// differences, which add up exactly.
pub trait LayoutService {
    /// The extent of a `Box` or a `Gap` on a page.
    fn extent(&mut self, node: NodeRef, page_inline: f32) -> f64;

    /// The extent of the line `line` of a paragraph of the flow on a page.
    ///
    /// The lines are the ones that the paragraph has, since the paragraphs are rigid.
    fn line_extent(&mut self, par: ParId, line: u32, page_inline: f32) -> f64;
}

/// A layout service that is backed by a galley: the document laid out once, at one width. The
/// extents are what the layout of the galley has computed.
#[derive(Debug, Default, Clone)]
pub struct GalleyService {
    nodes: Vec<f64>,
    paragraphs: Vec<Vec<f64>>,
}

impl GalleyService {
    pub fn add_node(&mut self, extent: f64) -> NodeRef {
        self.nodes.push(extent);
        NodeRef(self.nodes.len() as u32 - 1)
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

    fn line_extent(&mut self, par: ParId, line: u32, _page_inline: f32) -> f64 {
        self.paragraphs[par.0 as usize][line as usize]
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
            lines: line_extents.len() as u32,
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
