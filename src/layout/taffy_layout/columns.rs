//! Multi-column containers.
//!
//! Taffy has no multi-column layout. A container with `column-count` or `column-width` is a leaf
//! for Taffy, like a paragraph: its width comes from its container, and its height from the
//! columns (see `docs/pagination.md`).
//!
//! - **The plan.** The columns are chosen by the kernel of `helium_paginate` from the flow of the
//!   content of the container ([`super::flow::Inner`]). The problem has the columns as its pages: as
//!   many as the container has, as wide as a column, and as high as the content needs (balanced)
//!   or as the container is, if it has a height. The extents and the lines are those of the
//!   content laid out for the width of a column ([`super::subtree`]). The plan is made once for a
//!   width and a height.
//! - **Fragments.** A container whose height is the one of its content is split over pages (it is an
//!   `Item::Container` of the flow of the document). The kernel asks the layout service for the
//!   fragment of it that fits in what is left of a page ([`fragment_plan`]): the columns of the
//!   fragment are chosen from the flow of the content like the columns of a container that is not
//!   split, but from the place where the fragment starts, and with the height that is available.
//!   The plan of the fragment is kept by the arguments of the question, so that the layout of the
//!   page finds it again ([`FragmentKey`]). Where the container is laid out without a page (the
//!   galleys of the layout service), its height is not used, and it is not laid out.
//! - **The layout of the columns.** Once the pages are chosen, every column is laid out through
//!   a window ([`super::window`]) on the content of the container, with the container as a box
//!   without padding, border and margin that is as wide as a column. The layouts of the boxes in
//!   the window are copied for the column, because the next column lays out the same boxes
//!   again; the [`Writer`](super::Writer) puts them where the column is.

use super::flow::Inner;
use super::paginate;
use super::service::TreeService;
use super::tree::{Content, HeliumTree, calc};
use super::window::Window;
use crate::style::properties::computed::{ColumnWidth, GenericSize, PixelLength};
use crate::style::properties::specified::{ColumnCount, ColumnFill, Display};
use helium_paginate::{
    BoxRef, BreakLevel, ContainerCut, Cut, Item, Kernel, LayoutService, NodeRef, PageSpec, PageTable,
    ParId, ParLine, Position, Problem, balance, plan_fragment,
};
use std::collections::HashMap;
use std::rc::Rc;
use taffy::geometry::{Line, Size};
use taffy::style::AvailableSpace;
use taffy::tree::{Layout, LayoutInput, NodeId, RunMode, SizingMode};
use taffy::{MaybeResolve, RequestedAxis, compute_block_layout, compute_leaf_layout};

/// What a multi-column container knows about its columns (a part of its node).
#[derive(Debug, Default)]
pub(super) struct ColumnsNode {
    /// The flow of the content. It is made after the tree.
    pub inner: Option<Rc<Inner>>,
    /// Whether the container is split over pages: it is an item of the flow of the document.
    pub splittable: bool,
    /// The plans, by the width of the content box and the height of it, if it is definite.
    plans: HashMap<(u32, Option<u32>), Rc<ColumnPlan>>,
    /// The plans of the fragments of the container that the kernel asked about.
    pub fragments: HashMap<FragmentKey, Rc<ColumnPlan>>,
    /// The layouts of the columns of the last layout of the container. The last layout is the one
    /// that the layouts of the boxes around it are of, which a box that has a cached layout keeps.
    pub layouts: Option<Rc<ColumnLayouts>>,
}

/// What the kernel asked the layout service about a fragment of a container with: where it starts
/// in the container, the inline size of the page (the bits of the number) and the height that was
/// available on the page (the bits of the number), and where the fragment ends.
pub(super) type FragmentKey = (Option<ContainerCut>, u32, u64, Option<ContainerCut>);

/// The columns of a container for a width and a height.
#[derive(Debug)]
pub(super) struct ColumnPlan {
    /// The width of a column and the gap between two.
    pub width: f32,
    pub gap: f32,
    /// The block size of the content box of the container: the height of the columns, which is
    /// as much as the tallest of them needs, or the height of the container if it has one.
    pub height: f32,
    /// The columns that have content, in order. There can be fewer than the container can have.
    pub columns: Vec<Window>,
    /// For every column, the ways to break its paragraphs that are not the first, by paragraph.
    pub variants: Vec<HashMap<NodeId, usize>>,
    /// The extent that the content of every column has (for the report of the pages).
    pub used: Vec<f32>,
}

