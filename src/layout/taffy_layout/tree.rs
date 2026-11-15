//! The box tree as seen by Taffy.
//!
//! The tree is an arena of [`Node`]s built from helium's formatting contexts at the start of
//! every layout. Taffy's low-level traits are implemented on [`HeliumTree`], which also gives
//! the measure functions access to the inline formatting contexts.

use specs::{Entity, ReadStorage, WriteStorage};
use taffy::geometry::{Line, Point, Size};
use taffy::style::{AvailableSpace, Position};
use taffy::tree::{
    AxisStaticEdge, AxisStaticPosition, Baselines, Cache, Layout, LayoutInput, LayoutOutput,
    NodeId, OofCandidate, OofPositioningArea, RequestedAxis, RunMode, SizingMode,
};
use taffy::{
    CacheTree, LayoutBlockContainer, LayoutContainingBlock, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutPartialTree, MaybeResolve, ResolveOrZero, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_leaf_layout, compute_oof_layout,
};

use crate::layout::absolute::shrink_to_fit;
use crate::layout::components::{
    BlockLevelElement, ImageBox, IntrinsicSize, TableCellSpan, TableColumns, TextRun,
};
use crate::layout::inline::InlineFormattingContext;
use crate::layout::normal_formatting::BlockFormattingContext;
use crate::style::properties::computed::{ComputedValues, PixelLength};

use super::style::{is_out_of_flow, taffy_style_in};
use crate::layout::writing_mode::Flow;
use super::table::{GridTracks, TableStructure};
use taffy::DetailedGridInfo;

/// What is inside of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Content {
    /// Block-level boxes: the in-flow children of the node.
    Blocks,
    /// Inline content: the box has an [`InlineFormattingContext`], which is measured and broken
    /// into lines as a whole.
    Inline,
    /// A box without children whose size is its [`IntrinsicSize`], if it has one.
    Replaced,
    /// A table, laid out as a grid of its cells (see [`super::table`]). The children of the node
    /// are the grid items.
    Table,
}

#[derive(Debug)]
pub(super) struct Node {
    pub entity: Entity,
    /// The style in the logical axes of the flow of the node (see `flow`): the one that the
    /// box is laid out with, as a container and as an item of a container of the same flow.
    pub style: taffy::Style,
    /// The flow of the content of the box: `writing-mode`.
    pub flow: Flow,
    /// Whether the flow of the box is vertical when the one of its parent is horizontal, or the
    /// other way round: an orthogonal flow. It is laid out on its own, with the available space
    /// of its parent turned around, and its parent sees it with `item_style`.
    pub orthogonal: bool,
    /// The style of an orthogonal flow in the logical axes of its parent.
    pub item_style: Option<taffy::Style>,
    pub content: Content,
    /// The children of the node in document order, including the out-of-flow ones. A node with
    /// inline content only has out-of-flow children.
    pub children: Vec<NodeId>,
    /// The `inline-block` boxes in the inline content of this node. They are not children of
    /// the node but laid out on their own (see [`HeliumTree::layout_atom`]).
    pub atoms: Vec<NodeId>,
    /// Whether the sizes of the `inline-block` boxes were given to the inline context.
    pub atoms_done: bool,
    /// The out-of-flow boxes this node is the containing block of.
    pub hoisted: Vec<NodeId>,
    /// How the boxes of a table are arranged in its grid.
    pub table: Option<Box<TableStructure>>,
    /// The tracks of a grid container after layout.
    pub grid_tracks: Option<GridTracks>,
    /// Whether the in-flow children are laid out from right to left: the node is a block, flex or
    /// grid container or a table with `direction: rtl`. Taffy only lays out left to right, so the
    /// layout is mirrored afterwards (see [`HeliumTree::mirror_children`]).
    pub mirror_rtl: bool,
    /// The borders of a cell, row, row group or table of a table with collapsing borders (see
    /// [`super::collapse`]).
    pub collapsed: Option<crate::layout::components::EdgeValues<crate::layout::components::CollapsedEdge>>,
    pub cache: Cache,
    pub layout: Layout,
}

