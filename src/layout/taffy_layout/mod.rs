//! Box layout with [Taffy](https://github.com/DioxusLabs/taffy).
//!
//! Taffy computes the sizes and positions of all boxes (block, flex and grid containers and
//! absolutely positioned boxes). Helium keeps doing what Taffy cannot do: inline content is
//! broken into lines by [`InlineFormattingContext`]. The pages are chosen by the kernel of
//! `helium_paginate`, and every page is laid out for its own geometry (see `docs/pagination.md`).
//!
//! The result is written to the same components the rest of the layout pipeline reads:
//! [`Position`] (content box in document coordinates), [`Decoration`], the [`TextList`] of the
//! paginator and the [`AbsoluteGroups`].

mod collapse;
mod flow;
mod paginate;
mod replaced;
mod service;
mod style;
mod subtree;
mod table;
mod tree;
mod window;

use std::collections::HashMap;

use specs::{Entities, Entity, ReadStorage, World, WorldExt, WriteStorage};
use taffy::geometry::{Point, Size};
use taffy::style::AvailableSpace;
use taffy::tree::NodeId;

use crate::layout::components::{
    AbsoluteGroup, AbsoluteGroups, BlockLevelElement, Decoration, EdgeValues, ImageBox, IntrinsicSize,
    Position, TableCellSpan, TableColumns, TextList, TextRun, ZIndex,
};
use crate::layout::inline::InlineFormattingContext;
use crate::layout::inline_size::decoration_colors;
use crate::layout::normal_formatting::BlockFormattingContext;
use crate::style::properties::computed::{ComputedValues, PixelLength, TextAlign};

use crate::layout::writing_mode::Flow;
use tree::{Content, HeliumTree, Node, lines_between};
use window::Window;

/// Distance between the z indices of boxes at different levels of nested absolutely positioned
/// boxes. Absolutely positioned boxes are painted on top of the in-flow content.
const ABS_Z_STEP: i64 = 100000;

/// A relatively positioned box is painted in the layer of the positioned boxes, on top of the
/// in-flow content that follows it. The depth of the boxes in it adds to the step, so it has to
/// be bigger than the depth of any tree, and it is smaller than [`ABS_Z_STEP`] to leave room
/// for the absolutely positioned boxes above.
const REL_Z_STEP: i64 = 1000;

/// Hooks for the tests of the layout service: a test can ask the service questions in front of
/// the pagination and behind it (before the pages are laid out), in the middle of a real layout.
#[cfg(test)]
pub(super) mod probe {
    use super::service::TreeService;
    use std::cell::RefCell;

