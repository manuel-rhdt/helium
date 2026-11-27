//! MathML layout and painting with `formel`'s host API.
//!
//! The formula's elements are entities with a [`MathNode`] component (built in `construct.rs`).
//! [`EcsMathTree`] exposes them, together with their computed styles, as a
//! [`formel::host::MathTree`], so formel lays out umbruch's own tree with umbruch's CSS.

use super::{
    FontMap,
    components::{
        ContainingBlock, Decoration, GraphicsOp, ImageBox, IntrinsicSize, MathBox, MathNode, MathSegment,
        NO_CLUSTER, PieceKind, Paragraph, Position, TextOperator, TransformMatrix, VectorGraphics, ZIndex,
    },
    inline_size::calculate_padding_and_border,
    text::font::GlyphIndex,
};
use formel::{
    host::{
        Direction, Edges, FontFeatures, ForeignMetrics, Fragment, LayoutCollector, MathComputedStyle,
        MathLayoutHost, MathShift, MathTree, NodeKind, NodeLayout, OperatorOverrides, Rect,
        TokenSegment, axis_height, default_operator_overrides, layout_math,
    },
    shaper::Shaper,
};
use specs::{Entities, Entity, Join, Read, ReadStorage, System, WriteStorage};

use crate::style::properties::computed::{
    self, ComputedValues, MathStyle, PixelLength, color_to_paint,
};
use crate::style::properties::specified::AutoBool;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

use PixelLength as Px;

/// umbruch's ECS world, seen as a math tree.
struct EcsMathTree<'a, 'world> {
    nodes: &'a ReadStorage<'world, MathNode>,
    styles: &'a ReadStorage<'world, ComputedValues>,
    images: &'a ReadStorage<'world, ImageBox>,
    /// One shaper per font face, by font name.
    fonts: HashMap<String, Rc<Shaper>>,
}

impl<'a, 'world> EcsMathTree<'a, 'world> {
    fn new(
        nodes: &'a ReadStorage<'world, MathNode>,
        styles: &'a ReadStorage<'world, ComputedValues>,
        images: &'a ReadStorage<'world, ImageBox>,
        font_map: &'a FontMap,
    ) -> Self {
        let mut fonts = HashMap::new();
        for node in nodes.join() {
            fonts.entry(node.font_name.clone()).or_insert_with(|| {
                let font = &font_map[&node.font_name];
                Rc::new(
                    Shaper::from_slice(font.data.as_slice(), font.index)
                        .expect("the math font has a MATH table"),
                )
            });
        }
        EcsMathTree {
            nodes,
            styles,
            images,
            fonts,
        }
    }

    fn node(&self, entity: Entity) -> &'a MathNode {
        self.nodes
            .get(entity)
            .expect("math element without a MathNode")
    }
}