pub(super) struct HeliumTree<'a, 'w> {
    pub nodes: Vec<Node>,
    pub computed_values: &'a ReadStorage<'w, ComputedValues>,
    pub inline_ctxs: &'a mut WriteStorage<'w, InlineFormattingContext>,
    pub intrinsic_sizes: &'a mut WriteStorage<'w, IntrinsicSize>,
    pub images: &'a ReadStorage<'w, ImageBox>,
    pub text_runs: &'a ReadStorage<'w, TextRun>,
    pub table_spans: &'a ReadStorage<'w, TableCellSpan>,
    /// The `<col>` and `<colgroup>` elements of each table, in column order.
    pub table_columns: &'a ReadStorage<'w, TableColumns>,
    /// The line width to use for the `inline-block` boxes of an inline context if its width is
    /// not known when it is first measured.
    pub fallback_line_width: f32,
}

// --- building the tree ---

impl<'a, 'w> HeliumTree<'a, 'w> {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[usize::from(id)]
    }

    pub(super) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[usize::from(id)]
    }

    /// The style of a box as an item of its parent: in the logical axes of the parent.
    fn item_style(&self, id: NodeId) -> &taffy::Style {
        let node = self.node(id);
        node.item_style.as_ref().unwrap_or(&node.style)
    }

    /// Finds the orthogonal flows in the tree below `id` (which is laid out in `parent`'s flow):
    /// a box whose flow is vertical in a horizontal one, or horizontal in a vertical one.
    pub(super) fn mark_orthogonal(&mut self, id: NodeId, parent: Flow) {
        let flow = self.node(id).flow;
        if flow.is_vertical() != parent.is_vertical() {
            let entity = self.node(id).entity;
            let item = self
                .computed_values
                .get(entity)
                .map(|style| taffy_style_in(style, parent));
            let node = self.node_mut(id);
            node.orthogonal = true;
            node.item_style = item;
        }
        for child in self.node(id).children.clone() {
            self.mark_orthogonal(child, flow);
        }
        // the `inline-block` boxes of the lines of the box
        for atom in self.node(id).atoms.clone() {
            self.mark_orthogonal(atom, flow);
        }
    }

    pub(super) fn new_node(&mut self, entity: Entity, content: Content) -> NodeId {
        let flow = self
            .computed_values
            .get(entity)
            .map_or(Flow::default(), |style| Flow::from(style.text.writing_mode));
        let style = self
            .computed_values
            .get(entity)
            .map(|style| taffy_style_in(style, flow))
            .unwrap_or_default();
        self.nodes.push(Node {
            entity,
            style,
            flow,
            orthogonal: false,
            item_style: None,
            content,
            children: vec![],
            atoms: vec![],
            atoms_done: false,
            hoisted: vec![],
            table: None,
            grid_tracks: None,
            mirror_rtl: false,
            collapsed: None,
            cache: Cache::new(),
            layout: Layout::new(),
        });
        NodeId::from(self.nodes.len() - 1)
    }

    /// Builds the subtree of a box that is laid out on its own: the root, an absolutely
    /// positioned box or an `inline-block` box.
    pub fn build_box(
        &mut self,
        entity: Entity,
        block_ctxs: &ReadStorage<BlockFormattingContext>,
    ) -> NodeId {
        if let Some(bfc) = block_ctxs.get(entity) {
            if let Some(id) = self.build_from_elements(&bfc.elements, block_ctxs) {
                return id;
            }
        }
        let id = self.new_node(entity, Content::Replaced);
        self.make_leaf(id, block_ctxs);
        id
    }

    /// Builds the tree of a flat block formatting context, which starts with the start of the
    /// box establishing it and ends with its end.
    fn build_from_elements(
        &mut self,
        elements: &[BlockLevelElement],
        block_ctxs: &ReadStorage<BlockFormattingContext>,
    ) -> Option<NodeId> {
        let mut root = None;
        let mut stack: Vec<NodeId> = vec![];
        for &element in elements {
            match element {
                BlockLevelElement::ContainerStart(entity) => {
                    let id = self.new_node(entity, Content::Blocks);
                    match stack.last() {
                        Some(&parent) => self.node_mut(parent).children.push(id),
                        None => root = Some(id),
                    }
                    stack.push(id);
                }
                BlockLevelElement::ContainerEnd(_) => {
                    if let Some(id) = stack.pop() {
                        let is_table = self
                            .computed_values
                            .get(self.node(id).entity)
                            .is_some_and(|style| style.box_.display.is_table());
                        if is_table {
                            self.build_table(id);
                        }
                        self.prepare_right_to_left(id);
                    }
                }
                BlockLevelElement::Leaf(entity) => {
                    let Some(&parent) = stack.last() else {
                        continue;
                    };
                    if self.node(parent).entity == entity {
                        self.make_leaf(parent, block_ctxs);
                    } else {
                        log::error!("leaf {:?} is not the box it is in", entity);
                    }
                }
                BlockLevelElement::Absolute(entity) => {
                    let id = self.build_box(entity, block_ctxs);
                    match stack.last() {
                        Some(&parent) => self.node_mut(parent).children.push(id),
                        None => {
                            log::error!("absolutely positioned box {:?} outside of any box", entity)
                        }
                    }
                }
            }
        }
        root
    }

    /// Prepares a right-to-left box with block-level children (a block, flex or grid container,
    /// or a table) for being mirrored after layout: margins are physical, so the left and right
    /// margins of its in-flow children are swapped, to end up on the correct side again when the
    /// layout is mirrored.
    ///
    /// For a block container, the mirroring puts a block narrower than the container at its
    /// right edge, as CSS 2 resolves over-constrained margins in right-to-left containers.
    /// (Floats would have to be mirrored back, but they are not supported.)
    fn prepare_right_to_left(&mut self, id: NodeId) {
        let node = self.node(id);
        let rtl = self.computed_values.get(node.entity).is_some_and(|style| {
            style.text.direction == crate::style::properties::specified::Direction::Rtl
        });
        let has_block_children = matches!(node.content, Content::Blocks | Content::Table);
        // in a vertical flow, `direction` would run the lines upwards, which is not supported
        if !rtl || !has_block_children || node.flow.is_vertical() {
            return;
        }
        self.node_mut(id).mirror_rtl = true;
        for child in self.node(id).children.clone() {
            let style = &mut self.node_mut(child).style;
            if style.position != taffy::Position::Absolute {
                std::mem::swap(&mut style.margin.left, &mut style.margin.right);
            }
            // the offset of a relatively positioned box is mirrored with the layout, too
            if style.position == taffy::Position::Relative {
                std::mem::swap(&mut style.inset.left, &mut style.inset.right);
            }
        }
    }

    /// Mirrors the in-flow children of a right-to-left container (see `Node::mirror_rtl`) in its
    /// content box, so that the first column or flex item is at the right. `width` is the width
    /// of the border box of the container and `parent_width` the width that percentages of its
    /// padding refer to.
    ///
    /// This is called right after the container is laid out, before its parent stores its
    /// layout, so the border and padding are resolved from the style.
    pub(super) fn mirror_children(&mut self, id: NodeId, width: f32, parent_width: Option<f32>) {
        if !self.node(id).mirror_rtl {
            return;
        }
        let style = &self.node(id).style;
        let resolve = |lp: taffy::style::LengthPercentage| {
            let raw = lp.into_raw();
            if raw.tag() == taffy::CompactLength::LENGTH_TAG {
                raw.value()
            } else if raw.tag() == taffy::CompactLength::PERCENT_TAG {
                raw.value() * parent_width.unwrap_or(0.0)
            } else {
                0.0
            }
        };
        let content_left = resolve(style.border.left) + resolve(style.padding.left);
        let content_right = width - resolve(style.border.right) - resolve(style.padding.right);
        for child in self.node(id).children.clone() {
            let node = self.node_mut(child);
            if node.style.position == taffy::Position::Absolute
                || node.style.display == taffy::Display::None
            {
                continue;
            }
            let x = node.layout.location.x;
            node.layout.location.x = content_left + content_right - x - node.layout.size.width;
            // the margins were swapped for the layout (`prepare_right_to_left`): the used values
            // are the ones of the sides that they are on now
            let margin = &mut node.layout.margin;
            std::mem::swap(&mut margin.left, &mut margin.right);
        }
    }

    /// Turns a node into a box with inline content or a replaced box. The out-of-flow boxes of
    /// the inline content that are not children of the node yet become its children.
    fn make_leaf(&mut self, id: NodeId, block_ctxs: &ReadStorage<BlockFormattingContext>) {
        let entity = self.node(id).entity;
        let Some(ifc) = self.inline_ctxs.get(entity) else {
            // A box with an intrinsic size (such as a `<math>` element) is sized like a replaced
            // element: an `auto` width is its intrinsic width instead of the container's width.
            let image = self.images.get(entity).map(|image| (image.width, image.height));
            let replaced = image.is_some() || self.intrinsic_sizes.get(entity).is_some();
            let node = self.node_mut(id);
            node.content = Content::Replaced;
            node.style.item_is_replaced = replaced;
            // An image keeps its proportions when only one of its sides is given.
            // (when both are given, the ratio has nothing to do: it would only raise the minimum
            // size that an `auto` minimum takes from it)
            // With nothing but the size of the image, the ratio would only be applied to the
            // border box, which has the padding and border in one axis only: a box that is wider
            // by the padding on the right would be taller by it too.
            // A length for one side of a content box gives the other side directly: taffy
            // would apply the ratio to the border box.
            if let Some((width, height)) = image.filter(|&(w, h)| w > 0.0 && h > 0.0) {
                if node.style.box_sizing == taffy::BoxSizing::ContentBox && !node.flow.is_vertical() {
                    let length = |dimension: taffy::Dimension| {
                        let raw = dimension.into_raw();
                        (raw.tag() == taffy::CompactLength::LENGTH_TAG).then(|| raw.value())
                    };
                    let (known_width, known_height) =
                        (length(node.style.size.width), length(node.style.size.height));
                    let (inline, block) = node.flow.swap((width, height));
                    match (known_width, known_height) {
                        (Some(w), None) if node.style.size.height.is_auto() => {
                            node.style.size.height = taffy::Dimension::length(w * block / inline);
                        }
                        (None, Some(h)) if node.style.size.width.is_auto() => {
                            node.style.size.width = taffy::Dimension::length(h * inline / block);
                        }
                        _ => {}
                    }
                }
            }
            let sizes_given = !node.style.size.width.is_auto() && !node.style.size.height.is_auto();
            let constrained = !node.style.size.width.is_auto()
                || !node.style.size.height.is_auto()
                || !node.style.min_size.width.is_auto()
                || !node.style.min_size.height.is_auto()
                || !node.style.max_size.width.is_auto()
                || !node.style.max_size.height.is_auto();
            let nonzero = |lp: taffy::style::LengthPercentage| {
                let raw = lp.into_raw();
                raw.tag() != taffy::CompactLength::LENGTH_TAG || raw.value() != 0.0
            };
            let has_edges = [node.style.padding, node.style.border].iter().any(|edges| {
                [edges.left, edges.right, edges.top, edges.bottom].into_iter().any(nonzero)
            });
            if let Some((width, height)) = image
                .filter(|&(w, h)| w > 0.0 && h > 0.0)
                .filter(|_| !sizes_given && (constrained || !has_edges))
            {
                // the ratio of the logical sides: turned around in a vertical flow
                let (inline, block) = node.flow.swap((width, height));
                node.style.aspect_ratio = Some(inline / block);
            }
            return;
        };
        let absolutes = ifc.absolutes.clone();
        let atoms: Vec<Entity> = ifc.atom_offsets.iter().map(|&(atom, _)| atom).collect();
        self.node_mut(id).content = Content::Inline;

        let existing: Vec<Entity> = self
            .node(id)
            .children
            .iter()
            .map(|&child| self.node(child).entity)
            .collect();
        for absolute in absolutes {
            if !existing.contains(&absolute) {
                let child = self.build_box(absolute, block_ctxs);
                self.node_mut(id).children.push(child);
            }
        }
        for atom in atoms {
            let atom_id = self.build_box(atom, block_ctxs);
            self.node_mut(id).atoms.push(atom_id);
        }
    }
}