    pub type Hook = Box<dyn FnOnce(&mut TreeService<'_, '_, '_>)>;

    thread_local! {
        pub static BEFORE: RefCell<Option<Hook>> = const { RefCell::new(None) };
        pub static AFTER: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub fn call(which: &'static std::thread::LocalKey<RefCell<Option<Hook>>>, service: &mut TreeService<'_, '_, '_>) {
        if let Some(hook) = which.with(|hook| hook.borrow_mut().take()) {
            hook(service);
        }
    }
}

pub struct TaffyBlockLayout {
    pub root_entity: Entity,
    pub page_width: PixelLength,
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

/// The part of a box on one page.
struct Fragment {
    position: Position,
    decoration: Decoration,
}

impl TaffyBlockLayout {
    pub fn run(self, world: &mut World) -> Result<(), String> {
        world.register::<Position>();
        world.register::<Decoration>();
        world.register::<IntrinsicSize>();
        world.register::<ImageBox>();
        world.register::<BlockFormattingContext>();
        world.register::<InlineFormattingContext>();
        world.register::<TextRun>();
        world.register::<crate::layout::components::VerticalAlignment>();
        world.register::<crate::layout::components::CanvasSize>();
        world.register::<TableCellSpan>();
        world.register::<TableColumns>();
        world.register::<crate::layout::components::Fragmented>();
        world.register::<crate::layout::components::DecorationFragment>();
        world.entry::<TextList>().or_insert_with(Default::default);
        world
            .entry::<crate::layout::pages::PageMargins>()
            .or_insert_with(Default::default);
        world
            .entry::<crate::layout::pages::Pages>()
            .or_insert_with(Default::default);
        world
            .entry::<AbsoluteGroups>()
            .or_insert_with(Default::default);

        let entities = world.entities();
        let computed_values = world.read_storage::<ComputedValues>();
        let block_ctxs = world.read_storage::<BlockFormattingContext>();
        let mut inline_ctxs = world.write_storage::<InlineFormattingContext>();
        let mut intrinsic_sizes = world.write_storage::<IntrinsicSize>();
        let images = world.read_storage::<ImageBox>();
        let canvases = world.read_storage::<crate::layout::components::CanvasSize>();
        let page_margins = world.read_resource::<crate::layout::pages::PageMargins>();
        let text_runs = world.read_storage::<TextRun>();
        let aligns = world.read_storage::<crate::layout::components::VerticalAlignment>();
        let table_spans = world.read_storage::<TableCellSpan>();
        let table_columns = world.read_storage::<TableColumns>();

        let content_width = (self.page_width - self.margin.left - self.margin.right).0;
        let mut tree = HeliumTree {
            nodes: vec![],
            computed_values: &computed_values,
            inline_ctxs: &mut inline_ctxs,
            intrinsic_sizes: &mut intrinsic_sizes,
            images: &images,
            canvases: &canvases,
            text_runs: &text_runs,
            aligns: &aligns,
            table_spans: &table_spans,
            table_columns: &table_columns,
            fallback_line_width: content_width.max(0.0),
            window: None,
            windowed: vec![],
            lines: HashMap::new(),
        };
        let root = tree.build_box(self.root_entity, &block_ctxs);
        tree.mark_orthogonal(root, crate::layout::writing_mode::Flow::HorizontalTb);
        let out_of_flow: Vec<bool> = (0..tree.nodes.len())
            .map(|index| tree::node_is_out_of_flow(&tree, NodeId::from(index)))
            .collect();

        // the flow of the document, and the geometry of the page at every place of it
        let flow = flow::extract(&mut tree, root, page_margins.rtl);
        let geometry = paginate::Geometry {
            margins: &page_margins,
            size: (self.page_width, self.page_height),
            margin: self.margin,
        };
        let classes = std::sync::Arc::new(paginate::Classes::new(&tree, &flow));
        let first_class = classes.of(helium_paginate::Position::Item(0)).clone();
        let first_spec = geometry.spec(&geometry.context(0, &first_class));
        let problem = helium_paginate::Problem::new(geometry.table(classes.clone()), flow.items.clone());

        // choose the pages
        let before = std::time::Instant::now();
        let mut service = service::TreeService::new(&mut tree, &flow, first_spec.block_size);
        #[cfg(test)]
        probe::call(&probe::BEFORE, &mut service);
        // (start of the page, its end, the page)
        let mut planned: Vec<(helium_paginate::PageStart, helium_paginate::Position, helium_paginate::Position, helium_paginate::PageFill)> = vec![];
        {
            use helium_paginate::Solver;
            let mut kernel = helium_paginate::Kernel::new(&problem, &mut service);
            let solution = helium_paginate::ExactSolver::new(helium_paginate::CssObjective).solve(&mut kernel);
            let fills = kernel.realize(&solution).map_err(|error| format!("invalid pagination: {error:?}"))?;
            let end = helium_paginate::Position::Item(flow.items.len());
            for (page, fill) in fills.iter().enumerate() {
                let start = solution.pages[page];
                let finish = kernel.position(fill.end);
                planned.push((start, kernel.position(start.at), finish, *fill));
            }
            if fills.is_empty() {
                planned.push((
                    solution.pages[0],
                    helium_paginate::Position::Item(0),
                    end,
                    helium_paginate::PageFill {
                        end: solution.pages[0].at,
                        spec: first_spec,
                        kind: helium_paginate::PageKind::First,
                        used: 0.0,
                        end_kind: helium_paginate::BreakKind::Allowed,
                        orphan_violation: false,
                        widow_violation: false,
                        overflow: false,
                    },
                ));
            }
        }
        #[cfg(test)]
        probe::call(&probe::AFTER, &mut service);
        log::info!("TIMED: paginate {:?}", before.elapsed());
        log::debug!("{} pages, laid out for {} widths", planned.len(), service.galleys());
        let before = std::time::Instant::now();

        let mut page_contexts: Vec<crate::layout::pages::PageContext> = vec![];
        let mut fragments: HashMap<Entity, Vec<Fragment>> = HashMap::new();
        let mut positions = world.write_storage::<Position>();
        let mut decorations = world.write_storage::<Decoration>();
        let mut text_list_resource = world.write_resource::<TextList>();
        let mut groups_resource = world.write_resource::<AbsoluteGroups>();
        for (start, from, to, fill) in &planned {
            // the pages that a break to a left or right page left empty
            while page_contexts.len() < start.index {
                let index = page_contexts.len();
                page_contexts.push(crate::layout::pages::PageContext {
                    index,
                    name: None,
                    first_in_group: false,
                    rtl: page_margins.rtl,
                });
            }
            let class = if start.index == 0 {
                let mut class = classes.of(*from).clone();
                class.1 = true;
                class
            } else {
                classes.of(*from).clone()
            };
            let context = geometry.context(start.index, &class);
            let (edges, _) = geometry.of_page(&context);
            let spec = geometry.spec(&context);
            let window = paginate::window(&flow, *from, *to, |item| match flow.items[item] {
                helium_paginate::Item::Gap { node } => {
                    helium_paginate::LayoutService::extent(&mut service, node, spec.inline_size) as f32
                }
                _ => 0.0,
            });
            let tree = &mut *service.tree;
            tree.set_window(window, root);
            taffy::compute_root_layout(
                tree,
                root,
                Size {
                    width: AvailableSpace::Definite(spec.inline_size),
                    height: AvailableSpace::Definite(spec.block_size),
                },
            );
            let height = tree.node(root).layout.size.height;
            if !fill.overflow && (height - fill.used).abs() > 0.5 {
                log::warn!(
                    "page {} is {height} high, but the paginator counted {}",
                    start.index + 1,
                    fill.used
                );
            }
            let HeliumTree { nodes, window, inline_ctxs, lines, text_runs, aligns, intrinsic_sizes, .. } = tree;
            let mut writer = Writer {
                nodes,
                out_of_flow: &out_of_flow,
                entities: &entities,
                computed_values: &computed_values,
                inline_ctxs,
                text_runs,
                aligns,
                intrinsic_sizes,
                line_cache: lines,
                positions: &mut positions,
                decorations: &mut decorations,
                text_list: &mut text_list_resource.list,
                groups: &mut groups_resource.groups,
                group_stack: vec![],
                last_flow_entity: None,
                window: window.as_ref().expect("the window is set"),
                page: start.index,
                page_bottom: edges.top.0 + spec.block_size,
                fragments: &mut fragments,
            };
            // The root is placed at the top left corner of the content area of the page.
            writer.write_box(
                root,
                Point {
                    x: edges.left.0,
                    y: edges.top.0,
                },
                Flow::HorizontalTb,
                1,
                0,
            )?;
            page_contexts.push(context);
        }
        log::info!("TIMED: layout of the pages {:?}", before.elapsed());
        drop(service);
        drop(tree);

        // Boxes that are on more than one page are split: every page gets the part of the
        // decoration that is on that page.
        let mut fragmented_storage = world.write_storage::<crate::layout::components::Fragmented>();
        let mut fragment_storage = world.write_storage::<crate::layout::components::DecorationFragment>();
        for (entity, parts) in fragments {
            if parts.len() < 2 {
                continue;
            }
            let first = &parts[0];
            let deco = first.decoration;
            let visible = deco.background_color.alpha > 0
                || deco.has_background_image
                || deco.border.top > PixelLength(0.0)
                || deco.border.bottom > PixelLength(0.0)
                || deco.border.left > PixelLength(0.0)
                || deco.border.right > PixelLength(0.0)
                || parts.iter().any(|part| {
                    part.decoration.border.top > PixelLength(0.0)
                        || part.decoration.border.bottom > PixelLength(0.0)
                });
            if !visible {
                continue;
            }
            fragmented_storage.insert(entity, crate::layout::components::Fragmented).unwrap();
            for part in &parts {
                entities
                    .build_entity()
                    .with(part.position, &mut positions)
                    .with(part.decoration, &mut decorations)
                    .with(crate::layout::components::DecorationFragment(entity), &mut fragment_storage)
                    .build();
            }
        }
        drop(fragmented_storage);
        drop(fragment_storage);
        drop(text_list_resource);
        drop(groups_resource);
        drop(positions);
        drop(decorations);
        drop(page_margins);
        drop(inline_ctxs);
        drop(intrinsic_sizes);
        *world.write_resource::<crate::layout::pages::Pages>() = crate::layout::pages::Pages(page_contexts);
        Ok(())
    }
}

struct Writer<'a, 'w> {
    nodes: &'a [Node],
    out_of_flow: &'a [bool],
    entities: &'a Entities<'w>,
    computed_values: &'a ReadStorage<'w, ComputedValues>,
    inline_ctxs: &'a mut WriteStorage<'w, InlineFormattingContext>,
    text_runs: &'a ReadStorage<'w, TextRun>,
    aligns: &'a ReadStorage<'w, crate::layout::components::VerticalAlignment>,
    intrinsic_sizes: &'a WriteStorage<'w, IntrinsicSize>,
    line_cache: &'a mut HashMap<(u32, Option<usize>, u32), std::rc::Rc<Vec<crate::layout::inline::MeasuredLine>>>,
    positions: &'a mut WriteStorage<'w, Position>,
    decorations: &'a mut WriteStorage<'w, Decoration>,
    text_list: &'a mut Vec<BlockLevelElement>,
    groups: &'a mut Vec<AbsoluteGroup>,
    /// The groups of absolutely positioned and `inline-block` boxes that are being written.
    /// Everything that gets a position while a group is open belongs to it and is not part of
    /// the main flow.
    group_stack: Vec<usize>,
    /// The last entity that was placed in the main flow.
    last_flow_entity: Option<Entity>,
    /// The part of the document that is on the page, and the number of the page.
    window: &'a Window,
    page: usize,
    /// The bottom of the content area of the page.
    page_bottom: f32,
    /// The parts of the boxes that are on more than one page.
    fragments: &'a mut HashMap<Entity, Vec<Fragment>>,
}

impl<'a> Writer<'a, '_> {
    /// The node `id`. The reference does not borrow `self`.
    fn node(&self, id: NodeId) -> &'a Node {
        &self.nodes[usize::from(id)]
    }

    fn set_position(&mut self, entity: Entity, x: f32, y: f32, width: f32, height: f32, z: i64) {
        if let Some(&group) = self.group_stack.last() {
            let entities = &mut self.groups[group].entities;
            if !entities.contains(&entity) {
                entities.push(entity);
            }
        }
        let _ = self.positions.insert(
            entity,
            Position {
                x: PixelLength(x),
                y: PixelLength(y),
                width: PixelLength(width.max(0.0)),
                height: PixelLength(height.max(0.0)),
                z: ZIndex(z),
                page: self.page,
            },
        );
    }

    /// The z index and the z index offset of a child of a box at `z`: a relatively positioned box
    /// starts a layer of its own above the in-flow content.
    fn child_z(&self, child: NodeId, z: i64, z_base: i64) -> (i64, i64) {
        if self.node(child).style.position == taffy::Position::Relative {
            (z_base + REL_Z_STEP + 1, z_base + REL_Z_STEP)
        } else {
            (z + 1, z_base)
        }
    }

    fn emit(&mut self, element: BlockLevelElement) {
        // absolutely positioned and inline-block boxes are not paginated with the main flow
        if self.group_stack.is_empty() {
            self.text_list.push(element);
            self.last_flow_entity = Some(element.entity());
        }
    }

    fn open_group(&mut self, owner: Option<Entity>) {
        self.groups.push(AbsoluteGroup {
            containing_block: None,
            entities: vec![],
            owner,
            parent: self.group_stack.last().copied(),
            anchor: None,
        });
        self.group_stack.push(self.groups.len() - 1);
    }

    /// Writes the layout of a box and everything in it that is on the page.
    ///   origin       the top left corner of the border box of the box on the page
    ///   parent_flow  the flow that the layout of the box (its size and its edges) is in: the
    ///                one of its parent
    ///   z            the z index of the box
    ///   z_base       the z index offset of the absolutely positioned box the box is in
    fn write_box(
        &mut self,
        id: NodeId,
        origin: Point<f32>,
        parent_flow: Flow,
        z: i64,
        z_base: i64,
    ) -> Result<(), String> {
        let node = self.node(id);
        let entity = node.entity;
        let layout = node.layout;
        let content = node.content;
        let in_flow_box = node.span.as_ref().filter(|span| !span.anchored && self.group_stack.is_empty());
        let open_start = in_flow_box.is_some_and(|span| self.window.open_at_start(span));
        let open_end = in_flow_box.is_some_and(|span| self.window.open_at_end(span));
        // A table or a row group is laid out as a whole: the part on the page is cut from it.
        let table_part = node
            .span
            .as_ref()
            .filter(|span| !span.split && !span.kids.is_empty() && content != Content::Inline);
        let (cut_top, cut_height) = match table_part {
            Some(_) => {
                let (top, bottom) =
                    window::table_fragment(self.nodes, self.window, node, layout.size.height);
                (top, Some(bottom - top))
            }
            None => (0.0, None),
        };

        // The layout is in the logical axes of the parent: the physical size and edges of the
        // box are found by turning them around if the parent is vertical.
        let (width, mut height) = parent_flow.swap((layout.size.width, layout.size.height));
        if let Some(cut) = cut_height {
            height = cut;
        }
        let margin = parent_flow.edges_to_physical(layout.margin);
        let border = parent_flow.edges_to_physical(layout.border);
        let padding = parent_flow.edges_to_physical(layout.padding);
        // The logical size and edges of the box itself, which its content is laid out in.
        let flow = node.flow;
        let inline_size = flow.swap((width, height));
        let frame = Frame { flow, origin, inline_size: inline_size.0, block_size: inline_size.1 };
        let logical_border = flow.edges_to_logical(border);
        let logical_padding = flow.edges_to_logical(padding);

        let content_x = origin.x + border.left + padding.left;
        let content_y = origin.y + border.top + padding.top;
        let content_width = width - (border.left + border.right) - (padding.left + padding.right);
        let content_height = height - (border.top + border.bottom) - (padding.top + padding.bottom);
        if !open_start {
            self.set_position(entity, content_x, content_y, content_width, content_height, z);
        }

        let mut decoration = self
            .computed_values
            .get(entity)
            .map(decoration_colors)
            .unwrap_or_default();
        let edges = |rect: taffy::geometry::Rect<f32>| EdgeValues {
            top: PixelLength(rect.top),
            left: PixelLength(rect.left),
            bottom: PixelLength(rect.bottom),
            right: PixelLength(rect.right),
        };
        decoration.margin = edges(margin);
        decoration.border = edges(border);
        decoration.padding = edges(padding);
        decoration.collapsed = node.collapsed;
        // A box that goes on over the page break has no border and padding at the break (they are
        // not in its layout) and no rounded corners there.
        if open_start {
            decoration.radius.top_left = Default::default();
            decoration.radius.top_right = Default::default();
            if let Some(edges) = &mut decoration.collapsed {
                edges.top.paint = false;
                edges.left.extend_start = PixelLength(0.0);
                edges.right.extend_start = PixelLength(0.0);
            }
        }
        if open_end {
            decoration.radius.bottom_left = Default::default();
            decoration.radius.bottom_right = Default::default();
            if let Some(edges) = &mut decoration.collapsed {
                edges.bottom.paint = false;
                edges.left.extend_end = PixelLength(0.0);
                edges.right.extend_end = PixelLength(0.0);
            }
        }
        if !open_start {
            let _ = self.decorations.insert(entity, decoration);
        }
        if open_start || open_end {
            // The part of a box that goes on on the next page reaches the bottom of the content
            // area of its page, also where the content on the page ends above it.
            let fragment_height = if open_end {
                content_height.max(self.page_bottom - content_y)
            } else {
                content_height
            };
            self.fragments.entry(entity).or_default().push(Fragment {
                position: Position {
                    x: PixelLength(content_x),
                    y: PixelLength(content_y),
                    width: PixelLength(content_width.max(0.0)),
                    height: PixelLength(fragment_height.max(0.0)),
                    z: ZIndex(z),
                    page: self.page,
                },
                decoration,
            });
        }

        if !open_start {
            self.emit(BlockLevelElement::ContainerStart(entity));
        }

        // An orthogonal flow is not split between pages: it is a leaf of the main flow, and its
        // content is placed with it once the page it is on is known.
        let anchored = node.orthogonal && content == Content::Blocks;
        if anchored {
            self.emit(BlockLevelElement::Leaf(entity));
            self.open_group(None);
            if let Some(&group) = self.group_stack.last() {
                self.groups[group].anchor = Some((entity, Point { x: content_x, y: content_y }));
            }
        }

        // the content of a table cell that `vertical-align` moves down
        let shift = node.content_shift;
        let kid_shift = shift - cut_top;
        match content {
            Content::Blocks if table_part.is_some() => {
                for &child in &table_part.unwrap().kids {
                    if !self.in_window(child) {
                        continue;
                    }
                    let mut child_origin = frame.child_origin(&self.node(child).layout);
                    child_origin.y += kid_shift + self.part_top(child);
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                }
            }
            Content::Blocks => {
                for &child in self.window.children(self.nodes, node) {
                    if self.out_of_flow[usize::from(child)]
                        || self.node(child).style.display == taffy::Display::None
                    {
                        continue;
                    }
                    let mut child_origin = frame.child_origin(&self.node(child).layout);
                    child_origin.y += shift;
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                }
            }
            Content::Inline => {
                // the `inline-block` boxes are written once, with the first part of the paragraph
                if !open_start {
                    for &atom in &node.atoms {
                        // The contents of an `inline-block` box are placed relative to the top
                        // left corner of its margin box. They are moved to the line the box is
                        // on later.
                        self.open_group(Some(self.node(atom).entity));
                        // the border box is inside of the margin box, which is placed on the line
                        let margin = flow.edges_to_physical(self.node(atom).layout.margin);
                        let location = Point { x: margin.left, y: margin.top };
                        let (atom_z, atom_base) = self.child_z(atom, z, z_base);
                        self.write_box(atom, location, flow, atom_z, atom_base)?;
                        self.group_stack.pop();
                    }
                }
                let content_offset = (
                    logical_border.left + logical_padding.left,
                    logical_border.top + logical_padding.top + shift,
                );
                let content_inline_size = frame.inline_size
                    - (logical_border.left + logical_border.right)
                    - (logical_padding.left + logical_padding.right);
                self.write_lines(id, &frame, content_offset, content_inline_size, z + 1)?;
            }
            Content::Replaced => {
                if self.intrinsic_sizes.get(entity).is_some() {
                    self.emit(BlockLevelElement::Leaf(entity));
                }
            }
            Content::Table => {
                // The captions, row groups and rows in document order. The rows and groups
                // contain their cells, which are placed relative to them.
                let parts: Vec<NodeId> = match table_part {
                    Some(span) => span.kids.iter().copied().filter(|&kid| self.in_window(kid)).collect(),
                    None => node.table.iter().flat_map(|table| table.boxes.iter().copied()).collect(),
                };
                for child in parts {
                    let mut child_origin = frame.child_origin(&self.node(child).layout);
                    child_origin.y += kid_shift + if table_part.is_some() { self.part_top(child) } else { 0.0 };
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                }
            }
        }

        if anchored {
            self.group_stack.pop();
        }
        if !open_end {
            self.emit(BlockLevelElement::ContainerEnd(entity));
        }

        // the absolutely positioned boxes this box is the containing block of
        for &child in &node.hoisted {
            self.open_group(None);
            let child_origin = frame.child_origin(&self.node(child).layout);
            let abs_base = z_base + ABS_Z_STEP;
            self.write_box(child, child_origin, flow, abs_base + 1, abs_base)?;
            self.group_stack.pop();
        }
        Ok(())
    }

    /// How far the top of the part of a row group that is on the page is below its top.
    fn part_top(&self, id: NodeId) -> f32 {
        let node = self.node(id);
        if node.span.as_ref().is_some_and(|span| !span.split && !span.kids.is_empty()) {
            window::table_fragment(self.nodes, self.window, node, node.layout.size.height).0
        } else {
            0.0
        }
    }

    /// Whether a box of the flow has items on the page.
    fn in_window(&self, id: NodeId) -> bool {
        self.node(id)
            .span
            .as_ref()
            .is_some_and(|span| span.start < self.window.end && span.end > self.window.start)
    }

    /// Places the lines of the inline content of a box that are on the page. The lines are laid
    /// out in the logical axes of `frame`: `content_offset` is the position of the content box
    /// relative to the border box and `width` its inline size.
    fn write_lines(
        &mut self,
        id: NodeId,
        frame: &Frame,
        content_offset: (f32, f32),
        width: f32,
        z: i64,
    ) -> Result<(), String> {
        let entity = self.node(id).entity;
        let style = self.computed_values.get(entity);
        let indent = style
            .map(|s| s.text.text_indent.absolute(PixelLength(width)).0)
            .unwrap_or(0.0);
        // what the measure function of Taffy broke the lines for
        let (line_width, indent) = self.node(id).measured.unwrap_or((width, indent));
        let cut = self.window.cut_of(id);
        // the indent is the one of the first line of the paragraph
        let indent = if cut.is_some_and(|(from, _)| from.is_some()) { 0.0 } else { indent };
        let Some(ifc) = self.inline_ctxs.get_mut(entity) else {
            return Ok(());
        };
        let measured = lines_between(
            self.line_cache,
            ifc,
            entity.id(),
            cut,
            PixelLength(line_width),
            PixelLength(indent),
            self.text_runs,
            self.intrinsic_sizes,
            self.aligns,
        );
        let first = ifc.push_lines(measured, self.entities);
        let align = ifc.resolved_text_align();
        let indent_offset = ifc.indent_offset(PixelLength(indent)).0;
        let lines: Vec<_> = ifc.lines[first..]
            .iter()
            .map(|line| {
                (
                    line.entity,
                    line.height.0,
                    ifc.content_width(line.from_break, line.to_break).0,
                )
            })
            .collect();

        let mut y = content_offset.1;
        for (index, (line_entity, height, advance)) in lines.into_iter().enumerate() {
            let (x0, w0) = if index == 0 {
                (content_offset.0 + indent_offset, width - indent)
            } else {
                (content_offset.0, width)
            };
            let (x, w) = match align {
                TextAlign::Left | TextAlign::Start | TextAlign::End => (x0, advance),
                TextAlign::Right => (x0 + (w0 - advance), advance),
                TextAlign::Center => (x0 + (w0 - advance) / 2.0, advance),
                TextAlign::Justify => (x0, w0),
            };
            let (px, py, pw, ph) = frame.to_physical((x, y, w, height));
            self.set_position(line_entity, px, py, pw, ph, z);
            self.emit(BlockLevelElement::Leaf(line_entity));
            y += height;
        }
        Ok(())
    }
}

/// A box whose content is laid out in logical coordinates and the place of the box in the
/// document, to turn the logical position of the boxes in it into physical ones.
#[derive(Debug, Copy, Clone)]
struct Frame {
    /// The flow of the content of the box.
    flow: Flow,
    /// The top left corner of the border box of the box in document coordinates.
    origin: Point<f32>,
    /// The inline and block size of the border box.
    inline_size: f32,
    block_size: f32,
}

impl Frame {
    /// The physical position in the document of a logical rectangle `(x, y, width, height)` that
    /// is relative to the border box.
    fn to_physical(&self, rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
        let (x, y, width, height) = self.flow.rect_to_physical(rect, (self.inline_size, self.block_size));
        (self.origin.x + x, self.origin.y + y, width, height)
    }