impl<'a> MathTree for EcsMathTree<'a, '_> {
    type NodeId = Entity;
    type Font = Rc<Shaper>;

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

    fn token_content(&self, node: Entity) -> Option<Vec<TokenSegment<Entity>>> {
        let segments = self.node(node).segments.as_ref()?;
        Some(
            segments
                .iter()
                .map(|segment| match segment {
                    MathSegment::Text(text) => TokenSegment::Text(text.clone()),
                    MathSegment::Foreign(entity) => TokenSegment::Foreign(*entity),
                })
                .collect(),
        )
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

impl EcsMathTree<'_, '_> {
    /// The margin box of foreign content. Only replaced content is laid out: an image is as big
    /// as its `width` and `height` say, else as its intrinsic size (keeping the ratio when only
    /// one of them is given), and sits on the baseline with its bottom margin edge. Padding and
    /// border of the image are not taken into account. Anything else has no size. See below for
    /// `vertical-align`.
    fn foreign_metrics(&self, entity: Entity) -> ForeignMetrics {
        use computed::GenericSize;
        let (Some(image), Some(style)) = (self.images.get(entity), self.styles.get(entity)) else {
            return ForeignMetrics::default();
        };
        let (natural_width, natural_height) = (image.width, image.height);
        let ratio = if natural_height > 0.0 { natural_width / natural_height } else { 0.0 };
        let specified = |size: GenericSize| {
            (!matches!(size, GenericSize::Auto)).then(|| size.absolute(Px(0.0), Px(0.0)).0.max(0.0))
        };
        let (width, height) = match (specified(style.position.width), specified(style.position.height)) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) if ratio > 0.0 => (width, width / ratio),
            (None, Some(height)) => (height * ratio, height),
            _ => (natural_width, natural_height),
        };
        let deco = calculate_padding_and_border(Px(0.0), style);
        let margin_left = style.position.margin_left.absolute(Px(0.0), Px(0.0)).0;
        let margin_right = style.position.margin_right.absolute(Px(0.0), Px(0.0)).0;
        // `vertical-align` raises the box: a length or a percentage of the `line-height` as in
        // CSS, `middle` puts its middle on the math axis (CSS puts it half an x-height above the
        // baseline; the axis is what a formula is centered on). The other values refer to a line
        // box, which a formula does not have: they are the baseline.
        let shift = match style.box_.vertical_align {
            computed::VerticalAlign::LengthPercentage(length) => {
                let line_height = match style.font.line_height {
                    computed::LineHeight::LengthPercentage(line_height) => {
                        line_height.absolute(style.font.font_size).0
                    }
                    computed::LineHeight::Normal => 1.2 * style.font.font_size.0,
                };
                length.absolute(Px(line_height)).0
            }
            computed::VerticalAlign::Middle => axis_height(self, entity) - height / 2.0,
            _ => 0.0,
        };
        ForeignMetrics {
            width: margin_left + width + margin_right,
            ascent: deco.margin.top.0 + height + shift,
            descent: deco.margin.bottom.0 - shift,
        }
    }
}

/// The math tree with a host that lays out foreign content, and collects the layouts.
struct EcsMathHost<'a, 'world> {
    collector: LayoutCollector<EcsMathTree<'a, 'world>>,
}

impl<'a> MathTree for EcsMathHost<'a, '_> {
    type NodeId = Entity;
    type Font = Rc<Shaper>;

    fn kind(&self, node: Entity) -> NodeKind {
        self.collector.kind(node)
    }
    fn children(&self, node: Entity) -> impl Iterator<Item = Entity> {
        self.collector.children(node)
    }
    fn child_count(&self, node: Entity) -> usize {
        self.collector.child_count(node)
    }
    fn style(&self, node: Entity) -> MathComputedStyle {
        self.collector.style(node)
    }
    fn font(&self, node: Entity) -> Self::Font {
        self.collector.font(node)
    }
    fn text(&self, node: Entity) -> Option<Cow<'_, str>> {
        self.collector.text(node)
    }
    fn token_content(&self, node: Entity) -> Option<Vec<TokenSegment<Entity>>> {
        self.collector.token_content(node)
    }
    fn attribute(&self, node: Entity, name: &str) -> Option<Cow<'_, str>> {
        self.collector.attribute(node, name)
    }
    fn operator_overrides(&self, node: Entity) -> OperatorOverrides {
        self.collector.operator_overrides(node)
    }
}

impl MathLayoutHost for EcsMathHost<'_, '_> {
    fn layout_foreign(&mut self, node: Entity) -> ForeignMetrics {
        self.collector.tree.foreign_metrics(node)
    }
    fn set_layout(&mut self, node: Entity, layout: NodeLayout) {
        self.collector.set_layout(node, layout)
    }
}