// --- layout of the leaves ---

fn calc(_val: *const (), _basis: f32) -> f32 {
    0.0
}

impl HeliumTree<'_, '_> {
    /// Lays out the `inline-block` boxes of an inline context, once, and tells the context their
    /// widths, which it needs to break lines.
    fn ensure_atoms(&mut self, id: NodeId, inputs: &LayoutInput) {
        if self.node(id).atoms_done {
            return;
        }
        self.node_mut(id).atoms_done = true;
        let atoms = self.node(id).atoms.clone();
        if atoms.is_empty() {
            return;
        }
        let style = &self.node(id).style;
        let parent_width = inputs.parent_size.width;
        let inline_extras = (style.padding.resolve_or_zero(parent_width, calc)
            + style.border.resolve_or_zero(parent_width, calc))
        .horizontal_axis_sum();
        let line_width = inputs
            .known_dimensions
            .width
            .or(inputs.available_space.width.into_option())
            .map(|width| width - inline_extras)
            .unwrap_or(self.fallback_line_width)
            .max(0.0);

        for &atom in &atoms {
            self.layout_atom(atom, line_width);
        }

        let entity = self.node(id).entity;
        let widths = atoms
            .iter()
            .filter_map(|&atom| {
                let atom_entity = self.node(atom).entity;
                Some((atom_entity, self.intrinsic_sizes.get(atom_entity)?.width()?))
            })
            .collect();
        if let Some(ifc) = self.inline_ctxs.get_mut(entity) {
            ifc.apply_atom_widths(&widths);
        }
    }

