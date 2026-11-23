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
mod columns;
mod flow;
mod paginate;
mod replaced;
mod rows;
mod service;
mod style;
mod subtree;
mod table;
mod tree;
mod window;

use std::collections::HashMap;
use std::rc::Rc;

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

    /// A solver that the tests give to the layout instead of the exact one.
    pub type Solve = Box<dyn for<'k, 'a> FnOnce(&'k mut helium_paginate::Kernel<'a>) -> helium_paginate::Solution>;

    thread_local! {
        pub static BEFORE: RefCell<Option<Hook>> = const { RefCell::new(None) };
        pub static AFTER: RefCell<Option<Hook>> = const { RefCell::new(None) };
        pub static SOLVE: RefCell<Option<Solve>> = const { RefCell::new(None) };
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
    /// Make the page as wide as the content (the max-content width of the root, which is what
    /// shrink-to-fit comes to when nothing limits it), instead of `page_width`: for a document
    /// that is cropped to its content.
    pub shrink_wrap: bool,
}

/// A float on a page or in a column, laid out.
#[derive(Debug, Clone, Copy)]
struct PlacedFloat {
    id: NodeId,
    side: helium_paginate::FloatSide,
    /// The margins of the float, and the extent of its margin box.
    margin: taffy::geometry::Rect<f32>,
    outer: f32,
}

/// The part of a box on one page.
struct Fragment {
    position: Position,
    decoration: Decoration,
}

impl TaffyBlockLayout {
    pub fn run(self, world: &mut World) -> Result<(), String> {
        self.run_sized(world).map(|_| ())
    }

    /// Lays out the pages and returns the width of the page: `page_width`, or the width of the
    /// content if the layout is to `shrink_wrap` it.
    pub fn run_sized(self, world: &mut World) -> Result<PixelLength, String> {
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
        world.register::<crate::layout::components::ColumnRule>();
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
        let page_width_unmeasured = self.page_width;
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
            measuring_max_content: false,
            window: None,
            windowed: vec![],
            lines: HashMap::new(),
            flow: None,
            icb_height: 0.0,
            plain_root: None,
            page_fragments: HashMap::new(),
            paragraph_variants: 0,
            variants_config: helium_paginate::Variants::default(),
            earlier_fragments: 3,
            page_variants: HashMap::new(),
        };
        let root = tree.build_box(self.root_entity, &block_ctxs);
        tree.mark_orthogonal(root, crate::layout::writing_mode::Flow::HorizontalTb);
        tree.mark_columns(root);
        let out_of_flow: Vec<bool> = (0..tree.nodes.len())
            .map(|index| tree::node_is_out_of_flow(&tree, NodeId::from(index)))
            .collect();

        // the flow of the document, and the geometry of the page at every place of it
        let flow = std::rc::Rc::new(flow::extract(
            &mut tree,
            root,
            page_margins.rtl,
            self.page_height.0 > 0.0,
            self.shrink_wrap,
            (
                (self.page_width - self.margin.left - self.margin.right).0.max(0.0),
                (self.page_height - self.margin.top - self.margin.bottom).0.max(0.0),
            ),
        ));
        tree.flow = Some(flow.clone());

