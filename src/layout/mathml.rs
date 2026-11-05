//! MathML layout and painting with `math_render`'s host API.
//!
//! The formula's elements are entities with a [`MathNode`] component (built in `construct.rs`).
//! [`EcsMathTree`] exposes them, together with their computed styles, as a
//! [`math_render::host::MathTree`], so mlayout lays out helium's own tree with helium's CSS.

use super::{
    FontMap,
    components::{
        ContainingBlock, Decoration, Glyphs, GraphicsOp, IntrinsicSize, MathBox, MathNode,
        Paragraph, Position, TextOperator, TransformMatrix, VectorGraphics, ZIndex,
    },
    inline_size::calculate_padding_and_border,
    text::font::GlyphIndex,
};
use math_render::{
    host::{
        Direction, Edges, Fragment, LayoutCollector, MathComputedStyle, MathShift, MathTree,
        NodeKind, NodeLayout, OperatorOverrides, Rect, default_operator_overrides, layout_math,
    },
    shaper::HarfbuzzShaper,
};
use specs::{Entities, Entity, Join, Read, ReadStorage, System, WriteStorage};

use crate::style::properties::computed::{
    self, ComputedValues, MathStyle, PixelLength, color_to_rgba,
};
use crate::style::properties::specified::AutoBool;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use PixelLength as Px;

/// helium's ECS world, seen as a math tree.
struct EcsMathTree<'a, 'world> {
    nodes: &'a ReadStorage<'world, MathNode>,
    styles: &'a ReadStorage<'world, ComputedValues>,
    /// One shaper per font face, by font name.
    fonts: HashMap<String, Rc<HarfbuzzShaper<'static>>>,
}

impl<'a, 'world> EcsMathTree<'a, 'world> {
    fn new(
        nodes: &'a ReadStorage<'world, MathNode>,
        styles: &'a ReadStorage<'world, ComputedValues>,
        font_map: &FontMap,
    ) -> Self {
        let mut fonts = HashMap::new();
        for node in nodes.join() {
            fonts.entry(node.font_name.clone()).or_insert_with(|| {
                Rc::new(HarfbuzzShaper::new(font_map[&node.font_name].hb_font.clone()))
            });
        }
        EcsMathTree {
            nodes,
            styles,
            fonts,
        }
    }

    fn node(&self, entity: Entity) -> &'a MathNode {
        self.nodes
            .get(entity)
            .expect("math element without a MathNode")
    }
}

impl MathTree for EcsMathTree<'_, '_> {
    type NodeId = Entity;
    type Font = Rc<HarfbuzzShaper<'static>>;

    fn kind(&self, node: Entity) -> NodeKind {
        self.node(node).kind
    }

    fn children(&self, node: Entity) -> impl Iterator<Item = Entity> {
        self.node(node).children.iter().copied()
    }

    fn child_count(&self, node: Entity) -> usize {
        self.node(node).children.len()
    }

    fn style(&self, node: Entity) -> MathComputedStyle {
        let style = self
            .styles
            .get(node)
            .expect("math element without a computed style");
        math_computed_style(style)
    }

    fn font(&self, node: Entity) -> Self::Font {
        self.fonts[&self.node(node).font_name].clone()
    }

    fn text(&self, node: Entity) -> Option<Cow<'_, str>> {
        self.node(node).text.as_deref().map(Cow::Borrowed)
    }

    fn attribute(&self, node: Entity, name: &str) -> Option<Cow<'_, str>> {
        self.node(node)
            .attributes
            .get(name)
            .map(|value| Cow::Borrowed(value.as_str()))
    }

    fn operator_overrides(&self, node: Entity) -> OperatorOverrides {
        let mut overrides = default_operator_overrides(self, node);
        if let Some(style) = self.styles.get(node) {
            if let Some(form) = style.math.form.into() {
                overrides.form = Some(form);
            }
            match style.math.operator_is_stretchy {
                AutoBool::True => overrides.stretchy = Some(true),
                AutoBool::False => overrides.stretchy = Some(false),
                AutoBool::Auto => {}
            }
        }
        overrides
    }
}

/// The style mlayout needs, from helium's computed values. Percentages of padding and margins
/// resolve against 0, as math elements have no definite containing block width.
fn math_computed_style(style: &ComputedValues) -> MathComputedStyle {
    let deco = calculate_padding_and_border(Px(0.0), style);
    let edges = |e: &super::components::EdgeValues<Px>| Edges {
        top: e.top.0,
        right: e.right.0,
        bottom: e.bottom.0,
        left: e.left.0,
    };
    let position = &style.position;
    MathComputedStyle {
        font_size: style.font.font_size.0,
        math_depth: style.math.depth(),
        math_style: match style.math.math_style {
            MathStyle::Display => math_render::host::MathStyle::Normal,
            MathStyle::Inline => math_render::host::MathStyle::Compact,
        },
        math_shift: match style.math.math_shift {
            computed::MathShift::Normal => MathShift::Normal,
            computed::MathShift::Compact => MathShift::Compact,
        },
        direction: Direction::Ltr,
        padding: edges(&deco.padding),
        border: edges(&deco.border),
        margin: Edges {
            top: deco.margin.top.0,
            right: position.margin_right.absolute(Px(0.0), Px(0.0)).0,
            bottom: deco.margin.bottom.0,
            left: position.margin_left.absolute(Px(0.0), Px(0.0)).0,
        },
    }
}