    /// The top left corner of the border box of a child that has `layout`, which is logical.
    fn child_origin(&self, layout: &taffy::Layout) -> Point<f32> {
        let (x, y, _, _) = self.to_physical((
            layout.location.x,
            layout.location.y,
            layout.size.width,
            layout.size.height,
        ));
        Point { x, y }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod subtree_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::TryInto;

    use crate::simple_dom::{Quantity::*, body, div, html};

    fn layout(html: &crate::simple_dom::Node, page_width: crate::simple_dom::Quantity) -> World {
        let mut world = html.construct();
        let page_width: PixelLength = page_width.try_into().unwrap();
        TaffyBlockLayout {
            root_entity: html.entity(),
            page_width,
            page_height: PixelLength(1000.0),
            margin: EdgeValues::all(PixelLength(0.0)),
        }
        .run(&mut world)
        .expect("taffy block layout failed");
        world
    }

    fn pos(x: f32, y: f32, width: f32, height: f32, z: i64) -> Position {
        Position {
            x: PixelLength(x),
            y: PixelLength(y),
            width: PixelLength(width),
            height: PixelLength(height),
            z: ZIndex(z),
            page: 0,
        }
    }

    #[test]
    fn vertical_stacking() {
        let html = &html(vec![
            div(vec![])
                .height(Px(10.0))
                .margin_top(Px(10.0))
                .margin_bottom(Px(30.0)),
            div(vec![]),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        let pos1: &Position = html[0].get(&positions);
        let pos2: &Position = html[1].get(&positions);
        assert_eq!(pos1.y, PixelLength(10.0));
        assert_eq!(pos2.y, PixelLength(50.0));
    }

    #[test]
    fn content_height_of_container_with_children() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)),
            div(vec![]).height(Px(40.0)),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        assert_eq!(html.get(&positions), &pos(0.0, 0.0, 100.0, 50.0, 1));
    }