/// The style mlayout needs, from umbruch's computed values. Percentages of padding and margins
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
    // formel keeps at most `FontFeatures::CAPACITY` settings; a later setting of a tag wins.
    let mut font_features = FontFeatures::new();
    for &(tag, value) in &style.font.font_feature_settings.0 {
        font_features.set(tag, value);
    }
    MathComputedStyle {
        font_size: style.font.font_size.0,
        font_features,
        math_depth: style.math.depth(),
        math_style: match style.math.math_style {
            MathStyle::Normal => formel::host::MathStyle::Normal,
            MathStyle::Compact => formel::host::MathStyle::Compact,
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
        ReadStorage<'world, ImageBox>,
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
            images,
            mut intrinsic_sizes,
            mut math_boxes,
            mut parents,
            mut decorations,
        ): Self::SystemData,
    ) {
        let roots: Vec<Entity> = (&entities, &math_nodes)
            .join()
            .filter(|(_, node)| node.kind == NodeKind::Math(formel::host::MathElement::Math))
            .map(|(entity, _)| entity)
            .collect();
        if roots.is_empty() {
            return;
        }

        for root in roots {
            let mut host = EcsMathHost {
                collector: LayoutCollector::new(EcsMathTree::new(
                    &math_nodes,
                    &computed_values,
                    &images,
                    &font_map,
                )),
            };
            let root_layout = layout_math(&mut host, root);

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
            let mut layouts = host.collector.layouts;
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
                let children: Vec<Entity> = math_nodes.get(entity).unwrap().child_entities().collect();
                for child in children.into_iter().rev() {
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
        node: &MathNode,
        style: Option<&ComputedValues>,
        x: Px,
        y: Px,
        layout: &NodeLayout,
    ) {
        let mut color_set = false;
        for fragment in &layout.fragments {
            match fragment {
                Fragment::Glyphs {
                    origin,
                    font_size,
                    glyphs,
                } => {
                    if !color_set {
                        if let Some(style) = style {
                            para.ops
                                .push(TextOperator::SetColor(color_to_paint(style.font.color)));
                        }
                        color_set = true;
                    }
                    para.ops.push(TextOperator::SetFont(
                        node.font_name.clone(),
                        Px(*font_size),
                    ));

                    // the pieces of a stretched operator have no text of their own
                    let text = node.text.as_deref();
                    let first_glyph = para.ops.glyphs.len();
                    for glyph in glyphs {
                        let transform = TransformMatrix::translation(
                            x + Px(origin.x + glyph.offset.x),
                            y + Px(origin.y + glyph.offset.y),
                        );
                        para.ops.push(TextOperator::SetTransform(transform));
                        let cluster = if text.is_some() { glyph.cluster } else { NO_CLUSTER };
                        para.ops.push_glyph(GlyphIndex(glyph.glyph), cluster);
                    }
                    let glyphs = first_glyph..para.ops.glyphs.len();
                    para.ops.push_piece(glyphs, text.unwrap_or(""), false, PieceKind::Formula);
                }
                Fragment::Rule(rect) => push_rule(graphics, x, y, rect),
                Fragment::Line {
                    start,
                    end,
                    thickness,
                } => graphics.ops.push(GraphicsOp::Line {
                    from_x: x + Px(start.x),
                    from_y: y + Px(start.y),
                    to_x: x + Px(end.x),
                    to_y: y + Px(end.y),
                    thickness: Px(*thickness),
                }),
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
        Read<'world, crate::tagging::StructureTree>,
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
            structure,
        ): Self::SystemData,
    ) {
        for (entity, math_box, pos) in (&entities, &math_boxes, &positions).join() {
            let mut para = Paragraph::new(Px(0.0), Px(0.0));
            let mut vg = VectorGraphics {
                color: computed_values
                    .get(entity)
                    .map(|style| color_to_paint(style.font.color)),
                ..Default::default()
            };
            for (node_entity, x, y, layout) in &math_box.nodes {
                let Some(node) = math_nodes.get(*node_entity) else {
                    continue;
                };
                Self::paint_node(
                    &mut para,
                    &mut vg,
                    node,
                    computed_values.get(*node_entity),
                    pos.x + *x,
                    pos.y + *y,
                    layout,
                );
            }
            // the formula is tagged as a whole
            if let Some(tag) = structure.content_tag(entity, 0) {
                para.ops.insert(0, TextOperator::BeginTag(tag));
                para.ops.push(TextOperator::EndMarkedContent);
            }
            paragraphs.insert(entity, para).unwrap();
            vector_graphics.insert(entity, vg).unwrap();
        }
    }
}