        // The width of the page of a document that is cropped: what the content needs. The root
        // is laid out once with no limit (the containers with columns need the flow for that), and
        // what it comes to is the width of the page.
        let page_width = if self.shrink_wrap {
            tree.clear_window();
            tree.measuring_max_content = true;
            taffy::compute_root_layout(
                &mut tree,
                root,
                Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::Definite(self.page_height.0),
                },
            );
            tree.measuring_max_content = false;
            let measured = tree.node(root).layout.size.width;
            tree.clear_window();
            PixelLength(measured.max(0.0))
        } else {
            page_width_unmeasured
        };
        tree.fallback_line_width = (page_width - self.margin.left - self.margin.right).0.max(0.0);
        let geometry = paginate::Geometry {
            margins: &page_margins,
            size: (page_width, self.page_height),
            margin: self.margin,
        };
        let group_elements = paginate::group_elements(&tree, &flow, &geometry.margins.keys);
        let classes = std::sync::Arc::new(paginate::Classes::new(&tree, &flow, &group_elements));
        let first_class = classes.of(helium_paginate::Position::Item(0)).clone();
        let first_spec = geometry.spec(&geometry.context(0, &first_class));
        let problem = helium_paginate::Problem::new(geometry.table(classes.clone()), flow.items.clone())
            .with_repeats(flow.repeats.clone());
        // the pages choose among the ways to break the paragraphs that ask for it (`looseness`)
        let looseness = flow
            .paragraphs
            .iter()
            .map(|&paragraph| tree.computed_values.get(tree.node(paragraph).entity).map_or(0, |style| style.page.looseness as u32))
            .max()
            .unwrap_or(0);
        let tuning = Tuning::from_environment();
        let problem = problem.with_variants(looseness, tuning.variant_tolerance);
        let objective = helium_paginate::CssObjective { line_tolerance: tuning.line_tolerance };
        tree.paragraph_variants = looseness;
        tree.earlier_fragments = tuning.earlier_fragments;
        tree.variants_config = helium_paginate::Variants {
            extra: looseness,
            tolerance: tuning.variant_tolerance,
            line_tolerance: tuning.line_tolerance,
        };
        tree.icb_height = first_spec.block_size;

        // the header rows that the pages repeat inside of a table
        let headers: HashMap<NodeId, (NodeId, window::Window)> = (0..tree.nodes.len())
            .map(NodeId::from)
            .filter(|&id| tree.node(id).content == Content::Table)
            .filter_map(|table| {
                let header = tree.repeated_header(table)?;
                let span = tree.node(header).span.as_ref()?;
                let whole = window::Window::over(span.start, span.end);
                Some((table, (header, whole)))
            })
            .collect();
        // the footer rows that the pages repeat inside of a table
        let footers: HashMap<NodeId, (NodeId, window::Window)> = (0..tree.nodes.len())
            .map(NodeId::from)
            .filter(|&id| tree.node(id).content == Content::Table)
            .filter_map(|table| {
                let footer = tree.repeated_footer(table)?;
                let span = tree.node(footer).span.as_ref()?;
                let whole = window::Window::over(span.start, span.end);
                Some((table, (footer, whole)))
            })
            .collect();
        // choose the pages
        let before = std::time::Instant::now();
        let mut service = service::TreeService::new(&mut tree, &flow, first_spec.block_size);
        #[cfg(test)]
        probe::call(&probe::BEFORE, &mut service);
        // (start of the page, its end, the page)
        // (and the fragments of the containers that are on the page)
        let mut planned: Vec<(
            helium_paginate::PageStart,
            helium_paginate::Position,
            helium_paginate::Position,
            helium_paginate::PageFill,
            Vec<helium_paginate::PageFragment>,
            // the ways to break the paragraphs of the page that are not the first
            Vec<helium_paginate::Choice>,
            // the floats on the page
            Vec<(helium_paginate::BoxRef, helium_paginate::FloatSide)>,
        )> = vec![];
        {
            use helium_paginate::Solver;
            let mut kernel = helium_paginate::Kernel::new(&problem, &mut service);
            #[cfg(test)]
            let custom = probe::SOLVE.with(|slot| slot.borrow_mut().take()).map(|solve| solve(&mut kernel));
            #[cfg(not(test))]
            let custom: Option<helium_paginate::Solution> = None;
            let solution = custom
                .unwrap_or_else(|| helium_paginate::ExactSolver::new(objective).solve(&mut kernel));
            let fills = kernel.realize(&solution).map_err(|error| format!("invalid pagination: {error:?}"))?;
            let end = helium_paginate::Position::Item(flow.items.len());
            for (page, fill) in fills.iter().enumerate() {
                let start = solution.pages[page];
                let finish = kernel.position(fill.end);
                let fragments = kernel.fragments(&start, fill.end);
                let choices = kernel.choices(fill.choices).to_vec();
                let floats = kernel.float_boxes(fill.placed);
                planned.push((start, kernel.position(start.at), finish, *fill, fragments, choices, floats));
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
                        line_demerits: 0.0,
                        line_extent: 0.0,
                        choices: helium_paginate::ChoiceId::NONE,
                        floats_used: 0.0,
                        repeated: 0.0,
                        placed: helium_paginate::FloatList::NONE,
                        carry: helium_paginate::FloatList::NONE,
                    },
                    vec![],
                    vec![],
                    vec![],
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
        let mut repeated: std::collections::HashSet<Entity> = std::collections::HashSet::new();
        let mut rules: Vec<Rule> = vec![];
        let mut line_stats = LineStats::default();
        let reporting = PAGINATION_REPORT.load(std::sync::atomic::Ordering::Relaxed);
        let mut pages_report: Vec<serde_json::Value> = vec![];
        let mut positions = world.write_storage::<Position>();
        let mut decorations = world.write_storage::<Decoration>();
        let mut text_list_resource = world.write_resource::<TextList>();
        let mut groups_resource = world.write_resource::<AbsoluteGroups>();
        // the number of every page in the page groups that it is part of
        let page_groups = paginate::group_positions(
            &group_elements,
            &planned.iter().map(|(start, from, to, ..)| (start.index, *from, *to)).collect::<Vec<_>>(),
        );
        for (number, (start, from, to, fill, on_page, choices, floats)) in planned.iter().enumerate() {
            // the pages that a break to a left or right page left empty
            while page_contexts.len() < start.index {
                let index = page_contexts.len();
                page_contexts.push(crate::layout::pages::PageContext {
                    index,
                    name: None,
                    rtl: page_margins.rtl,
                    groups: vec![],
                });
            }
            let class = classes.of(*from).clone();
            let context = geometry.context(start.index, &class);
            let (edges, _) = geometry.of_page(&context);
            let spec = geometry.spec(&context);
            let window = paginate::window(
                &flow.items,
                &flow.leaves,
                helium_paginate::BreakLevel::Page,
                None,
                *from,
                *to,
                |item| match flow.items[item] {
                    helium_paginate::Item::Gap { node } => {
                        helium_paginate::LayoutService::extent(&mut service, node, spec.inline_size) as f32
                    }
                    _ => 0.0,
                },
            );
            let tree = &mut *service.tree;
            // The floats of the page are laid out on their own for the width of the page. They
            // are in the order of the flow, the ones that waited first.
            let mut placed_floats: Vec<PlacedFloat> = vec![];
            for &(node, side) in floats {
                let id = NodeId::from(node.0 as usize);
                let laid_out = tree.layout_float(id, spec.inline_size, tree.icb_height);
                placed_floats.push(PlacedFloat { id, side, margin: laid_out.margin, outer: laid_out.outer_height() });
            }
            let floats_height = |side| -> f32 {
                placed_floats.iter().filter(|placed| placed.side == side).map(|placed| placed.outer).sum()
            };
            let (top_floats, bottom_floats) =
                (floats_height(helium_paginate::FloatSide::Top), floats_height(helium_paginate::FloatSide::Bottom));
            tree.page_fragments = on_page
                .iter()
                .map(|fragment| {
                    let key = (fragment.from, fragment.page_inline.to_bits(), fragment.available.to_bits(), fragment.end);
                    (NodeId::from(fragment.container.0 as usize), key)
                })
                .collect();
            tree.page_variants = choices
                .iter()
                .filter_map(|choice| match flow.items[choice.item] {
                    helium_paginate::Item::Paragraph { id, .. } => Some((flow.paragraphs[id.0 as usize], choice.variant)),
                    _ => None,
                })
                .collect();
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
            // (the text of the page is what is left of it next to the floats)
            let text_used = fill.used - fill.floats_used - fill.repeated;
            if !fill.overflow && (height - text_used).abs() > 0.5 {
                log::warn!(
                    "page {} has {height} of text, but the paginator counted {text_used}",
                    start.index + 1,
                );
            }
            // A box that does not fit on a page and has no break in it is not split: what is
            // below the bottom of the page is laid out but is not on the page.
            if fill.overflow && height > spec.block_size + 0.5 {
                log::warn!(
                    "page {} has {height} of content on a page of {} high: the content below the \
                     page is not printed (a box taller than a page is not split)",
                    start.index + 1,
                    spec.block_size
                );
            }
            let HeliumTree {
                nodes, window, inline_ctxs, lines, text_runs, aligns, intrinsic_sizes, page_variants, paragraph_variants, ..
            } = tree;
            let mut paragraphs_log: Vec<serde_json::Value> = vec![];
            let report = reporting.then_some(());
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
                page_variants: &*page_variants,
                column_variants: None,
                paragraph_variants: *paragraph_variants,
                positions: &mut positions,
                decorations: &mut decorations,
                text_list: &mut text_list_resource.list,
                groups: &mut groups_resource.groups,
                group_stack: vec![],
                last_flow_entity: None,
                window: window.as_ref().expect("the window is set"),
                page: start.index,
                // (the text ends above the floats at the bottom of the page)
                page_bottom: edges.top.0 + spec.block_size - bottom_floats,
                fragments: &mut fragments,
                headers: &headers,
                footers: &footers,
                block_size: spec.block_size,
                repeating: false,
                repeated: &mut repeated,
                overlay: None,
                rules: &mut rules,
                line_stats: &mut line_stats,
                paragraphs_log: report.as_ref().map(|_| &mut paragraphs_log),
                clip: None,
                row: None,
            };
            // The floats at the top of the page, in order, then the text below them, then the floats
            // at the bottom, which end at the bottom of the content area of the page (the order
            // is the one of reading).
            let mut write_floats = |writer: &mut Writer, side, mut y: f32| -> Result<(), String> {
                for placed in placed_floats.iter().filter(|placed| placed.side == side) {
                    let origin = Point { x: edges.left.0 + placed.margin.left, y: y + placed.margin.top };
                    writer.write_box(placed.id, origin, Flow::HorizontalTb, 1, 0)?;
                    y += placed.outer;
                }
                Ok(())
            };
            write_floats(&mut writer, helium_paginate::FloatSide::Top, edges.top.0)?;
            // The root is placed at the top left corner of the content area of the page, below
            // the floats at its top.
            writer.write_box(
                root,
                Point {
                    x: edges.left.0,
                    y: edges.top.0 + top_floats,
                },
                Flow::HorizontalTb,
                1,
                0,
            )?;
            write_floats(
                &mut writer,
                helium_paginate::FloatSide::Bottom,
                edges.top.0 + spec.block_size - bottom_floats,
            )?;
            if reporting {
                let containers: Vec<serde_json::Value> = on_page
                    .iter()
                    .filter_map(|fragment| {
                        let id = NodeId::from(fragment.container.0 as usize);
                        let key = (fragment.from, fragment.page_inline.to_bits(), fragment.available.to_bits(), fragment.end);
                        let plan = service.tree.node(id).columns.as_ref()?.fragments.get(&key)?.clone();
                        Some(serde_json::json!({
                            "container": usize::from(id),
                            "starts_in_container": fragment.from.is_some(),
                            "ends_in_container": fragment.end.is_some(),
                            "available": fragment.available,
                            "height": plan.height,
                            "columns": plan.used,
                        }))
                    })
                    .collect();
                pages_report.push(page_report(start, from, to, fill, on_page, choices, &objective, &flow, std::mem::take(&mut paragraphs_log), containers, number + 1 == planned.len()));
            }
            page_contexts.push(crate::layout::pages::PageContext { groups: page_groups[number].clone(), ..context });
        }
        if reporting {
            let report = serde_json::json!({
                "tuning": {
                    "line_tolerance": tuning.line_tolerance,
                    "variant_tolerance": tuning.variant_tolerance,
                    "looseness": looseness,
                },
                "items": flow.items.len(),
                "lines": {
                    "count": line_stats.lines,
                    "mean_badness": line_stats.badness / line_stats.lines.max(1) as f64,
                    "worst_badness": line_stats.worst,
                    "over_100": line_stats.loose,
                },
                "pages": pages_report,
            });
            if let Ok(mut slot) = PAGINATION_JSON.lock() {
                *slot = serde_json::to_string_pretty(&report).ok();
            }
        }
        log::info!("TIMED: layout of the pages {:?}", before.elapsed());
        log::info!(
            "LINES: {} lines, mean badness {:.2}, worst {:.1}, {} over 100",
            line_stats.lines,
            line_stats.badness / line_stats.lines.max(1) as f64,
            line_stats.worst,
            line_stats.loose
        );
        drop(service);
        drop(tree);

        // The rules between the columns are boxes with a background of their own.
        {
            let mut rule_storage = world.write_storage::<crate::layout::components::ColumnRule>();
            for rule in rules {
                entities
                    .build_entity()
                    .with(rule.position, &mut positions)
                    .with(
                        Decoration { background_color: rule.color, ..Decoration::default() },
                        &mut decorations,
                    )
                    .with(crate::layout::components::ColumnRule, &mut rule_storage)
                    .build();
            }
        }

        // Boxes that are on more than one page are split: every page gets the part of the
        // decoration that is on that page.
        let mut fragmented_storage = world.write_storage::<crate::layout::components::Fragmented>();
        let mut fragment_storage = world.write_storage::<crate::layout::components::DecorationFragment>();
        // (in the order of the entities: a hash map has no order, and the order in which the parts of
        // boxes are painted would change from one run to the next)
        let mut fragments: Vec<_> = fragments.into_iter().collect();
        fragments.sort_by_key(|(entity, _)| entity.id());
        for (entity, mut parts) in fragments {
            // a box that was written again (the header rows of a table) was written as it is on
            // the first page, too: that is the first of its parts
            if repeated.contains(&entity) {
                if let (Some(&position), Some(&decoration)) = (positions.get(entity), decorations.get(entity)) {
                    parts.insert(0, Fragment { position, decoration });
                }
            }
            if parts.len() < 2 {
                continue;
            }
            let first = &parts[0];
            let deco = first.decoration;
            let visible = deco.background_color.alpha() > 0
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
        Ok(page_width)
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
    line_cache: &'a mut tree::LineCache,
    /// The ways that the paragraphs of the page are broken that are not the first, and the most lines
    /// more or fewer than the best breaking that the paragraphs may have.
    page_variants: &'a HashMap<NodeId, usize>,
    /// The ways to break the paragraphs of the column that is being written, which are the ones
    /// of the column and not of the page.
    column_variants: Option<&'a HashMap<NodeId, usize>>,
    paragraph_variants: u32,
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
    /// The header rows that the tables repeat, by table: the row group and a window of the items
    /// that it has.
    headers: &'a HashMap<NodeId, (NodeId, window::Window)>,
    /// The footer rows that the tables repeat, by table: the row group and a window of the items
    /// that it has.
    footers: &'a HashMap<NodeId, (NodeId, window::Window)>,
    /// The height of the content area of the page, which decides whether a footer is repeated.
    block_size: f32,
    /// Whether the header of a table is being written again, behind a page break. It is written
    /// as parts of the boxes (the box is on the page that it was first written on, too), and it
    /// is not part of the structure of the document.
    repeating: bool,
    /// The boxes that have been written again, which have a part on every page of the table.
    repeated: &'a mut std::collections::HashSet<Entity>,
    /// The layouts of the boxes in the column that is being written, which are not the ones the
    /// nodes have (they are the ones of the last column).
    overlay: Option<&'a HashMap<NodeId, columns::Snap>>,
    /// The rules between the columns.
    rules: &'a mut Vec<Rule>,
    /// How well the lines that are written are broken (for the log).
    line_stats: &'a mut LineStats,
    /// What is written of the paragraphs of the page, if the choices of the pages are reported.
    paragraphs_log: Option<&'a mut Vec<serde_json::Value>>,
    /// The part of the page (its top and its bottom) that a table row split over pages shows,
    /// while the boxes in it are written: what is outside of it is on another page.
    clip: Option<(f32, f32)>,
    /// The row that is being written, if it is split over pages: its cuts and the part of it that
    /// the page has, from which the cells find theirs.
    row: Option<RowOnPage>,
}