/// Lays out every `<math>` element. Sets the intrinsic size of the `<math>` box, and the
/// position (relative to the `<math>` box) and decoration of each element inside it.
pub struct MathFormat {}

impl<'world> System<'world> for MathFormat {
    type SystemData = (
        Read<'world, FontMap>,
        Entities<'world>,
        WriteStorage<'world, Position>,
        ReadStorage<'world, MathNode>,
        ReadStorage<'world, ComputedValues>,
        WriteStorage<'world, IntrinsicSize>,
        WriteStorage<'world, MathBox>,
        WriteStorage<'world, ContainingBlock>,
        WriteStorage<'world, Decoration>,
    );
    fn run(
        &mut self,
        (
            font_map,
            entities,
            mut positions,
            math_nodes,
            computed_values,
            mut intrinsic_sizes,
            mut math_boxes,
            mut parents,
            mut decorations,
        ): Self::SystemData,
    ) {
        let roots: Vec<Entity> = (&entities, &math_nodes)
            .join()
            .filter(|(_, node)| {
                node.kind == NodeKind::Math(math_render::host::MathElement::Math)
            })
            .map(|(entity, _)| entity)
            .collect();
        if roots.is_empty() {
            return;
        }

        let tree = EcsMathTree::new(&math_nodes, &computed_values, &font_map);
        for root in roots {
            let mut collector = LayoutCollector::new(&tree);
            let root_layout = layout_math(&mut collector, root);

            intrinsic_sizes
                .insert(
                    root,
                    IntrinsicSize::SizeWithBaseline {
                        width: Px(root_layout.size.width),
                        height: Px(root_layout.size.height),
                        baseline: Px(root_layout.baseline),
                    },
                )
                .unwrap();

            // Walk the tree top-down to turn the parent-relative locations into offsets from
            // the `<math>` box.
            let mut layouts = collector.layouts;
            let mut nodes = Vec::with_capacity(layouts.len());
            let mut stack = vec![(root, 0.0f32, 0.0f32)];
            while let Some((entity, parent_x, parent_y)) = stack.pop() {
                let Some(layout) = layouts.remove(&entity) else {
                    // not laid out, e.g. the children of foreign content
                    continue;
                };
                let (x, y) = if entity == root {
                    (0.0, 0.0)
                } else {
                    (parent_x + layout.location.x, parent_y + layout.location.y)
                };

                if entity != root {
                    positions
                        .insert(
                            entity,
                            Position {
                                x: Px(x),
                                y: Px(y),
                                width: Px(layout.size.width),
                                height: Px(layout.size.height),
                                z: ZIndex(0),
                                page: 0,
                            },
                        )
                        .unwrap();
                    parents
                        .insert(entity, ContainingBlock { entity: root })
                        .unwrap();
                    let decoration =
                        calculate_padding_and_border(Px(0.0), computed_values.get(entity).unwrap());
                    decorations.insert(entity, decoration).unwrap();
                }

                // reversed, so that the nodes come out in tree order
                for &child in math_nodes.get(entity).unwrap().children.iter().rev() {
                    stack.push((child, x, y));
                }
                nodes.push((entity, Px(x), Px(y), layout));
            }

            math_boxes.insert(root, MathBox { nodes }).unwrap();
        }
    }
}

/// Paints the glyphs and rules of every laid out formula.
pub struct LayoutMathGlyphs {}