/// What the layout of one box in a column found out, which the layout of the next column
/// overwrites.
#[derive(Debug, Clone, Copy)]
pub(super) struct Snap {
    pub layout: Layout,
    pub measured: Option<(f32, f32)>,
    pub content_shift: f32,
}

/// The layouts of the boxes of the columns of a container, by column.
#[derive(Debug)]
pub(super) struct ColumnLayouts {
    pub plan: Rc<ColumnPlan>,
    pub columns: Vec<HashMap<NodeId, Snap>>,
}

/// The window that Taffy is shown, to put it back.
pub(super) struct SavedWindow {
    window: Option<Window>,
    windowed: Vec<NodeId>,
    styles: Vec<(NodeId, Option<taffy::Style>, bool)>,
}

/// The questions of the kernel about the content of a container, answered for the content laid
/// out for the width of a column.
struct InColumns<'s, 't, 'a, 'w> {
    service: &'s mut TreeService<'t, 'a, 'w>,
    root: BoxRef,
}

impl LayoutService for InColumns<'_, '_, '_, '_> {
    fn extent(&mut self, node: NodeRef, inline: f32) -> f64 {
        self.service.extent_in(self.root, node, inline)
    }

    fn lines(&mut self, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]> {
        self.service.lines_in(self.root, par, from, inline)
    }

    fn variants(&mut self, par: ParId, from: Option<Cut>, inline: f32, extra: u32) -> Vec<Rc<[ParLine]>> {
        self.service.variants_in(self.root, par, from, inline, extra)
    }

    fn variants_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32, extra: u32) -> Vec<Rc<[ParLine]>> {
        self.service.variants_in(root, par, from, inline, extra)
    }

    fn lines_to(&mut self, par: ParId, from: Option<Cut>, to: Cut, inline: f32) -> Rc<[ParLine]> {
        self.service.lines_to_in(self.root, par, from, to, inline)
    }

    fn lines_to_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, to: Cut, inline: f32) -> Rc<[ParLine]> {
        self.service.lines_to_in(root, par, from, to, inline)
    }

    fn content_extent(&mut self, root: BoxRef, inline: f32) -> f64 {
        self.service.content_extent(root, inline)
    }

    fn extent_in(&mut self, root: BoxRef, node: NodeRef, inline: f32) -> f64 {
        self.service.extent_in(root, node, inline)
    }

    fn lines_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]> {
        self.service.lines_in(root, par, from, inline)
    }
}

impl HeliumTree<'_, '_> {
    /// Finds the multi-column containers: boxes with block or inline content that have a
    /// `column-count` or a `column-width`. Not the root, not tables and not flex or grid
    /// containers, and not boxes with a vertical flow (a column is a fragmentainer of the block
    /// axis of a horizontal flow only).
    pub(super) fn mark_columns(&mut self, root: NodeId) {
        for index in 0..self.nodes.len() {
            let id = NodeId::from(index);
            let node = self.node(id);
            if id == root || node.orthogonal || node.flow.is_vertical() {
                continue;
            }
            if !matches!(node.content, Content::Blocks | Content::Inline) {
                continue;
            }
            let Some(style) = self.computed_values.get(node.entity) else {
                continue;
            };
            if !matches!(style.box_.display, Display::Block | Display::ListItem | Display::InlineBlock) {
                continue;
            }
            if style.box_.column_count == ColumnCount::Auto && style.box_.column_width == ColumnWidth::Auto {
                continue;
            }
            self.node_mut(id).columns = Some(Box::default());
        }
    }