    /// Lays out an `inline-block` box for a line of the given width and stores the size of its
    /// margin box and its baseline as its [`IntrinsicSize`].
    ///
    /// https://www.w3.org/TR/CSS22/visudet.html#inlineblock-width
    fn layout_atom(&mut self, atom: NodeId, line_width: f32) {
        let style = self.item_style(atom);
        // `auto` margins are 0
        let margin = style.margin.resolve_or_zero(Some(line_width), calc);
        let available = (line_width - margin.horizontal_axis_sum()).max(0.0);
        let width_is_auto = style.size.width.is_auto();

        let parent_size = Size {
            width: Some(line_width),
            height: None,
        };
        let size_input = |available_width| LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Horizontal,
            known_dimensions: Size::NONE,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size,
            available_space: Size {
                width: available_width,
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let width = if width_is_auto {
            let min = self
                .compute_child_layout(atom, size_input(AvailableSpace::MinContent))
                .size
                .width;
            let max = self
                .compute_child_layout(atom, size_input(AvailableSpace::MaxContent))
                .size
                .width;
            Some(shrink_to_fit(min, max, available))
        } else {
            None
        };

        let output = self.compute_child_layout(
            atom,
            LayoutInput {
                run_mode: RunMode::PerformLayout,
                axis: RequestedAxis::Both,
                known_dimensions: Size {
                    width,
                    height: None,
                },
                available_space: Size {
                    width: AvailableSpace::Definite(available),
                    height: AvailableSpace::MaxContent,
                },
                ..size_input(AvailableSpace::Definite(available))
            },
        );
        if !output.oof_candidates.is_empty() {
            log::warn!(
                "absolutely positioned boxes in an inline-block box without a containing block are not supported"
            );
        }

        let style = self.item_style(atom);
        let layout = Layout {
            location: Point {
                x: margin.left,
                y: margin.top,
            },
            size: output.size,
            border: style.border.resolve_or_zero(Some(line_width), calc),
            padding: style.padding.resolve_or_zero(Some(line_width), calc),
            margin,
            ..Layout::new()
        };
        self.node_mut(atom).layout = layout;

        let total_width = output.size.width + margin.horizontal_axis_sum();
        let total_height = output.size.height + margin.vertical_axis_sum();
        // the baseline is the baseline of the last line, or the bottom margin edge without lines
        let baseline = output
            .baselines
            .last
            .or(output.baselines.first)
            .map_or(total_height, |baseline| baseline + margin.top);
        let entity = self.node(atom).entity;
        let _ = self.intrinsic_sizes.insert(
            entity,
            IntrinsicSize::SizeWithBaseline {
                width: PixelLength(total_width),
                height: PixelLength(total_height),
                baseline: PixelLength(baseline),
            },
        );
    }

    /// Lays out a box with inline content: its content size is the size of its lines.
    fn compute_inline_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.ensure_atoms(id, &inputs);

        let node = &self.nodes[usize::from(id)];
        let entity = node.entity;
        let ifc = self.inline_ctxs.get(entity);
        let text_runs = self.text_runs;
        let intrinsic_sizes = &*self.intrinsic_sizes;
        let text_indent = self.computed_values.get(entity).map(|s| s.text.text_indent);
        let run_mode = inputs.run_mode;
        // Taffy gives the measure function a known size of the border box (`known`) but the
        // available space of the content box, so the lines must not be broken for `known`
        // itself: a cell that a grid track makes 16px wider than its content would otherwise
        // be measured with fewer lines than it gets when it is laid out for real.
        let horizontal_extras = {
            let parent_width = inputs.parent_size.width;
            let style = &node.style;
            (style.padding.resolve_or_zero(parent_width, calc)
                + style.border.resolve_or_zero(parent_width, calc))
            .horizontal_axis_sum()
        };

        // (first baseline, last baseline) relative to the top of the content box
        let mut baselines = None;
        let mut output = compute_leaf_layout(inputs, &node.style, calc, |known, available| {
            let Some(ifc) = ifc else {
                return Size::ZERO;
            };
            let known = Size {
                width: known.width.map(|width| (width - horizontal_extras).max(0.0)),
                height: known.height,
            };
            // Percentages of `text-indent` refer to the width of the box, which is not known
            // when the box is sized by its contents.
            let definite_width = known.width.or(available.width.into_option());
            let indent = match (text_indent, definite_width) {
                (Some(indent), Some(width)) => indent.absolute(PixelLength(width)),
                (Some(indent), None) => indent.length().unwrap_or_default(),
                (None, _) => PixelLength(0.0),
            };
            let (min, max) = ifc.intrinsic_widths(indent);
            // The width the lines are broken for. When Taffy lays out the box for real, the
            // definite available space is the width of its content box.
            let line_width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) if run_mode == RunMode::PerformLayout => width,
                AvailableSpace::Definite(width) => shrink_to_fit(min.0, max.0, width),
                AvailableSpace::MinContent => min.0,
                AvailableSpace::MaxContent => max.0,
            });
            // The width of the content: a box that is sized by its contents is not wider than
            // its longest line
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => shrink_to_fit(min.0, max.0, width),
                _ => line_width,
            });
            let lines = ifc
                .compute_lines(PixelLength(line_width), indent, text_runs, intrinsic_sizes)
                .unwrap_or_else(|error| {
                    log::error!("cannot break lines of {:?}: {}", entity, error);
                    vec![]
                });
            let height: f32 = lines.iter().map(|line| line.height.0).sum();
            if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
                baselines = Some((first.baseline.0, height - last.height.0 + last.baseline.0));
            }
            Size {
                width,
                height: known.height.unwrap_or(height),
            }
        });

        let style = &node.style;
        let parent_width = inputs.parent_size.width;
        let border = style.border.resolve_or_zero(parent_width, calc);
        let padding = style.padding.resolve_or_zero(parent_width, calc);
        if let Some((first, last)) = baselines {
            let top = border.top + padding.top;
            output.baselines = Baselines {
                first: Some(top + first),
                last: Some(top + last),
            };
        }

        if inputs.run_mode == RunMode::PerformLayout && !node.children.is_empty() {
            // The out-of-flow boxes in inline content are placed as if they were at the start
            // of the content.
            // TODO: use the position in the line where the box was
            let content_left = border.left + padding.left;
            let content_top = border.top + padding.top;
            for (order, &child) in node.children.iter().enumerate() {
                output.oof_candidates.push(OofCandidate {
                    node: child,
                    order: order as u32,
                    position: Position::Absolute,
                    static_position: Point {
                        x: AxisStaticPosition::from_edge(content_left, AxisStaticEdge::Start),
                        y: AxisStaticPosition::from_edge(content_top, AxisStaticEdge::Start),
                    },
                });
            }
            output.oof_positioning_area = Some(OofPositioningArea {
                size: Size {
                    width: output.size.width - border.horizontal_axis_sum(),
                    height: output.size.height - border.vertical_axis_sum(),
                },
                offset: Point {
                    x: border.left,
                    y: border.top,
                },
            });
        }
        output
    }

    fn compute_replaced_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        let node = &self.nodes[usize::from(id)];
        // An image is as big as the image, however often it was an inline-block atom before (which
        // stores the size of its margin box as its `IntrinsicSize`).
        let image = self.images.get(node.entity).map(|image| (image.width, image.height));
        // An image without a given size is as big as its pixels, but within `min-` and
        // `max-width` and `-height`, which then change the other side too (CSS 2.1 10.4: the
        // size keeps its proportions).
        let image = image.map(|(width, height)| {
            let parent = inputs.parent_size;
            let style = &node.style;
            let resolve = |dimension: taffy::LengthPercentageAuto, basis: Option<f32>| {
                dimension.maybe_resolve(basis, calc)
            };
            let min_width = resolve(style.min_size.width, parent.width).unwrap_or(0.0);
            let max_width = resolve(style.max_size.width, parent.width).unwrap_or(f32::INFINITY);
            let min_height = resolve(style.min_size.height, parent.height).unwrap_or(0.0);
            let max_height = resolve(style.max_size.height, parent.height).unwrap_or(f32::INFINITY);
            if width <= 0.0 || height <= 0.0 {
                return (width, height);
            }
            let ratio = width / height;
            let mut w = width.clamp(min_width.min(max_width), max_width);
            let h = (w / ratio).clamp(min_height.min(max_height), max_height);
            if (w / h - ratio).abs() > 1e-3 {
                // the height was limited: the width follows
                w = (h * ratio).clamp(min_width.min(max_width), max_width);
            }
            (w, h)
        });
        // the size of an image is physical: in a vertical flow the width is the block size
        let image = image.map(|size| node.flow.swap(size));
        let intrinsic = if image.is_some() {
            None
        } else {
            self.intrinsic_sizes.get(node.entity).copied()
        };
        let mut output = compute_leaf_layout(inputs, &node.style, calc, |known, _available| Size {
            width: known.width.unwrap_or(
                image.map_or_else(|| intrinsic.and_then(|s| s.width()).map_or(0.0, |w| w.0), |i| i.0),
            ),
            height: known.height.unwrap_or(
                image.map_or_else(|| intrinsic.and_then(|s| s.height()).map_or(0.0, |h| h.0), |i| i.1),
            ),
        });
        if let Some(IntrinsicSize::SizeWithBaseline { baseline, .. }) = intrinsic {
            output.baselines = Baselines::from_first(Some(baseline.0));
        }
        output
    }
}

