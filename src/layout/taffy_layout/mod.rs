//! Box layout with [Taffy](https://github.com/DioxusLabs/taffy).
//!
//! Taffy computes the sizes and positions of all boxes (block, flex and grid containers and
//! absolutely positioned boxes). Helium keeps doing what Taffy cannot do: inline content is
//! broken into lines by [`InlineFormattingContext`], and the document is split into pages by the
//! paginator afterwards.
//!
//! The result is written to the same components the rest of the layout pipeline reads:
//! [`Position`] (content box in document coordinates), [`Decoration`], the [`TextList`] of the
//! paginator and the [`AbsoluteGroups`].

mod collapse;
mod page_groups;
mod style;
mod table;
mod tree;

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
use tree::{Content, HeliumTree, Node};

/// Distance between the z indices of boxes at different levels of nested absolutely positioned
/// boxes. Absolutely positioned boxes are painted on top of the in-flow content.
const ABS_Z_STEP: i64 = 100000;

pub struct TaffyBlockLayout {
    pub root_entity: Entity,
    pub page_width: PixelLength,
    pub page_height: PixelLength,
    pub margin: EdgeValues<PixelLength>,
}

impl TaffyBlockLayout {
    pub fn run(self, world: &mut World) -> Result<(), String> {
        world.register::<Position>();
        world.register::<Decoration>();
        world.register::<IntrinsicSize>();
        world.register::<ImageBox>();
        world.register::<crate::layout::components::PageGroupBox>();
        world.register::<BlockFormattingContext>();
        world.register::<InlineFormattingContext>();
        world.register::<TextRun>();
        world.register::<TableCellSpan>();
        world.register::<TableColumns>();
        world.entry::<TextList>().or_insert_with(Default::default);
        world
            .entry::<crate::layout::pages::PageMargins>()
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
        let page_margins = world.read_resource::<crate::layout::pages::PageMargins>();
        let mut group_boxes = world.write_storage::<crate::layout::components::PageGroupBox>();
        let text_runs = world.read_storage::<TextRun>();
        let table_spans = world.read_storage::<TableCellSpan>();
        let table_columns = world.read_storage::<TableColumns>();

        let content_width = (self.page_width - self.margin.left - self.margin.right).0;
        let icb_height = (self.page_height - self.margin.top - self.margin.bottom)
            .0
            .max(0.0);

        let mut tree = HeliumTree {
            nodes: vec![],
            computed_values: &computed_values,
            inline_ctxs: &mut inline_ctxs,
            intrinsic_sizes: &mut intrinsic_sizes,
            images: &images,
            text_runs: &text_runs,
            table_spans: &table_spans,
            table_columns: &table_columns,
            fallback_line_width: content_width.max(0.0),
        };
        let root = tree.build_box(self.root_entity, &block_ctxs);
        page_groups::wrap_page_groups(
            &mut tree,
            root,
            &page_margins,
            &entities,
            &mut group_boxes,
        );
        tree.mark_orthogonal(root, crate::layout::writing_mode::Flow::HorizontalTb);
        // The initial containing block is the content area of the first page.
        taffy::compute_root_layout(
            &mut tree,
            root,
            Size {
                width: AvailableSpace::Definite(content_width),
                height: AvailableSpace::Definite(icb_height),
            },
        );
        let out_of_flow: Vec<bool> = (0..tree.nodes.len())
            .map(|index| tree::node_is_out_of_flow(&tree, NodeId::from(index)))
            .collect();
        let nodes = std::mem::take(&mut tree.nodes);
        drop(tree);

        let mut writer = Writer {
            nodes: &nodes,
            out_of_flow,
            root,
            entities: &entities,
            computed_values: &computed_values,
            inline_ctxs: &mut inline_ctxs,
            text_runs: &text_runs,
            intrinsic_sizes: &intrinsic_sizes,
            positions: &mut world.write_storage::<Position>(),
            decorations: &mut world.write_storage::<Decoration>(),
            text_list: &mut world.write_resource::<TextList>().list,
            groups: &mut world.write_resource::<AbsoluteGroups>().groups,
            group_stack: vec![],
            last_flow_entity: None,
            anchors: HashMap::new(),
        };
        // The root is placed relative to the top left corner of the content area of the first
        // page. The paginator moves everything to its page.
        writer.write_box(
            root,
            Point {
                x: self.margin.left.0,
                y: 0.0,
            },
            Flow::HorizontalTb,
            1,
            0,
        )
    }
}