    /// The number of columns, their width and the gap between them, for a container whose
    /// content box is `content_width` wide.
    /// https://www.w3.org/TR/css-multicol-1/#pseudo-algorithm
    pub(super) fn column_geometry(&self, id: NodeId, content_width: f32) -> (usize, f32, f32) {
        let style = self.computed_values.get(self.node(id).entity);
        let (count, width, gap) = style.map_or((None, None, 0.0), |style| {
            let gap = match style.box_.column_gap {
                GenericSize::Auto => style.font.font_size.0,
                GenericSize::LengthPercentage(gap) => gap.absolute(PixelLength(content_width)).0,
            };
            let count = match style.box_.column_count {
                ColumnCount::Auto => None,
                ColumnCount::Count(count) => Some(count),
            };
            let width = match style.box_.column_width {
                ColumnWidth::Auto => None,
                // (a column is not narrower than a pixel)
                ColumnWidth::Length(width) => Some(width.0.max(1.0)),
            };
            (count, width, gap.max(0.0))
        });
        let fits = |width: f32| (((content_width + gap) / (width + gap)).floor() as usize).max(1);
        let columns = match (count, width) {
            (Some(count), Some(width)) => count.min(fits(width)),
            (Some(count), None) => count,
            (None, Some(width)) => fits(width),
            (None, None) => 1,
        };
        let width = ((content_width - (columns - 1) as f32 * gap) / columns as f32).max(0.0);
        (columns, width, gap)
    }

    /// Takes the window off the tree, to put it back with [`Self::restore_window`].
    pub(super) fn save_window(&mut self) -> SavedWindow {
        let styles = self
            .windowed
            .iter()
            .map(|&id| {
                let node = self.node(id);
                (id, node.window_style.clone(), node.windowed)
            })
            .collect();
        let saved = SavedWindow {
            window: self.window.clone(),
            windowed: self.windowed.clone(),
            styles,
        };
        self.clear_window();
        saved
    }

    pub(super) fn restore_window(&mut self, saved: SavedWindow) {
        self.clear_window();
        for (id, style, windowed) in saved.styles {
            let node = self.node_mut(id);
            node.window_style = style;
            node.windowed = windowed;
        }
        self.windowed = saved.windowed;
        self.window = saved.window;
    }

    /// The columns of a container for a content box of the width, and of the height if it is
    /// definite.
    fn plan_columns(&mut self, id: NodeId, content_width: f32, height: Option<f32>) -> Rc<ColumnPlan> {
        let key = (content_width.to_bits(), height.map(f32::to_bits));
        if let Some(plan) = self.node(id).columns.as_ref().and_then(|columns| columns.plans.get(&key)) {
            return plan.clone();
        }
        let (count, width, gap) = self.column_geometry(id, content_width);
        let inner = self
            .node(id)
            .columns
            .as_ref()
            .and_then(|columns| columns.inner.clone())
            .expect("the content of a container with columns is a flow");
        let flow = self.flow.clone().expect("the flow is known when the columns are chosen");
        let icb_height = self.icb_height;
        let config = self.variants_config;
        let root = BoxRef(usize::from(id) as u32);
        // (the question about the content lay it out, and that takes the window off the tree)
        let saved = self.save_window();
        let plan = {
            let mut service = TreeService::new(self, &*flow, icb_height);
            let mut service = InColumns { service: &mut service, root };
            choose(&mut service, id, root, &inner, count, width, gap, height, config)
        };
        self.restore_window(saved);
        let plan = Rc::new(plan);
        if let Some(columns) = self.node_mut(id).columns.as_mut() {
            columns.plans.insert(key, plan.clone());
        }
        plan
    }

    /// The height of a container with columns that is split over pages in a layout that is not of
    /// a page (see `compute_columns_layout`).
    const UNPLACED_HEIGHT: f32 = 0.01;