/// The input of the layout of an orthogonal flow, seen from the flow itself: the axes turned
/// around.
fn transpose_input(inputs: LayoutInput) -> LayoutInput {
    fn swap<T>(size: Size<T>) -> Size<T> {
        Size { width: size.height, height: size.width }
    }
    LayoutInput {
        run_mode: inputs.run_mode,
        sizing_mode: inputs.sizing_mode,
        axis: match inputs.axis {
            RequestedAxis::Horizontal => RequestedAxis::Vertical,
            RequestedAxis::Vertical => RequestedAxis::Horizontal,
            RequestedAxis::Both => RequestedAxis::Both,
        },
        known_dimensions: swap(inputs.known_dimensions),
        known_dimensions_are_definite: swap(inputs.known_dimensions_are_definite),
        parent_size: swap(inputs.parent_size),
        available_space: swap(inputs.available_space),
        // an orthogonal flow is a formatting context of its own: nothing collapses through it
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

/// The result of the layout of an orthogonal flow, seen from its parent.
fn transpose_output(output: &mut LayoutOutput) {
    output.size = Size { width: output.size.height, height: output.size.width };
    // the baselines and the margins are in the axes of the flow
    output.baselines = Baselines::NONE;
    output.top_margin = taffy::CollapsibleMarginSet::ZERO;
    output.bottom_margin = taffy::CollapsibleMarginSet::ZERO;
    output.margins_can_collapse_through = false;
}

// --- Taffy traits ---

pub(super) struct ChildIter<'t>(std::slice::Iter<'t, NodeId>);

impl Iterator for ChildIter<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        self.0.next().copied()
    }
}