impl LayoutMathGlyphs {
    #[allow(clippy::too_many_arguments)]
    fn paint_node(
        para: &mut Paragraph,
        graphics: &mut VectorGraphics,
        font_map: &FontMap,
        node: &MathNode,
        style: Option<&ComputedValues>,
        x: Px,
        y: Px,
        layout: &NodeLayout,
        to_unicode_maps: &mut HashMap<String, BTreeMap<GlyphIndex, String>>,
    ) {
        let mut color_set = false;
        for fragment in &layout.fragments {
            match fragment {
                Fragment::Glyphs {
                    origin,
                    font_size,
                    glyphs,
                } => {
                    let font = &font_map[&node.font_name];
                    if !color_set {
                        if let Some(style) = style {
                            para.ops
                                .push(TextOperator::SetColor(color_to_rgba(style.font.color)));
                        }
                        color_set = true;
                    }
                    para.ops.push(TextOperator::SetFont(
                        node.font_name.clone(),
                        Px(*font_size),
                    ));

                    let to_unicode_map = to_unicode_maps
                        .entry(node.font_name.clone())
                        .or_default();
                    let text = node.text.as_deref();
                    let mut previous_cluster = u32::MAX;
                    let mut cluster_starting_glyph = 0;
                    for glyph in glyphs {
                        let transform = TransformMatrix::translation(
                            x + Px(origin.x + glyph.offset.x),
                            y + Px(origin.y + glyph.offset.y),
                        );
                        para.ops.push(TextOperator::SetTransform(transform));
                        let glyph_index = GlyphIndex(glyph.glyph);
                        let _ = font.advance(glyph_index);
                        para.ops
                            .push(TextOperator::Glyphs(Glyphs(vec![glyph_index])));

                        if let Some(text) = text {
                            if glyph.cluster != previous_cluster {
                                // a new cluster began
                                if previous_cluster < glyph.cluster {
                                    if let Some(cluster_text) = text
                                        .get(previous_cluster as usize..glyph.cluster as usize)
                                    {
                                        to_unicode_map.insert(
                                            GlyphIndex(cluster_starting_glyph),
                                            cluster_text.to_owned(),
                                        );
                                    }
                                }
                                cluster_starting_glyph = glyph.glyph;
                                previous_cluster = glyph.cluster;
                            }
                        }
                    }
                    if let Some(text) = text {
                        if let Some(cluster_text) = text.get(previous_cluster as usize..) {
                            to_unicode_map
                                .insert(GlyphIndex(cluster_starting_glyph), cluster_text.to_owned());
                        }
                    }
                }
                Fragment::Rule(rect) => push_rule(graphics, x, y, rect),
                Fragment::DashedRule(rect) => {
                    // dashes three times as long as the line is thick
                    let thickness = rect.size.width.min(rect.size.height);
                    let horizontal = rect.size.width >= rect.size.height;
                    let length = rect.size.width.max(rect.size.height);
                    let dash = (3.0 * thickness).max(0.5);
                    let mut start = 0.0;
                    while start < length {
                        let end = (start + dash).min(length);
                        let piece = if horizontal {
                            Rect::new(rect.origin.x + start, rect.origin.y, end - start, thickness)
                        } else {
                            Rect::new(rect.origin.x, rect.origin.y + start, thickness, end - start)
                        };
                        push_rule(graphics, x, y, &piece);
                        start += 2.0 * dash;
                    }
                }
            }
        }
    }
}

/// Paints a filled rectangle (relative to `(x, y)`) as a line along its longer side.
fn push_rule(graphics: &mut VectorGraphics, x: Px, y: Px, rect: &Rect) {
    if rect.size.width >= rect.size.height {
        let center = rect.origin.y + rect.size.height / 2.0;
        graphics.ops.push(GraphicsOp::Line {
            from_x: x + Px(rect.origin.x),
            from_y: y + Px(center),
            to_x: x + Px(rect.right()),
            to_y: y + Px(center),
            thickness: Px(rect.size.height),
        });
    } else {
        let center = rect.origin.x + rect.size.width / 2.0;
        graphics.ops.push(GraphicsOp::Line {
            from_x: x + Px(center),
            from_y: y + Px(rect.origin.y),
            to_x: x + Px(center),
            to_y: y + Px(rect.bottom()),
            thickness: Px(rect.size.width),
        });
    }
}

impl<'world> System<'world> for LayoutMathGlyphs {
    type SystemData = (
        Entities<'world>,
        ReadStorage<'world, MathBox>,
        ReadStorage<'world, MathNode>,
        ReadStorage<'world, Position>,
        WriteStorage<'world, Paragraph>,
        WriteStorage<'world, VectorGraphics>,
        ReadStorage<'world, ComputedValues>,
        Read<'world, FontMap>,
    );
    fn run(
        &mut self,
        (
            entities,
            math_boxes,
            math_nodes,
            positions,
            mut paragraphs,
            mut vector_graphics,
            computed_values,
            font_map,
        ): Self::SystemData,
    ) {
        let mut to_unicode_maps = HashMap::new();
        for (entity, math_box, pos) in (&entities, &math_boxes, &positions).join() {
            let mut para = Paragraph::new(Px(0.0), Px(0.0));
            let mut vg = VectorGraphics {
                color: computed_values
                    .get(entity)
                    .map(|style| color_to_rgba(style.font.color)),
                ..Default::default()
            };
            for (node_entity, x, y, layout) in &math_box.nodes {
                let Some(node) = math_nodes.get(*node_entity) else {
                    continue;
                };
                Self::paint_node(
                    &mut para,
                    &mut vg,
                    &font_map,
                    node,
                    computed_values.get(*node_entity),
                    pos.x + *x,
                    pos.y + *y,
                    layout,
                    &mut to_unicode_maps,
                );
            }
            paragraphs.insert(entity, para).unwrap();
            vector_graphics.insert(entity, vg).unwrap();
        }
        for (font_name, mut map) in to_unicode_maps {
            font_map[&font_name].to_unicode.lock().append(&mut map);
        }
    }
}