    /// Lays out a container with columns as a leaf: it is as wide as a block is, and as high as its
    /// columns.
    pub(super) fn compute_columns_layout(&mut self, id: NodeId, inputs: LayoutInput) -> taffy::LayoutOutput {
        let style = self.active_style(id).clone();
        let splittable = self.node(id).columns.as_ref().is_some_and(|columns| columns.splittable);
        if splittable {
            // A container that is split over pages has the height of its fragment on the page. In
            // a layout that is not of a page (the galley, which only gives extents) its height is
            // not used, and nothing in it is laid out.
            let plan = self.page_fragments.get(&id).filter(|_| self.window.is_some()).and_then(|key| {
                self.node(id).columns.as_ref().and_then(|columns| columns.fragments.get(key).cloned())
            });
            let output = compute_leaf_layout(inputs, &style, calc, |_, available| Size {
                width: available.width.into_option().unwrap_or(self.fallback_line_width).max(0.0),
                // (the container has a height in a layout that is not of a page, too, though not its
                // own: with none, its margins would collapse through it, and the gap between it
                // and the box that follows it, which the paginator counts, would be lost)
                height: plan.as_ref().map_or(Self::UNPLACED_HEIGHT, |plan| plan.height),
            });
            if inputs.run_mode == RunMode::PerformLayout {
                if let Some(plan) = plan {
                    self.layout_columns(id, plan);
                }
            }
            return output;
        }
        // the height of the content box, if the container has a height of its own
        let definite = inputs.known_dimensions.height.is_some()
            || style.size.height.maybe_resolve(inputs.parent_size.height, calc).is_some();
        let mut planned = None;
        let output = compute_leaf_layout(inputs, &style, calc, |_, available| {
            let width = available.width.into_option().unwrap_or(self.fallback_line_width).max(0.0);
            let height = available.height.into_option().filter(|_| definite).map(|height| height.max(0.0));
            let plan = self.plan_columns(id, width, height);
            let size = Size { width, height: plan.height };
            planned = Some(plan);
            size
        });
        if inputs.run_mode == RunMode::PerformLayout {
            if let Some(plan) = planned {
                self.layout_columns(id, plan);
            }
        }
        output
    }

    /// Lays out every column of the plan, and keeps what it found out for the pass that writes
    /// the positions.
    fn layout_columns(&mut self, id: NodeId, plan: Rc<ColumnPlan>) {
        let saved = self.save_window();
        let mut columns = vec![];
        let page_variants = self.page_variants.clone();
        for (index, window) in plan.columns.iter().enumerate() {
            // (the ways to break the paragraphs that the column has, for the lines and for the
            // boxes above them, which Taffy must not take from its cache)
            self.page_variants = page_variants.clone();
            if let Some(ways) = plan.variants.get(index) {
                self.page_variants.extend(ways.iter().map(|(&node, &way)| (node, way)));
            }
            self.set_window(window.clone(), id);
            // the box of a column: as wide as a column, with nothing around it
            let mut style = self.node(id).style.clone();
            style.size = taffy::Size { width: taffy::Dimension::length(plan.width), height: taffy::Dimension::auto() };
            style.min_size = taffy::Size { width: taffy::style_helpers::auto(), height: taffy::style_helpers::auto() };
            style.max_size = taffy::Size { width: taffy::style_helpers::auto(), height: taffy::style_helpers::auto() };
            let zero = taffy::style::LengthPercentage::length(0.0);
            style.padding = taffy::Rect { left: zero, right: zero, top: zero, bottom: zero };
            style.border = taffy::Rect { left: zero, right: zero, top: zero, bottom: zero };
            let zero_auto = taffy::style::LengthPercentageAuto::length(0.0);
            style.margin = taffy::Rect { left: zero_auto, right: zero_auto, top: zero_auto, bottom: zero_auto };
            style.aspect_ratio = None;
            style.position = taffy::Position::Relative;
            let node = self.node_mut(id);
            node.window_style = Some(style);
            node.windowed = true;
            let column_inputs = LayoutInput {
                run_mode: RunMode::PerformLayout,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                known_dimensions: Size { width: Some(plan.width), height: None },
                known_dimensions_are_definite: Size { width: true, height: false },
                // (the height of the container is not what the percentages in the columns refer to:
                // the extents that the columns were chosen from were made without it)
                parent_size: Size { width: Some(plan.width), height: None },
                available_space: Size {
                    width: AvailableSpace::Definite(plan.width),
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            };
            match self.node(id).content {
                Content::Inline => {
                    self.compute_inline_layout(id, column_inputs);
                }
                _ => {
                    compute_block_layout(self, id, column_inputs, None);
                    self.mirror_in(id, plan.width);
                }
            }
            columns.push(self.snapshot(id));
        }
        self.page_variants = page_variants;
        self.restore_window(saved);
        if let Some(node) = self.node_mut(id).columns.as_mut() {
            node.layouts = Some(Rc::new(ColumnLayouts { plan, columns }));
        }
    }

    /// Mirrors the in-flow children of a right-to-left container with columns in a column that
    /// is `width` wide (see `Node::mirror_rtl`, [`HeliumTree::mirror_children`]).
    fn mirror_in(&mut self, id: NodeId, width: f32) {
        if !self.node(id).mirror_rtl {
            return;
        }
        for child in self.window_children(id).to_vec() {
            let node = self.node_mut(child);
            if node.style.position == taffy::Position::Absolute || node.style.display == taffy::Display::None {
                continue;
            }
            node.layout.location.x = width - node.layout.location.x - node.layout.size.width;
            let margin = &mut node.layout.margin;
            std::mem::swap(&mut margin.left, &mut margin.right);
        }
    }

    /// The layouts of the boxes that Taffy has just laid out in the window.
    fn snapshot(&self, id: NodeId) -> HashMap<NodeId, Snap> {
        let mut snaps = HashMap::new();
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let node = self.node(id);
            snaps.insert(
                id,
                Snap { layout: node.layout, measured: node.measured, content_shift: node.content_shift },
            );
            stack.extend(self.window_children(id).iter().copied());
            stack.extend(node.hoisted.iter().copied());
            if let Some(table) = &node.table {
                stack.extend(table.boxes.iter().copied());
            }
        }
        snaps
    }
}