impl TraversePartialTree for HeliumTree<'_, '_> {
    type ChildIter<'t>
        = ChildIter<'t>
    where
        Self: 't;

    fn child_ids(&self, parent: NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.node(parent).children.iter())
    }

    fn child_count(&self, parent: NodeId) -> usize {
        self.node(parent).children.len()
    }

    fn get_child_id(&self, parent: NodeId, index: usize) -> NodeId {
        self.node(parent).children[index]
    }
}

impl TraverseTree for HeliumTree<'_, '_> {}

impl LayoutPartialTree for HeliumTree<'_, '_> {
    type CoreContainerStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;
    type CustomIdent = String;

    fn get_core_container_style(&self, id: NodeId) -> Self::CoreContainerStyle<'_> {
        &self.node(id).style
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &Layout) {
        self.node_mut(id).layout = *layout;
    }

    fn compute_child_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        if self.node(id).style.display == taffy::Display::None {
            return compute_hidden_layout(self, id);
        }
        compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            // An orthogonal flow is laid out in its own axes: its parent's available space is
            // turned around, and so is the result.
            let orthogonal = tree.node(id).orthogonal;
            let inputs = if orthogonal { transpose_input(inputs) } else { inputs };
            let mut output = match tree.node(id).content {
                Content::Blocks => {
                    let output = match tree.node(id).style.display {
                        taffy::Display::Flex => compute_flexbox_layout(tree, id, inputs),
                        taffy::Display::Grid => compute_grid_layout(tree, id, inputs),
                        _ => compute_block_layout(tree, id, inputs, None),
                    };
                    if inputs.run_mode == RunMode::PerformLayout {
                        tree.mirror_children(id, output.size.width, inputs.parent_size.width);
                    }
                    output
                }
                Content::Inline => tree.compute_inline_layout(id, inputs),
                Content::Replaced => tree.compute_replaced_layout(id, inputs),
                Content::Table => tree.compute_table_layout(id, inputs),
            };
            // lay out the out-of-flow boxes this node is the containing block of
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut output);
            }
            if orthogonal {
                transpose_output(&mut output);
            }
            output
        })
    }
}