struct Writer<'a, 'w> {
    nodes: &'a [Node],
    out_of_flow: Vec<bool>,
    root: NodeId,
    entities: &'a Entities<'w>,
    computed_values: &'a ReadStorage<'w, ComputedValues>,
    inline_ctxs: &'a mut WriteStorage<'w, InlineFormattingContext>,
    text_runs: &'a ReadStorage<'w, TextRun>,
    intrinsic_sizes: &'a WriteStorage<'w, IntrinsicSize>,
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
    /// For every absolutely positioned box, the entity in the main flow in front of it.
    anchors: HashMap<NodeId, Option<Entity>>,
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
                page: 0,
            },
        );
    }

    fn emit(&mut self, element: BlockLevelElement) {
        // absolutely positioned and inline-block boxes are not paginated with the main flow
        if self.group_stack.is_empty() {
            self.text_list.push(element);
            self.last_flow_entity = Some(element.entity());
        }
    }

    fn open_group(&mut self, containing_block: Option<Entity>, owner: Option<Entity>) {
        self.groups.push(AbsoluteGroup {
            containing_block,
            entities: vec![],
            owner,
            parent: self.group_stack.last().copied(),
            anchor: None,
        });
        self.group_stack.push(self.groups.len() - 1);
    }

    /// Writes the layout of a box and everything in it.
    ///   origin       the top left corner of the border box of the box in document coordinates
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

        // The layout is in the logical axes of the parent: the physical size and edges of the
        // box are found by turning them around if the parent is vertical.
        let (width, height) = parent_flow.swap((layout.size.width, layout.size.height));
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
        self.set_position(
            entity,
            content_x,
            content_y,
            content_width,
            content_height,
            z,
        );

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
        let _ = self.decorations.insert(entity, decoration);

        self.emit(BlockLevelElement::ContainerStart(entity));

        // An orthogonal flow is not split between pages: it is a leaf of the main flow, and its
        // content is placed with it once the page it is on is known.
        let anchored = node.orthogonal && content == Content::Blocks;
        if anchored {
            self.emit(BlockLevelElement::Leaf(entity));
            self.open_group(None, None);
            if let Some(&group) = self.group_stack.last() {
                self.groups[group].anchor = Some((entity, Point { x: content_x, y: content_y }));
            }
        }

        let children = &node.children;
        match content {
            Content::Blocks => {
                for &child in children {
                    if self.out_of_flow[usize::from(child)] {
                        self.anchors.insert(child, self.last_flow_entity);
                        continue;
                    }
                    let child_origin = frame.child_origin(&self.node(child).layout);
                    self.write_box(child, child_origin, flow, z + 1, z_base)?;
                }
            }
            Content::Inline => {
                for &child in children {
                    self.anchors.insert(child, self.last_flow_entity);
                }
                for &atom in &node.atoms {
                    // The contents of an `inline-block` box are placed relative to the top left
                    // corner of its margin box. They are moved to the line the box is on later.
                    self.open_group(None, Some(self.node(atom).entity));
                    // the border box is inside of the margin box, which is placed on the line
                    let margin = flow.edges_to_physical(self.node(atom).layout.margin);
                    let location = Point { x: margin.left, y: margin.top };
                    self.write_box(atom, location, flow, z + 1, z_base)?;
                    self.group_stack.pop();
                }
                let content_offset = (
                    logical_border.left + logical_padding.left,
                    logical_border.top + logical_padding.top,
                );
                let content_inline_size = frame.inline_size
                    - (logical_border.left + logical_border.right)
                    - (logical_padding.left + logical_padding.right);
                self.write_lines(entity, &frame, content_offset, content_inline_size, z + 1)?;
            }
            Content::Replaced => {
                if self.intrinsic_sizes.get(entity).is_some() {
                    self.emit(BlockLevelElement::Leaf(entity));
                }
            }
            Content::Table => {
                // The captions, row groups and rows in document order. The rows and groups
                // contain their cells, which are placed relative to them.
                for &child in node.table.iter().flat_map(|table| &table.boxes) {
                    let child_origin = frame.child_origin(&self.node(child).layout);
                    self.write_box(child, child_origin, flow, z + 1, z_base)?;
                }
                for &child in children {
                    if self.out_of_flow[usize::from(child)] {
                        self.anchors.insert(child, self.last_flow_entity);
                    }
                }
            }
        }

        if anchored {
            self.group_stack.pop();
        }
        self.emit(BlockLevelElement::ContainerEnd(entity));

        // the absolutely positioned boxes this box is the containing block of
        for &child in &node.hoisted {
            let containing_block = if id == self.root && !self.out_of_flow[usize::from(id)] {
                // The initial containing block is on the first page, but a box that is placed
                // by its static position goes on the page of the content in front of it.
                let inset = &self.node(child).style.inset;
                if inset.top.is_auto() && inset.bottom.is_auto() {
                    self.anchors.get(&child).copied().flatten()
                } else {
                    None
                }
            } else {
                Some(entity)
            };
            self.open_group(containing_block, None);
            let child_origin = frame.child_origin(&self.node(child).layout);
            let abs_base = z_base + ABS_Z_STEP;
            self.write_box(child, child_origin, flow, abs_base + 1, abs_base)?;
            self.group_stack.pop();
        }
        Ok(())
    }

    /// Breaks the inline content of a box into lines and places them. The lines are laid out in
    /// the logical axes of `frame`: `content_offset` is the position of the content box relative to
    /// the border box and `width` its inline size.
    fn write_lines(
        &mut self,
        entity: Entity,
        frame: &Frame,
        content_offset: (f32, f32),
        width: f32,
        z: i64,
    ) -> Result<(), String> {
        let style = self.computed_values.get(entity);
        let indent = style
            .map(|s| s.text.text_indent.absolute(PixelLength(width)).0)
            .unwrap_or(0.0);
        let Some(ifc) = self.inline_ctxs.get_mut(entity) else {
            return Ok(());
        };
        ifc.break_lines(
            PixelLength(width),
            PixelLength(indent),
            self.entities,
            self.text_runs,
            self.intrinsic_sizes,
        )
        .map_err(|e| format!("cannot break lines of {:?}: {}", entity, e))?;
        let align = ifc.resolved_text_align();
        let indent_offset = ifc.indent_offset(PixelLength(indent)).0;
        let lines: Vec<_> = ifc
            .lines
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