/// The ways to break the paragraphs of a column that are not the first, by paragraph.
fn ways_of(inner: &Inner, choices: &[helium_paginate::Choice]) -> HashMap<NodeId, usize> {
    choices
        .iter()
        .filter_map(|choice| inner.leaves.get(choice.item).copied().flatten().map(|node| (node, choice.variant)))
        .collect()
}

/// Chooses the columns of a container from its content, which is laid out for the width of a
/// column.
#[allow(clippy::too_many_arguments)]
fn choose(
    service: &mut InColumns<'_, '_, '_, '_>,
    id: NodeId,
    root: BoxRef,
    inner: &Inner,
    count: usize,
    width: f32,
    gap: f32,
    height: Option<f32>,
    config: helium_paginate::Variants,
) -> ColumnPlan {
    let whole = || Window {
        start: 0,
        end: inner.items.len() as u32,
        start_cut: None,
        end_cut: None,
        leading: super::window::Leading::Natural,
        at_end: false,
        scope: Some(id),
        start_container: None,
        end_container: None,
    };
    if inner.items.is_empty() {
        return ColumnPlan { width, gap, height: height.unwrap_or(0.0), columns: vec![], variants: vec![], used: vec![] };
    }
    // what the container is as high as: a column of the content on its own is as high as the
    // content, the columns of a container with a height are as high as it is
    let content = service.content_extent(root, width) as f32;
    let mut bound = match height {
        Some(height) => height,
        // The columns are not much higher than the content divided by their number, unless
        // something has to be kept together. The bound grows if it is too small.
        None => (content / count as f32 * 1.15).min(content) + 1.0,
    };
    let (starts, used, variants) = loop {
        let table = PageTable::uniform(PageSpec { inline_size: width, block_size: bound });
        let problem = Problem::new(table, inner.items.clone())
            .with_variants(config.extra, config.tolerance)
            .with_line_tolerance(config.line_tolerance)
            .with_level(BreakLevel::Column)
            .with_page_limit(count);
        let mut kernel = Kernel::new(&problem, service);
        let solution = balance(&mut kernel);
        let Ok(fills) = kernel.realize(&solution) else {
            log::warn!("the columns of a container cannot be chosen: its content is not split");
            return ColumnPlan { width, gap, height: height.unwrap_or(content), columns: vec![whole()], variants: vec![], used: vec![] };
        };
        // Within the bound, nothing was found that keeps all the rules, and there is room for a
        // taller column: look again with a higher bound.
        let broken = fills.iter().any(|fill| fill.breaks_a_rule());
        if height.is_none() && broken && bound < content + 1.0 {
            bound = (bound * 1.6).min(content + 1.0);
            continue;
        }
        let positions: Vec<(Position, Position)> = solution
            .pages
            .iter()
            .zip(&fills)
            .map(|(start, fill)| (kernel.position(start.at), kernel.position(fill.end)))
            .collect();
        let ways: Vec<HashMap<NodeId, usize>> = fills
            .iter()
            .map(|fill| ways_of(inner, &kernel.choices(fill.choices)))
            .collect();
        break (positions, fills.iter().map(|fill| fill.used).collect::<Vec<f32>>(), ways);
    };
    let mut columns = vec![];
    for (start, end) in starts {
        let gap_extent = |item: usize| match inner.items[item] {
            Item::Gap { node } => service.extent(node, width) as f32,
            _ => 0.0,
        };
        columns.push(paginate::window(&inner.items, &inner.leaves, BreakLevel::Column, Some(id), start, end, gap_extent));
    }
    let tallest = used.iter().copied().fold(0.0, f32::max);
    ColumnPlan { width, gap, height: height.unwrap_or(tallest), columns, variants, used: used.clone() }
}