    #[test]
    fn nested_elements() {
        let child = div(vec![]).height(Px(20.0)).margin_bottom(Px(30.0));
        let html = &html(vec![
            body(vec![
                div(vec![child.clone(), child])
                    .padding(Px(10.0))
                    .margin_top(Px(5.0)),
            ])
            .margin_left(Px(10.0)),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();

        let container: &Position = html["body"][0].get(&positions);
        assert_eq!(
            (container.x, container.y),
            (PixelLength(20.0), PixelLength(15.0))
        );

        let child1: &Position = html["body"][0][0].get(&positions);
        assert_eq!((child1.x, child1.y), (container.x, container.y));
        assert_eq!(child1.height, PixelLength(20.0));

        let child2: &Position = html["body"][0][1].get(&positions);
        assert_eq!(child2.x, container.x);
        assert_eq!(child2.y - child1.y, PixelLength(50.0));
        assert_eq!(child2.height, PixelLength(20.0));
    }

    #[test]
    fn left_margin() {
        let html = &html(vec![
            div(vec![div(vec![]).margin_left(Px(15.0))]).margin_left(Px(20.0)),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        assert_eq!(html["div"].get(&positions), &pos(20.0, 0.0, 80.0, 0.0, 2));
        assert_eq!(
            html["div"]["div"].get(&positions),
            &pos(35.0, 0.0, 65.0, 0.0, 3)
        );
    }

    #[test]
    fn margin_top_offsets_first_element() {
        let html = &html(vec![div(vec![]).margin_top(Px(10.0))]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        assert_eq!(html[0].get(&positions), &pos(0.0, 10.0, 100.0, 0.0, 2));
    }

    #[test]
    fn margin_collapsing() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)).margin_bottom(Px(30.0)),
            div(vec![]).height(Px(10.0)).margin_top(Px(10.0)),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        let pos2: &Position = html[1].get(&positions);
        assert_eq!(pos2.y, PixelLength(40.0));
    }

    #[test]
    fn percentage_width() {
        let html = &html(vec![
            div(vec![]).width(Percent(50.0)),
            div(vec![]).width(Percent(150.0)),
        ]);
        let world = layout(html, Px(100.0));
        let positions = world.read_storage::<Position>();
        assert_eq!(html[0].get(&positions), &pos(0.0, 0.0, 50.0, 0.0, 2));
        assert_eq!(html[1].get(&positions), &pos(0.0, 0.0, 150.0, 0.0, 2));
    }

    #[test]
    fn text_list_is_in_document_order() {
        let html = &html(vec![
            div(vec![]).height(Px(10.0)),
            div(vec![]).height(Px(10.0)),
        ]);
        let world = layout(html, Px(100.0));
        let text_list = world.read_resource::<TextList>();
        let starts: Vec<Entity> = text_list
            .list
            .iter()
            .filter_map(|element| match element {
                BlockLevelElement::ContainerStart(entity) => Some(*entity),
                _ => None,
            })
            .collect();
        assert_eq!(
            starts,
            vec![html.entity(), html[0].entity(), html[1].entity()]
        );
    }
}
