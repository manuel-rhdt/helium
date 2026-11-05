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
    LayoutGridContainer, LayoutPartialTree, ResolveOrZero, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_hidden_layout, compute_leaf_layout, compute_oof_layout,
};

use crate::layout::absolute::shrink_to_fit;
use crate::layout::components::{BlockLevelElement, IntrinsicSize, TableCellSpan, TextRun};
use crate::layout::inline::InlineFormattingContext;
use crate::layout::normal_formatting::BlockFormattingContext;
use crate::style::properties::computed::{ComputedValues, PixelLength};

use super::style::{is_out_of_flow, taffy_style};
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
    pub style: taffy::Style,
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
    pub cache: Cache,
    pub layout: Layout,
}

pub(super) struct HeliumTree<'a, 'w> {
    pub nodes: Vec<Node>,
    pub computed_values: &'a ReadStorage<'w, ComputedValues>,
    pub inline_ctxs: &'a mut WriteStorage<'w, InlineFormattingContext>,
    pub intrinsic_sizes: &'a mut WriteStorage<'w, IntrinsicSize>,
    pub text_runs: &'a ReadStorage<'w, TextRun>,
    pub table_spans: &'a ReadStorage<'w, TableCellSpan>,
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

    fn new_node(&mut self, entity: Entity, content: Content) -> NodeId {
        let style = self
            .computed_values
            .get(entity)
            .map(taffy_style)
            .unwrap_or_default();
        self.nodes.push(Node {
            entity,
            style,
            content,
            children: vec![],
            atoms: vec![],
            atoms_done: false,
            hoisted: vec![],
            table: None,
            grid_tracks: None,
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

    /// Turns a node into a box with inline content or a replaced box. The out-of-flow boxes of
    /// the inline content that are not children of the node yet become its children.
    fn make_leaf(&mut self, id: NodeId, block_ctxs: &ReadStorage<BlockFormattingContext>) {
        let entity = self.node(id).entity;
        let Some(ifc) = self.inline_ctxs.get(entity) else {
            // A box with an intrinsic size (such as a `<math>` element) is sized like a replaced
            // element: an `auto` width is its intrinsic width instead of the container's width.
            let replaced = self.intrinsic_sizes.get(entity).is_some();
            let node = self.node_mut(id);
            node.content = Content::Replaced;
            node.style.item_is_replaced = replaced;
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
        let style = &self.node(atom).style;
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

        let style = &self.node(atom).style;
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

        // (first baseline, last baseline) relative to the top of the content box
        let mut baselines = None;
        let mut output = compute_leaf_layout(inputs, &node.style, calc, |known, available| {
            let Some(ifc) = ifc else {
                return Size::ZERO;
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
        let intrinsic = self.intrinsic_sizes.get(node.entity).copied();
        let mut output = compute_leaf_layout(inputs, &node.style, calc, |known, _available| Size {
            width: known
                .width
                .unwrap_or(intrinsic.and_then(|s| s.width()).map_or(0.0, |w| w.0)),
            height: known
                .height
                .unwrap_or(intrinsic.and_then(|s| s.height()).map_or(0.0, |h| h.0)),
        });
        if let Some(IntrinsicSize::SizeWithBaseline { baseline, .. }) = intrinsic {
            output.baselines = Baselines::from_first(Some(baseline.0));
        }
        output
    }
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
            let mut output = match tree.node(id).content {
                Content::Blocks => match tree.node(id).style.display {
                    taffy::Display::Flex => compute_flexbox_layout(tree, id, inputs),
                    taffy::Display::Grid => compute_grid_layout(tree, id, inputs),
                    _ => compute_block_layout(tree, id, inputs, None),
                },
                Content::Inline => tree.compute_inline_layout(id, inputs),
                Content::Replaced => tree.compute_replaced_layout(id, inputs),
                Content::Table => tree.compute_table_layout(id, inputs),
            };
            // lay out the out-of-flow boxes this node is the containing block of
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut output);
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
        &self.node(id).style
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
        &self.node(id).style
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
        &self.node(id).style
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
        &self.node(id).style
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