/// A table row split over pages, on one page.
#[derive(Clone)]
struct RowOnPage {
    cuts: Rc<rows::RowCuts>,
    /// The cuts at the start and at the end of the part on the page.
    start: u32,
    end: u32,
    /// Where the part starts on the page.
    top: f32,
}

/// A rule between two columns: a filled rectangle.
struct Rule {
    position: Position,
    color: crate::style::properties::computed::PaintColor,
}

impl<'a> Writer<'a, '_> {
    /// The node `id`. The reference does not borrow `self`.
    fn node(&self, id: NodeId) -> &'a Node {
        &self.nodes[usize::from(id)]
    }

    /// The layout of a box: the one in the column that is being written, if there is one.
    fn layout_of(&self, id: NodeId) -> taffy::Layout {
        self.overlay
            .and_then(|overlay| overlay.get(&id))
            .map_or(self.node(id).layout, |snap| snap.layout)
    }

    /// The width and the indent that the lines of a paragraph were broken for.
    fn measured_of(&self, id: NodeId) -> Option<(f32, f32)> {
        match self.overlay.and_then(|overlay| overlay.get(&id)) {
            Some(snap) => snap.measured,
            None => self.node(id).measured,
        }
    }

    fn shift_of(&self, id: NodeId) -> f32 {
        self.overlay
            .and_then(|overlay| overlay.get(&id))
            .map_or(self.node(id).content_shift, |snap| snap.content_shift)
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
        if self.group_stack.is_empty() && !self.repeating {
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
        // A table row that the page has a part of: the part is between two cuts, and the row is
        // written whole, above the page where it starts in it.
        let node = self.node(id);
        let part = (self.clip.is_none()
            && self.group_stack.is_empty()
            && node.columns.is_none()
            && node.content == Content::Blocks
            && (self.window.start_container == Some(id) || self.window.end_container == Some(id)))
        .then(|| window::row_part(self.window, node, id))
        .flatten();
        let (clip_before, row_before) = (self.clip, self.row.take());
        if let (Some(part), Some(cuts)) = (part, node.row_cuts.clone()) {
            let (start, end) = self.window.row_cut_offsets(id);
            let top = origin.y + part.top;
            self.clip = Some((top, top + part.height));
            self.row = Some(RowOnPage { start: start.unwrap_or(0), end: end.unwrap_or(cuts.height), cuts, top });
        } else {
            self.row = row_before.clone();
        }
        let written = self.write_box_in(id, origin, parent_flow, z, z_base);
        self.clip = clip_before;
        self.row = row_before;
        written
    }

    fn write_box_in(
        &mut self,
        id: NodeId,
        origin: Point<f32>,
        parent_flow: Flow,
        z: i64,
        z_base: i64,
    ) -> Result<(), String> {
        let node = self.node(id);
        let entity = node.entity;
        let layout = self.layout_of(id);
        let content = node.content;
        let in_flow_box = node.span.as_ref().filter(|span| !span.anchored && self.group_stack.is_empty());
        let open_start = in_flow_box.is_some_and(|span| self.window.cut_at_start(id, span));
        let open_end = in_flow_box.is_some_and(|span| self.window.cut_at_end(id, span));
        // the part of the page that a row split over pages shows (the boxes of a group are not)
        let clip = if self.group_stack.is_empty() { self.clip } else { None };
        // A table or a row group is laid out as a whole: the part on the page is cut from it.
        let table_part = node
            .span
            .as_ref()
            .filter(|span| !span.split && !span.kids.is_empty() && content != Content::Inline);
        // A page that goes on inside of a table has the header rows of the table at its top.
        let repeat = if open_start && content == Content::Table { self.repeat_of(id) } else { None };
        let repeat_height = repeat.map_or(0.0, |(_, height)| height);
        // A page that ends inside of a table has the footer rows of the table behind its last row.
        let footer = if open_end && content == Content::Table { self.footer_of(id) } else { None };
        let footer_height = footer.map_or(0.0, |(_, height)| height);
        let (cut_top, cut_height) = match table_part {
            Some(_) => {
                let (top, bottom) =
                    window::table_fragment(self.nodes, self.window, node, layout.size.height);
                (top, Some(bottom - top + repeat_height + footer_height))
            }
            None => (0.0, None),
        };

        // The layout is in the logical axes of the parent: the physical size and edges of the
        // box are found by turning them around if the parent is vertical.
        let (width, mut height) = parent_flow.swap((layout.size.width, layout.size.height));
        if let Some(cut) = cut_height {
            height = cut;
        }
        // A box that is outside of the part of a row on the page is on another page; one that is
        // across its edge goes on there.
        const EDGE: f32 = 0.01;
        let (mut clip_start, mut clip_end) = (false, false);
        if let Some((visible_top, visible_bottom)) = clip {
            let (top, bottom) = (origin.y, origin.y + height);
            let outside = if bottom - top <= EDGE {
                top < visible_top - EDGE || top >= visible_bottom - EDGE
            } else {
                bottom <= visible_top + EDGE || top >= visible_bottom - EDGE
            };
            if outside {
                return Ok(());
            }
            clip_start = top < visible_top - EDGE;
            clip_end = bottom > visible_bottom + EDGE;
        }
        let (open_start, open_end) = (open_start || clip_start, open_end || clip_end);
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
        // what is written of the box: the edges of the part of a row on the page have no padding
        // and no border
        let (rec_y, rec_height) = match clip {
            Some((visible_top, visible_bottom)) => {
                let top = if clip_start { visible_top } else { content_y };
                let bottom = if clip_end { visible_bottom } else { content_y + content_height };
                (top, (bottom - top).max(0.0))
            }
            None => (content_y, content_height),
        };
        if self.repeating {
            self.repeated.insert(entity);
        } else if !open_start {
            self.set_position(entity, content_x, rec_y, content_width, rec_height, z);
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
        if clip_start {
            decoration.border.top = PixelLength(0.0);
            decoration.padding.top = PixelLength(0.0);
        }
        if clip_end {
            decoration.border.bottom = PixelLength(0.0);
            decoration.padding.bottom = PixelLength(0.0);
        }
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
        if !open_start && !self.repeating {
            let _ = self.decorations.insert(entity, decoration);
        }
        if open_start || open_end || self.repeating {
            // The part of a box that goes on on the next page reaches the bottom of the content
            // area of its page, also where the content on the page ends above it.
            let fragment_height = if open_end {
                rec_height.max(self.page_bottom - rec_y)
            } else {
                rec_height
            };
            self.fragments.entry(entity).or_default().push(Fragment {
                position: Position {
                    x: PixelLength(content_x),
                    y: PixelLength(rec_y),
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

        // the content of a table cell that `vertical-align` moves down, and the one of a cell of a
        // row that is split over pages, which starts where its part of the row does
        let (cell_shift, content_clip) = match (&self.row, self.group_stack.is_empty()) {
            (Some(row), true) => match row.cuts.cell_part(id, row.start, row.end) {
                Some((shift, until)) => (
                    rows::dequantize(shift),
                    Some((row.top, until.map_or(f32::INFINITY, |until| row.top + rows::dequantize(until)))),
                ),
                None => (0.0, None),
            },
            _ => (0.0, None),
        };
        let shift = self.shift_of(id) + cell_shift;
        let clip_of_row = self.clip;
        if let Some(visible) = content_clip {
            self.clip = Some(visible);
        }
        let kid_shift = shift - cut_top;
        let columns = node.columns.as_ref().and_then(|columns| columns.layouts.as_ref());
        match content {
            // a container with columns: the columns are written, one after the other
            _ if columns.is_some() => {
                let layouts = columns.expect("the columns are laid out");
                self.write_columns(
                    id,
                    layouts,
                    flow,
                    (content_x, content_y),
                    (content_width, content_height),
                    (z, z_base),
                    !open_start,
                )?;
            }
            Content::Blocks if table_part.is_some() => {
                // (the rows that are cut are higher than their part of the layout: the boxes
                // behind them are moved down)
                let mut extra = 0.0;
                for &child in &table_part.unwrap().kids {
                    if !self.in_window(child) {
                        continue;
                    }
                    let mut child_origin = frame.child_origin(&self.layout_of(child));
                    child_origin.y += kid_shift + self.part_top(child) + extra;
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                    extra += window::part_extra(self.nodes, self.window, child);
                }
            }
            Content::Blocks => {
                for &child in self.window.children(self.nodes, id) {
                    if self.out_of_flow[usize::from(child)]
                        || self.node(child).style.display == taffy::Display::None
                    {
                        continue;
                    }
                    let mut child_origin = frame.child_origin(&self.layout_of(child));
                    child_origin.y += shift;
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                }
            }
            Content::Inline => {
                // the `inline-block` boxes are written once, with the first part of the paragraph
                if !open_start {
                    self.write_atoms(id, flow, z, z_base)?;
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
                if let Some((header, _)) = repeat {
                    // the header rows again, at the top of the part of the table on this page
                    let header_layout = self.layout_of(header);
                    let mut header_origin = frame.child_origin(&header_layout);
                    header_origin.y += shift - header_layout.location.y;
                    let (header_z, header_base) = self.child_z(header, z, z_base);
                    let headers = self.headers;
                    let window_before = std::mem::replace(&mut self.window, &headers[&id].1);
                    let repeating_before = std::mem::replace(&mut self.repeating, true);
                    let written = self.write_box(header, header_origin, flow, header_z, header_base);
                    self.repeating = repeating_before;
                    self.window = window_before;
                    written?;
                }
                // (what goes on on the next page reaches down to the footer, not to the bottom of
                // the page)
                let bottom_before = self.page_bottom;
                if footer.is_some() {
                    self.page_bottom = frame.origin.y + shift + (inline_size.1 - footer_height);
                }
                let mut extra = 0.0;
                for child in parts {
                    let mut child_origin = frame.child_origin(&self.layout_of(child));
                    child_origin.y += kid_shift + repeat_height + extra + if table_part.is_some() { self.part_top(child) } else { 0.0 };
                    let (child_z, child_base) = self.child_z(child, z, z_base);
                    self.write_box(child, child_origin, flow, child_z, child_base)?;
                    extra += window::part_extra(self.nodes, self.window, child);
                }
                self.page_bottom = bottom_before;
                if let Some((footer, _)) = footer {
                    // the footer rows again, directly behind the last row of the part of the table
                    // on this page
                    let footer_layout = self.layout_of(footer);
                    let mut footer_origin = frame.child_origin(&footer_layout);
                    footer_origin.y = frame.origin.y + shift + (inline_size.1 - footer_height);
                    let (footer_z, footer_base) = self.child_z(footer, z, z_base);
                    let footers = self.footers;
                    let window_before = std::mem::replace(&mut self.window, &footers[&id].1);
                    let repeating_before = std::mem::replace(&mut self.repeating, true);
                    let written = self.write_box(footer, footer_origin, flow, footer_z, footer_base);
                    self.repeating = repeating_before;
                    self.window = window_before;
                    written?;
                }
            }
        }

        self.clip = clip_of_row;
        if anchored {
            self.group_stack.pop();
        }
        if !open_end {
            self.emit(BlockLevelElement::ContainerEnd(entity));
        }

        // the absolutely positioned boxes this box is the containing block of
        for &child in &node.hoisted {
            self.open_group(None);
            let child_origin = frame.child_origin(&self.layout_of(child));
            let abs_base = z_base + ABS_Z_STEP;
            self.write_box(child, child_origin, flow, abs_base + 1, abs_base)?;
            self.group_stack.pop();
        }
        Ok(())
    }

    /// Writes the `inline-block` boxes of the lines of a box.
    fn write_atoms(&mut self, id: NodeId, flow: Flow, z: i64, z_base: i64) -> Result<(), String> {
        for &atom in &self.node(id).atoms {
            // The contents of an `inline-block` box are placed relative to the top left corner of
            // its margin box. They are moved to the line the box is on later.
            self.open_group(Some(self.node(atom).entity));
            // the border box is inside of the margin box, which is placed on the line
            let margin = flow.edges_to_physical(self.node(atom).layout.margin);
            let location = Point { x: margin.left, y: margin.top };
            let (atom_z, atom_base) = self.child_z(atom, z, z_base);
            self.write_box(atom, location, flow, atom_z, atom_base)?;
            self.group_stack.pop();
        }
        Ok(())
    }

    /// Writes the columns of a container: what is in every column of it, where the column is,
    /// and the rules between the columns.
    #[allow(clippy::too_many_arguments)]
    fn write_columns(
        &mut self,
        id: NodeId,
        layouts: &'a columns::ColumnLayouts,
        flow: Flow,
        (content_x, content_y): (f32, f32),
        (content_width, content_height): (f32, f32),
        (z, z_base): (i64, i64),
        first_fragment: bool,
    ) -> Result<(), String> {
        let node = self.node(id);
        let plan = &layouts.plan;
        // (the `inline-block` boxes are written once, with the first fragment of the container)
        if node.content == Content::Inline && first_fragment {
            self.write_atoms(id, flow, z, z_base)?;
        }
        // the first column of a container with a direction from right to left is at the right
        let rtl = node.mirror_rtl;
        let column_x = |index: usize| {
            if rtl {
                content_x + content_width - (index + 1) as f32 * plan.width - index as f32 * plan.gap
            } else {
                content_x + index as f32 * (plan.width + plan.gap)
            }
        };
        for (index, (window, snapshot)) in plan.columns.iter().zip(&layouts.columns).enumerate() {
            let window_before = std::mem::replace(&mut self.window, window);
            let overlay_before = self.overlay.replace(snapshot);
            let variants_before = std::mem::replace(&mut self.column_variants, plan.variants.get(index));
            // The floats of the column are at its top and its bottom, and its text is between
            // them (the column is as high as the fragment, which has them).
            let floats = &layouts.floats[index];
            let height_of = |side| -> f32 {
                floats.iter().filter(|placed| placed.side == side).map(|placed| placed.outer).sum()
            };
            let (top_floats, bottom_floats) =
                (height_of(helium_paginate::FloatSide::Top), height_of(helium_paginate::FloatSide::Bottom));
            // the part of a box that goes on in the next column is painted down to the bottom of
            // the column's text
            let bottom_before = std::mem::replace(&mut self.page_bottom, content_y + plan.height - bottom_floats);
            let frame = Frame {
                flow,
                origin: Point { x: column_x(index), y: content_y + top_floats },
                inline_size: plan.width,
                block_size: plan.height - top_floats - bottom_floats,
            };
            let mut written = Ok(());
            for (side, mut y) in [
                (helium_paginate::FloatSide::Top, content_y),
                (helium_paginate::FloatSide::Bottom, content_y + plan.height - bottom_floats),
            ] {
                for placed in floats.iter().filter(|placed| placed.side == side) {
                    let origin = Point { x: column_x(index) + placed.margin.left, y: y + placed.margin.top };
                    written = written.and_then(|()| self.write_box(placed.id, origin, flow, z + 1, z_base));
                    y += placed.outer;
                }
            }
            let written = written.and_then(|()| match node.content {
                Content::Inline => self.write_lines(id, &frame, (0.0, 0.0), plan.width, z + 1),
                _ => {
                    let mut written = Ok(());
                    for &child in self.window.children(self.nodes, id) {
                        if self.out_of_flow[usize::from(child)]
                            || self.node(child).style.display == taffy::Display::None
                        {
                            continue;
                        }
                        let child_origin = frame.child_origin(&self.layout_of(child));
                        let (child_z, child_base) = self.child_z(child, z, z_base);
                        written = self.write_box(child, child_origin, flow, child_z, child_base);
                        if written.is_err() {
                            break;
                        }
                    }
                    written
                }
            });
            self.window = window_before;
            self.overlay = overlay_before;
            self.column_variants = variants_before;
            self.page_bottom = bottom_before;
            written?;
        }

        // the rules are in the middle of the gaps between two columns that have content
        let Some(style) = self.computed_values.get(node.entity) else {
            return Ok(());
        };
        let width = style.box_.column_rule_width.0;
        if !style.box_.column_rule_style.is_visible() || width <= 0.0 {
            return Ok(());
        }
        use crate::style::properties::computed::{Color, color_to_paint};
        let color = match style.box_.column_rule_color {
            Color::CurrentColor => color_to_paint(style.font.color),
            color => color_to_paint(color),
        };
        for index in 1..plan.columns.len() {
            let (before, after) = (column_x(index - 1), column_x(index));
            // the gap is between the end of the one that is on the left and the start of the other
            let (left, right) = if rtl { (after + plan.width, before) } else { (before + plan.width, after) };
            self.rules.push(Rule {
                position: Position {
                    x: PixelLength((left + right) / 2.0 - width / 2.0),
                    y: PixelLength(content_y),
                    width: PixelLength(width),
                    height: PixelLength(content_height.max(0.0)),
                    z: ZIndex(z + 1),
                    page: self.page,
                },
                color,
            });
        }
        Ok(())
    }

    /// The header rows that the page repeats at the top of a table that it goes on in, and their
    /// height: the page starts behind the header (and behind the break that follows it), and the
    /// header is not more than a quarter of the page.
    fn repeat_of(&self, table: NodeId) -> Option<(NodeId, f32)> {
        let (header, _) = self.headers.get(&table)?;
        let span = self.node(*header).span.as_ref()?;
        let height = self.layout_of(*header).size.height;
        // (not more than a quarter of the page, as the kernel has it)
        (self.window.start > span.end && f64::from(height) * 4.0 <= f64::from(self.block_size))
            .then(|| (*header, height))
    }

    /// The footer rows that the page repeats behind the last row of a table that it ends in, and
    /// their height: the footer is not on the page, and it is not more than a quarter of it (CSS
    /// Tables 3, 6.2).
    fn footer_of(&self, table: NodeId) -> Option<(NodeId, f32)> {
        let (footer, _) = self.footers.get(&table)?;
        let span = self.node(*footer).span.as_ref()?;
        let height = self.layout_of(*footer).size.height;
        (self.window.end <= span.start && f64::from(height) * 4.0 <= f64::from(self.block_size)).then(|| (*footer, height))
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
        let (line_width, indent) = self.measured_of(id).unwrap_or((width, indent));
        let cut = self.window.cut_of(id);
        // the indent is the one of the first line of the paragraph
        let indent = if cut.is_some_and(|(from, _)| from.is_some()) { 0.0 } else { indent };
        let Some(ifc) = self.inline_ctxs.get_mut(entity) else {
            return Ok(());
        };
        let looseness = style.map_or(0, |style| style.page.looseness as u32);
        let way = self
            .column_variants
            .and_then(|ways| ways.get(&id))
            .or_else(|| self.page_variants.get(&id))
            .copied()
            .unwrap_or(0);
        let variant = (way, self.paragraph_variants.min(looseness));
        let measured = lines_between(
            self.line_cache,
            ifc,
            entity.id(),
            cut,
            variant,
            PixelLength(line_width),
            PixelLength(indent),
            self.text_runs,
            self.intrinsic_sizes,
            self.aligns,
        );
        for line in &measured {
            self.line_stats.add(line.badness);
        }
        if self.paragraphs_log.is_some() {
            let ways = if variant.1 > 0 {
                let from = cut.and_then(|(from, _)| from).map(tree::LineCut::of);
                tree::variants_from(
                    self.line_cache,
                    ifc,
                    entity.id(),
                    from,
                    PixelLength(line_width),
                    PixelLength(indent),
                    variant.1,
                    self.text_runs,
                    self.intrinsic_sizes,
                    self.aligns,
                )
                .iter()
                .map(|lines| {
                    serde_json::json!({
                        "lines": lines.len(),
                        "demerits": lines.iter().map(|line| line.demerits).sum::<f64>(),
                        "worst_badness": lines.iter().map(|line| line.badness).fold(0.0, f64::max),
                    })
                })
                .collect::<Vec<_>>()
            } else {
                vec![]
            };
            let entry = serde_json::json!({
                "entity": entity.id(),
                "cut_at_start": cut.is_some_and(|(from, _)| from.is_some()),
                "cut_at_end": cut.is_some_and(|(_, to)| to.is_some()),
                "lines": measured.len(),
                "way": variant.0,
                "looseness": variant.1,
                "demerits": measured.iter().map(|line| line.demerits).sum::<f64>(),
                "badness": measured.iter().map(|line| (line.badness * 10.0).round() / 10.0).collect::<Vec<_>>(),
                // the ways that the page chose among, if the paragraph has any
                "ways": ways,
            });
            if let Some(log) = self.paragraphs_log.as_mut() {
                log.push(entry);
            }
        }
        // The lines of a paragraph in a row that is split over pages: the ones that are on the
        // part of the row on the page, which are what the page has of the paragraph.
        let clip = if self.group_stack.is_empty() { self.clip } else { None };
        let mut kept_y: Vec<f32> = vec![];
        let mut first_line_kept = true;
        let mut offset = content_offset.1;
        let mut kept = vec![];
        for (index, line) in measured.into_iter().enumerate() {
            let (_, py, _, ph) = frame.to_physical((0.0, offset, 1.0, line.height.0));
            // (the cuts are in 1/64 pixel, and a line is not less than a pixel high)
            let visible = clip.is_none_or(|(top, bottom)| py >= top - 0.1 && py + ph <= bottom + 0.1);
            if index == 0 {
                first_line_kept = visible;
            }
            if visible {
                kept_y.push(offset);
                kept.push(line);
            }
            offset += line.height.0;
        }
        let measured = kept;
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
                    // a line with a `leader()` fills the paragraph, whatever the alignment
                    ifc.leaders_on(line.from_break, line.to_break) > 0,
                )
            })
            .collect();

        let mut line_boxes: Vec<[f32; 4]> = vec![];
        for (index, (line_entity, height, advance, has_leader)) in lines.into_iter().enumerate() {
            let (x0, w0) = if index == 0 && first_line_kept {
                (content_offset.0 + indent_offset, width - indent)
            } else {
                (content_offset.0, width)
            };
            let (x, w) = match align {
                _ if has_leader => (x0, w0.max(advance)),
                TextAlign::Left | TextAlign::Start | TextAlign::End => (x0, advance),
                TextAlign::Right => (x0 + (w0 - advance), advance),
                TextAlign::Center => (x0 + (w0 - advance) / 2.0, advance),
                TextAlign::Justify => (x0, w0),
            };
            let y = kept_y[index];
            let (px, py, pw, ph) = frame.to_physical((x, y, w, height));
            line_boxes.push([px, py, pw, ph]);
            self.set_position(line_entity, px, py, pw, ph, z);
            self.emit(BlockLevelElement::Leaf(line_entity));
        }
        // (where the lines are, for the report of the choices of the pages)
        if let Some(entry) = self.paragraphs_log.as_mut().and_then(|log| log.last_mut()) {
            entry["boxes"] = serde_json::json!(line_boxes);
            entry["justified"] = serde_json::json!(matches!(align, TextAlign::Justify));
        }
        Ok(())
    }
}

/// One page of the report of the choices of the pages: what the page is, what it costs and why,
/// and the paragraphs that are on it.
#[allow(clippy::too_many_arguments)]
fn page_report(
    start: &helium_paginate::PageStart,
    from: &helium_paginate::Position,
    to: &helium_paginate::Position,
    fill: &helium_paginate::PageFill,
    fragments: &[helium_paginate::PageFragment],
    choices: &[helium_paginate::Choice],
    objective: &helium_paginate::CssObjective,
    flow: &flow::Flow,
    paragraphs: Vec<serde_json::Value>,
    containers: Vec<serde_json::Value>,
    last: bool,
) -> serde_json::Value {
    use helium_paginate::Objective;
    let cost = objective.page_cost(fill, last).unwrap_or_default();
    let line_room = objective.line_room(fill);
    let unit = helium_paginate::CssCost::SLACK_UNIT;
    serde_json::json!({
        "page": start.index + 1,
        "kind": format!("{:?}", fill.kind),
        "from": format!("{from:?}"),
        "to": format!("{to:?}"),
        "block_size": fill.spec.block_size,
        "used": fill.used,
        "room": fill.spec.block_size - fill.used,
        "ends_with": format!("{:?}", fill.end_kind),
        "orphan_violation": fill.orphan_violation,
        "widow_violation": fill.widow_violation,
        "overflow": fill.overflow,
        "line_demerits_above_first_way": fill.line_demerits,
        "line_room": line_room,
        "cost": {
            "overflows": cost.overflows,
            "avoided_breaks": cost.avoided_breaks,
            "orphans_and_widows": cost.orphans_and_widows,
            "pages": cost.pages,
            // (the cube of the room in pixels)
            "slack": cost.slack as f64 / (unit * unit * unit),
        },
        "variants": choices
            .iter()
            .map(|choice| serde_json::json!({ "item": choice.item, "way": choice.variant }))
            .collect::<Vec<_>>(),
        "containers": containers,
        "paragraphs": paragraphs,
        "items": flow.items.len(),
    })
}

/// Whether the choices of the pages are written down (`--dump-pagination`).
static PAGINATION_REPORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks for a report of the choices of the pages: [`take_pagination_report`] has it as JSON
/// after the layout.
pub fn report_pagination(on: bool) {
    PAGINATION_REPORT.store(on, std::sync::atomic::Ordering::Relaxed);
}

static PAGINATION_JSON: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The choices of the pages of the last layout as JSON, if they were asked for with
/// [`report_pagination`].
pub fn take_pagination_report() -> Option<String> {
    PAGINATION_JSON.lock().ok()?.take()
}

/// The numbers that decide how much a page gives up for better lines. They are the values of an
/// experiment, and can be changed to try others (`tools/looseness_bench.py`): the environment
/// variables `HELIUM_LINE_TOLERANCE` (the exchange rate of lines and room,
/// [`helium_paginate::CssObjective::line_tolerance`]) and `HELIUM_VARIANT_TOLERANCE` (the worst
/// badness of a line of another way to break a paragraph, [`helium_paginate::Variants`]).
#[derive(Debug, Copy, Clone)]
struct Tuning {
    line_tolerance: f64,
    variant_tolerance: f64,
    /// The fragments of a container that end earlier than the one that fills the page, which it
    /// offers the pages besides it (`HELIUM_EARLIER_FRAGMENTS`, 3 by default; 0 gives only the one that fills it).
    earlier_fragments: u32,
}

impl Tuning {
    fn from_environment() -> Self {
        let number = |name: &str, default: f64| match std::env::var(name) {
            Ok(value) => match value.parse::<f64>() {
                Ok(number) if number > 0.0 && number.is_finite() => {
                    log::info!("TUNING: {name} = {number}");
                    number
                }
                _ => {
                    log::warn!("{name} must be a positive number, not {value:?}");
                    default
                }
            },
            Err(_) => default,
        };
        Tuning {
            line_tolerance: number("HELIUM_LINE_TOLERANCE", helium_paginate::CssObjective::LINE_TOLERANCE),
            variant_tolerance: number("HELIUM_VARIANT_TOLERANCE", helium_paginate::Variants::TOLERANCE),
            earlier_fragments: match std::env::var("HELIUM_EARLIER_FRAGMENTS").ok().and_then(|value| value.parse().ok()) {
                Some(count) => count,
                None => 3,
            },
        }
    }
}

/// How well the lines of the document are broken: the badness of TeX (100 for a line that is
/// stretched or shrunk by its whole stretch), which is infinite for a line that does not fit.
#[derive(Debug, Default)]
struct LineStats {
    lines: usize,
    badness: f64,
    worst: f64,
    loose: usize,
}

impl LineStats {
    fn add(&mut self, badness: f64) {
        // (the badness of a line that cannot be stretched, as the last one, is not a measure)
        let badness = if badness.is_finite() { badness.min(10_000.0) } else { 10_000.0 };
        self.lines += 1;
        self.badness += badness;
        self.worst = self.worst.max(badness);
        self.loose += usize::from(badness > 100.0);
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
            shrink_wrap: false,
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