impl LayoutContainingBlock for HeliumTree<'_, '_> {
    type OofItemStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;

    fn get_oof_item_style(&self, id: NodeId) -> Self::OofItemStyle<'_> {
        self.item_style(id)
    }

    fn clear_hoisted_children(&mut self, id: NodeId) {
        self.node_mut(id).hoisted.clear();
    }

    fn add_hoisted_children(&mut self, id: NodeId, hoisted: &[NodeId]) {
        self.node_mut(id).hoisted.extend_from_slice(hoisted);
    }
}

impl CacheTree for HeliumTree<'_, '_> {
    fn cache_get(&mut self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.node_mut(id).cache.get(input)
    }

    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.node_mut(id).cache.store(input, output)
    }

    fn cache_clear(&mut self, id: NodeId) {
        self.node_mut(id).cache.clear();
    }
}

impl LayoutBlockContainer for HeliumTree<'_, '_> {
    type BlockContainerStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;
    type BlockItemStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;

    fn get_block_container_style(&self, id: NodeId) -> Self::BlockContainerStyle<'_> {
        &self.node(id).style
    }

    fn get_block_child_style(&self, id: NodeId) -> Self::BlockItemStyle<'_> {
        self.item_style(id)
    }
}

impl LayoutFlexboxContainer for HeliumTree<'_, '_> {
    type FlexboxContainerStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;
    type FlexboxItemStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;

    fn get_flexbox_container_style(&self, id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.node(id).style
    }

    fn get_flexbox_child_style(&self, id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.item_style(id)
    }
}

impl LayoutGridContainer for HeliumTree<'_, '_> {
    type GridContainerStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;
    type GridItemStyle<'t>
        = &'t taffy::Style
    where
        Self: 't;

    fn get_grid_container_style(&self, id: NodeId) -> Self::GridContainerStyle<'_> {
        &self.node(id).style
    }

    fn get_grid_child_style(&self, id: NodeId) -> Self::GridItemStyle<'_> {
        self.item_style(id)
    }

    fn set_detailed_grid_info(&mut self, id: NodeId, info: DetailedGridInfo<String>) {
        self.node_mut(id).grid_tracks = Some(GridTracks {
            rows: info.rows.positions,
            columns: info.columns.positions,
        });
    }
}

/// Whether a node is taken out of the flow.
pub(super) fn node_is_out_of_flow(tree: &HeliumTree, id: NodeId) -> bool {
    tree.computed_values
        .get(tree.node(id).entity)
        .is_some_and(is_out_of_flow)
}