/// The fragment of a container that starts at `from` and has `available` height on a page that is
/// `page_inline` wide, as the layout service answers the kernel about it: the columns, and what
/// the fragment is as a whole. The plan is kept in the container by the arguments, for the layout
/// of the page.
///
/// `available` is what the kernel asked about, and what the plan is kept for; `planned` is what the
/// columns are chosen for, which is less than it for a fragment that ends earlier than the one that
/// fills the page.
///
/// `content_width` is the width of the content box of the container on that page, `top` the
/// padding and the border that the fragment has at its top, and `bottom` the ones that it has at
/// its bottom if it ends the container.
#[allow(clippy::too_many_arguments)]
pub(super) fn fragment_plan(
    service: &mut TreeService<'_, '_, '_>,
    id: NodeId,
    content_width: f32,
    from: Option<ContainerCut>,
    page_inline: f32,
    available: f64,
    top: f32,
    bottom: f32,
    planned: f64,
) -> (Rc<ColumnPlan>, helium_paginate::FragmentPlan) {
    let tree = &*service.tree;
    let (count, width, gap) = tree.column_geometry(id, content_width);
    let inner = tree
        .node(id)
        .columns
        .as_ref()
        .and_then(|columns| columns.inner.clone())
        .expect("the content of a container with columns is a flow");
    let fill = tree
        .computed_values
        .get(tree.node(id).entity)
        .map_or(ColumnFill::Balance, |style| style.box_.column_fill);
    let fill = match fill {
        ColumnFill::Balance => helium_paginate::ColumnFill::Balance,
        ColumnFill::BalanceAll => helium_paginate::ColumnFill::BalanceAll,
        ColumnFill::Auto => helium_paginate::ColumnFill::Auto,
    };
    let root = BoxRef(usize::from(id) as u32);
    let mut inside = InColumns { service, root };
    let room = planned as f32 - top;
    let config = inside.service.tree.variants_config;
    let found = plan_fragment(&mut inside, &inner.items, count, width, from, room, bottom, fill, config);
    let service = inside.service;
    let mut columns = vec![];
    for &(start, end) in &found.columns {
        let gap_extent = |item: usize| match inner.items[item] {
            Item::Gap { node } => service.extent_in(root, node, width) as f32,
            _ => 0.0,
        };
        columns.push(paginate::window(&inner.items, &inner.leaves, BreakLevel::Column, Some(id), start, end, gap_extent));
    }
    let variants = found.choices.iter().map(|choices| ways_of(&inner, choices)).collect();
    let plan = Rc::new(ColumnPlan { width, gap, height: found.height, columns, variants, used: found.used.clone() });
    let key = (from, page_inline.to_bits(), available.to_bits(), found.end);
    if let Some(columns) = service.tree.node_mut(id).columns.as_mut() {
        columns.fragments.insert(key, plan.clone());
    }
    (plan, found)
}
