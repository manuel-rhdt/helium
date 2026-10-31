use super::components::{
    self, BlockLevelElement, LayoutEntity, MathExpression, Position, RootNode,
};
use super::inline::InlineFormattingContext;
use super::margin_boxes::{self, MarginBoxes, ResolvedMarginBox};
use super::normal_formatting::BlockFormattingContext;
use super::{text::Font, FontMap};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::{MarginBoxKind, PropertyDeclarationBlock, Styles};
use crate::style::properties::computed::{
    color_to_rgba, ComputedValues, Display, LineHeight, PositionSpec, StyleBuilder, TextAlign,
};
use crate::style::properties::specified::{MathFamily, PropertyDeclaration, Text};
use crate::traverse::Traversal;
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{local_name, ns, Attribute, QualName};
use math_render::{mathmlparser, Field};
use specs::{Builder, Entities, Entity, SystemData, World, WorldExt, WriteStorage};

use components::TextRun;
use std::collections::HashMap;

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
pub struct FlowConstructor {
    pub(crate) styles: Styles,
    entity_constructor: EntityConstructor,
}

impl FlowConstructor {
    pub fn new(world: World) -> Self {
        FlowConstructor {
            styles: Default::default(),
            entity_constructor: EntityConstructor::new(world),
        }
    }

    /// Resolves the styles and fonts of the margin boxes (headers and footers) of the document.
    pub fn resolve_margin_boxes(&mut self) -> MarginBoxes {
        let mut resolve = |first: bool, left: bool| {
            let mut resolved = vec![];
            for kind in MarginBoxKind::ALL {
                let mut style_builder = StyleBuilder::new(Default::default());
                let mut content = None;
                let mut text_align = None;
                for rule in self.styles.rules_iter() {
                    let Some(selector) = rule.page_selector() else { continue };
                    if !margin_boxes::selector_matches(selector, first, left) {
                        continue;
                    }
                    for margin_box in rule.margin_boxes.iter().filter(|b| b.kind == kind) {
                        for declaration in margin_box.properties.declarations() {
                            if let PropertyDeclaration::Text(Text::TextAlign(align)) = declaration {
                                text_align = Some(*align);
                            }
                            style_builder.add_declaration(declaration);
                        }
                        if margin_box.content.is_some() {
                            content = margin_box.content.clone();
                        }
                    }
                }
                let content = match content {
                    Some(content) if !content.is_empty() => content,
                    _ => continue,
                };
                let style = style_builder.build();
                let font_name = self.entity_constructor.match_font(&style).postscript_name();
                resolved.push(ResolvedMarginBox {
                    kind,
                    content,
                    font_name,
                    font_size: style.font.font_size,
                    color: color_to_rgba(style.font.color),
                    text_align: text_align.unwrap_or(match kind {
                        MarginBoxKind::TopCenter | MarginBoxKind::BottomCenter => TextAlign::Center,
                        MarginBoxKind::TopRight | MarginBoxKind::BottomRight => TextAlign::Right,
                        MarginBoxKind::TopLeft | MarginBoxKind::BottomLeft => TextAlign::Left,
                    }),
                });
            }
            resolved
        };
        // the first page is a right page
        MarginBoxes {
            first: resolve(true, false),
            left: resolve(false, true),
            right: resolve(false, false),
        }
    }

    pub fn layout_context(self) -> super::components::LayoutContext {
        self.entity_constructor.world
    }

    fn read_stylesheet(&mut self, node: &DomRef) {
        if let ParsedData::Element(ElementData {
            name, attributes, ..
        }) = &node.html_data
        {
            if name == &QualName::new(None, ns!(html), local_name!("style")) {
                for child in TreeLinked::children(node) {
                    if let ParsedData::Text(css) = &child.html_data {
                        self.styles.parse_css(&css.borrow())
                    } else {
                        log::warn!("<style> element did not contain any style information.")
                    }
                }
            }
            if name == &QualName::new(None, ns!(html), local_name!("link")) {
                if let Some(Attribute { value, .. }) =
                    attributes
                        .borrow()
                        .get(&QualName::new(None, ns!(), local_name!("rel")))
                {
                    if value.as_ref() == "stylesheet" {
                        if let Some(Attribute { value, .. }) = attributes
                            .borrow()
                            .get(&QualName::new(None, ns!(), local_name!("href")))
                        {
                            self.styles.add_stylesheet(value.as_ref())
                        }
                    }
                }
            }
        }
    }
}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, node: DomRef) {
        self.read_stylesheet(&node);

        // TODO: Selector matching, Cascade
        match &node.clone().into_element_ref() { Some(element_ref) => {
            let parent_style = element_ref
                .as_node()
                .parent()
                .and_then(|parent| NodeData::style(&parent));
            let mut style_builder = StyleBuilder::new(parent_style.unwrap_or_default());
            for matched_rule in self
                .styles
                .rules_iter()
                .filter(|rule| rule.matches(element_ref))
            {
                let properties = &matched_rule.properties;
                for declaration in properties.declarations() {
                    style_builder.add_declaration(declaration);
                }
            }
            // declarations in the `style` attribute override all matched rules
            let inline_style = element_ref
                .attributes
                .borrow()
                .get(&QualName::new(None, ns!(), local_name!("style")))
                .map(|attr| PropertyDeclarationBlock::parse_inline(&attr.value));
            if let Some(block) = &inline_style {
                for declaration in block.declarations() {
                    style_builder.add_declaration(declaration);
                }
            }
            element_ref.as_node().set_style(style_builder.build());
        } _ => {
            let parent_style = node.parent().and_then(|parent| NodeData::style(&parent));
            let style = StyleBuilder::new(parent_style.unwrap_or_default()).build();
            node.set_style(style);
        }}

        self.entity_constructor.build_entity(node)
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        match node.html_data() {
            ParsedData::Element(_) => self.entity_constructor.build_formatting_context(&node),
            _ => {}
        }
    }
}

pub trait AssignLayoutEntity {
    fn assign_layout_entity(&mut self, entity: LayoutEntity);
    fn layout_entity(&self) -> Option<LayoutEntity>;
}

impl AssignLayoutEntity for DomRef {
    fn assign_layout_entity(&mut self, entity: LayoutEntity) {
        self.layout_entity.set(Some(entity))
    }

    fn layout_entity(&self) -> Option<LayoutEntity> {
        self.layout_entity.get()
    }
}

pub trait Styled {
    fn style(&self) -> ComputedValues;
}

impl Styled for DomRef {
    fn style(&self) -> ComputedValues {
        NodeData::style(&self).expect("No style was assigned to DOM node.")
    }
}

pub trait HtmlNode: Sized {
    fn html_data(&self) -> &ParsedData;
}

impl HtmlNode for DomRef {
    fn html_data(&self) -> &ParsedData {
        &self.html_data
    }
}

pub trait IterChildren<'a>: Sized {
    type ChildrenIter: 'a + Iterator<Item = Self>;
    fn children(&'a self) -> Self::ChildrenIter;

    fn next_sibling(&self) -> Option<Self>;
    fn previous_sibling(&self) -> Option<Self>;
}

impl<'a> IterChildren<'a> for DomRef {
    type ChildrenIter = crate::tree::SiblingIter<DomRef>;
    fn children(&'a self) -> Self::ChildrenIter {
        TreeLinked::children(&self)
    }

    fn next_sibling(&self) -> Option<Self> {
        crate::tree::TreeLinked::next_sibling(&self)
    }

    fn previous_sibling(&self) -> Option<Self> {
        crate::tree::TreeLinked::previous_sibling(&self)
    }
}

pub trait AbstractNode<'a>: AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a> {}

impl<'a, T: 'a + AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a>> AbstractNode<'a> for T {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FontSelector {
    families: Vec<crate::style::properties::computed::FontFamily>,
    weight: crate::style::properties::computed::FontWeight,
    style: crate::style::properties::computed::FontStyle,
}

impl FontSelector {
    fn from_style(style: &ComputedValues) -> Self {
        FontSelector {
            families: style.font.font_family.clone(),
            weight: style.font.font_weight.clone(),
            style: style.font.font_style.clone(),
        }
    }
}

pub struct EntityConstructor {
    font_match_cache: HashMap<FontSelector, String>,
    mathml_context: Option<mathmlparser::ParseContext>,
    pub world: World,
    trim_next_inline_whitespace: bool,
}

impl std::fmt::Debug for EntityConstructor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("EntityConstructor").finish()
    }
}

impl EntityConstructor {
    pub fn new(mut world: World) -> Self {
        world.register::<ComputedValues>();
        world.register::<InlineFormattingContext>();
        world.register::<BlockFormattingContext>();
        world.register::<Position>();
        world.register::<components::TextRun>();
        world.register::<components::ListMarker>();
        world.register::<components::Decoration>();
        world.register::<MathExpression>();
        world.insert(FontMap::new());
        world.insert(RootNode(None));
        EntityConstructor {
            world,
            font_match_cache: Default::default(),
            mathml_context: None,
            trim_next_inline_whitespace: true,
        }
    }

    fn match_font(&mut self, style: &ComputedValues) -> Font {
        // TODO: handle error
        let font_selector = FontSelector::from_style(&style);
        let mut font_map = self.world.write_resource::<FontMap>();
        let font_name = self
            .font_match_cache
            .entry(font_selector)
            .or_insert_with(|| {
                let font = crate::layout::text::font::match_font(&style)
                    .expect("Could not find any matching fonts.");
                let font_name = font.postscript_name();
                font_map.insert(font_name.clone(), font);
                font_name
            });
        font_map[font_name].clone()
    }

    // Called on top-down traversal. Takes a node and its style to create a corresponding
    // `LayoutEntity`, and assigns the entity to the node.
    pub fn build_entity<'a>(&mut self, mut node: impl AbstractNode<'a>) {
        let style = node.style();

        if style.box_.display == Display::None {
            return;
        }

        let entity = match node.html_data() {
            ParsedData::Element(element_data) => {
                if element_data.name == QualName::new(None, ns!(mathml), local_name!("math")) {
                    self.mathml_context = Some(mathmlparser::ParseContext::default());
                }
                if element_data.name == QualName::new(None, ns!(html), local_name!("br")) {
                    // treat <br> as whitespace
                    self.trim_next_inline_whitespace = true;
                }

                let is_list_item = element_data.name == QualName::new(None, ns!(html), local_name!("li"));
                let ordinal = if is_list_item {
                    // the number of `li` siblings up to and including this one
                    let mut count = 1;
                    let mut sibling = node.previous_sibling();
                    while let Some(s) = sibling {
                        if let ParsedData::Element(data) = s.html_data() {
                            if data.name == QualName::new(None, ns!(html), local_name!("li")) {
                                count += 1;
                            }
                        }
                        sibling = s.previous_sibling();
                    }
                    count
                } else {
                    0
                };
                let marker_text = style.font.list_style_type.marker_text(ordinal).filter(|_| is_list_item);
                let marker = marker_text.map(|text| {
                    let font_name = self.match_font(&style).postscript_name();
                    components::ListMarker {
                        text,
                        font_name,
                        font_size: style.font.font_size,
                        color: crate::style::properties::computed::color_to_rgba(style.font.color),
                    }
                });

                if style.box_.display == Display::InlineBlock {
                    // its contents are a block of their own
                    self.trim_next_inline_whitespace = true;
                }
                let entity = self.build_default_entity(style);
                if let Some(marker) = marker {
                    let LayoutEntity(e) = entity;
                    self.world.write_storage::<components::ListMarker>().insert(e, marker).unwrap();
                }
                entity
            }
            ParsedData::Document(_) => self.build_root_entity(),
            ParsedData::Text(string) => {
                // TODO: implement proper whitespace algorithm

                let mut text = string.borrow_mut();

                match node.previous_sibling() {
                    Some(node) if matches!(node.style().box_.display, Display::Inline | Display::InlineBlock) => {}
                    _ => self.trim_next_inline_whitespace = true,
                };

                super::text::whitespace::transform_text(
                    &mut text,
                    self.trim_next_inline_whitespace,
                );

                if &*text != text.trim_end() {
                    // this text node ends with whitespace
                    self.trim_next_inline_whitespace = true
                } else {
                    self.trim_next_inline_whitespace = false
                }

                if text.is_empty() {
                    return;
                } else {
                    self.build_text_entity(style, &*text)
                }
            }
            _ => self.build_default_entity(style),
        };

        node.assign_layout_entity(entity);
    }

    // called on top-down traversal
    fn build_root_entity(&mut self) -> LayoutEntity {
        self.build_default_entity(Default::default())
    }

    // called on top-down traversal
    fn build_default_entity(&mut self, style: ComputedValues) -> LayoutEntity {
        self.world.create_entity().with(style).build().into()
    }

    fn build_text_entity(&mut self, style: ComputedValues, text: &str) -> LayoutEntity {
        let font = self.match_font(&style);
        let font_name = font.postscript_name();

        let font_size = style.font.font_size;
        let font_height = font.scale_to_px(font.font_height(), font_size);
        let line_height = match style.font.line_height {
            LineHeight::Normal => font_height * 1.2,
            LineHeight::LengthPercentage(lp) => lp.absolute(style.font.font_size),
        };
        let line_gap = line_height - font_height;
        let baseline = font.scale_to_px(font.ascent(), font_size) + line_gap / 2.0;

        let color = crate::style::properties::computed::color_to_rgba(style.font.color);
        let decoration = style.font.text_decoration;
        let entity = self
            .world
            .create_entity()
            .with(style)
            .with(components::TextRun {
                font_name,
                text: text.to_owned(),
                font_size,
                line_height,
                baseline,
                color,
                decoration,
            })
            .build();
        LayoutEntity(entity)
    }

    // Called on bottom-up traversal. Traverses the node's children and decides (based on their
    // computed styles) whether to build a block formatting context or an inline formatting
    // context.
    pub fn build_formatting_context<'a, Node: AbstractNode<'a>>(&mut self, node: &'a Node) {
        if node.layout_entity().is_none() {
            return;
        }

        if self.mathml_context.is_some() {
            self.build_math_expression(node);
            return;
        }

        if node.style().box_.display == Display::InlineBlock {
            // an inline-block box is content, so the whitespace after it must be kept
            self.trim_next_inline_whitespace = false;
        }

        let is_display_inline = |node: &Node| match node.style().box_.display {
            Display::Inline => true,
            _ => false,
        };

        if is_display_inline(node) {
            self.build_inline_formatting_context(node)
        } else {
            self.build_block_formatting_context(node)
        }
    }

    fn build_anonymous_block(
        entities: &Entities,
        inline_formatting_contexts: &mut WriteStorage<InlineFormattingContext>,
        computed_values: &mut WriteStorage<ComputedValues>,
        elements: &[Entity],
        style: &ComputedValues,
    ) -> Entity {
        // build anonymous block element
        assert!(style.box_.display == Display::Block);
        entities
            .build_entity()
            .with(
                InlineFormattingContext {
                    elements: elements.to_owned(),
                    absolutes: vec![],
                    break_points: vec![],
                    lines: vec![],
                    atom_offsets: vec![],
                    text_align: style.text.text_align,
                },
                inline_formatting_contexts,
            )
            .with(style.clone(), computed_values)
            .build()
    }

    fn build_blocks(
        elem_iter: impl Iterator<Item = Entity>,
        inline_level_elements: &mut Vec<Entity>,
        block_level_elements: &mut Vec<BlockLevelElement>,
        style: &ComputedValues,
        entities: &Entities,
        computed_values: &mut WriteStorage<ComputedValues>,
        inline_formatting_contexts: &mut WriteStorage<InlineFormattingContext>,
        block_formatting_contexts: &mut WriteStorage<BlockFormattingContext>,
    ) {
        let push_block =
            |ble: &mut Vec<BlockLevelElement>,
             entity,
             block_formatting_contexts: &mut WriteStorage<BlockFormattingContext>,
             inline_formatting_contexts: &WriteStorage<InlineFormattingContext>| {
                if let Some(child_bfc) = block_formatting_contexts.remove(entity) {
                    ble.extend_from_slice(&child_bfc.elements);
                } else {
                    ble.push(BlockLevelElement::ContainerStart(entity));
                    if let Some(ifc) = inline_formatting_contexts.get(entity) {
                        ble.extend(ifc.absolutes.iter().map(|&a| BlockLevelElement::Absolute(a)));
                    }
                    ble.push(BlockLevelElement::Leaf(entity));
                    ble.push(BlockLevelElement::ContainerEnd(entity));
                }
            };

        for elem in elem_iter {
            let child_style = computed_values.get(elem);
            let display = child_style
                .map(|s| s.box_.display)
                .unwrap_or(Display::Inline);
            let position = child_style.map(|s| s.box_.position).unwrap_or_default();

            if position == PositionSpec::Absolute {
                block_level_elements.push(BlockLevelElement::Absolute(elem));
                continue;
            }

            if display == Display::InlineBlock {
                // an atomic inline-level box: its contents stay in its own formatting context
                inline_level_elements.push(elem);
                continue;
            }

            if display == Display::Block {
                if !inline_level_elements.is_empty() {
                    push_block(
                        block_level_elements,
                        Self::build_anonymous_block(
                            entities,
                            inline_formatting_contexts,
                            computed_values,
                            inline_level_elements,
                            style,
                        ),
                        block_formatting_contexts,
                        inline_formatting_contexts,
                    );
                    inline_level_elements.clear();
                }
                push_block(block_level_elements, elem, block_formatting_contexts, inline_formatting_contexts);
            } else {
                if let Some(InlineFormattingContext { elements, absolutes, .. }) =
                    inline_formatting_contexts.remove(elem)
                {
                    block_level_elements
                        .extend(absolutes.into_iter().map(BlockLevelElement::Absolute));
                    Self::build_blocks(
                        elements.iter().cloned(),
                        inline_level_elements,
                        block_level_elements,
                        style,
                        entities,
                        computed_values,
                        inline_formatting_contexts,
                        block_formatting_contexts,
                    );
                } else {
                    inline_level_elements.push(elem);
                }
            }
        }
    }

    // called on bottom-up traversal
    fn build_block_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let entities = self.world.entities();
        let mut block_formatting_contexts = self.world.write_storage::<BlockFormattingContext>();
        let mut inline_formatting_contexts = self.world.write_storage::<InlineFormattingContext>();
        let mut computed_values = self.world.write_storage::<ComputedValues>();

        let LayoutEntity(entity) = node.layout_entity().unwrap();

        let style = computed_values.get(entity).unwrap().clone();

        let mut block_level_elements = vec![];
        let mut inline_level_elements = vec![];

        let entity_iter = node
            .children()
            .filter_map(|child| child.layout_entity().map(|LayoutEntity(ent)| ent));

        Self::build_blocks(
            entity_iter,
            &mut inline_level_elements,
            &mut block_level_elements,
            &style,
            &entities,
            &mut computed_values,
            &mut inline_formatting_contexts,
            &mut block_formatting_contexts,
        );

        let has_block_level_elements = block_level_elements
            .iter()
            .any(|elem| !matches!(elem, BlockLevelElement::Absolute(_)));
        if !has_block_level_elements {
            // no block-level elements were found, so we create a inline_formatting_context
            let absolutes = block_level_elements.iter().map(|elem| elem.entity()).collect();
            inline_formatting_contexts
                .insert(
                    entity,
                    InlineFormattingContext {
                        elements: inline_level_elements,
                        absolutes,
                        text_align: style.text.text_align,
                        break_points: vec![],
                        lines: vec![],
                        atom_offsets: vec![],
                    },
                )
                .unwrap();
            return;
        }

        // push last anonymous block, should there be one
        if !inline_level_elements.is_empty() {
            let entity = Self::build_anonymous_block(
                &entities,
                &mut inline_formatting_contexts,
                &mut computed_values,
                &inline_level_elements,
                &style,
            );
            block_level_elements.extend_from_slice(&[
                BlockLevelElement::ContainerStart(entity),
                BlockLevelElement::Leaf(entity),
                BlockLevelElement::ContainerEnd(entity),
            ]);
        }

        block_level_elements.insert(0, BlockLevelElement::ContainerStart(entity));
        block_level_elements.push(BlockLevelElement::ContainerEnd(entity));

        block_formatting_contexts
            .insert(
                entity,
                BlockFormattingContext {
                    elements: block_level_elements,
                },
            )
            .unwrap();

        let mut root_node = self.world.write_resource::<RootNode>();
        *root_node = RootNode(Some(entity));
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let mut inline_formatting_context = InlineFormattingContext {
            elements: vec![],
            absolutes: vec![],
            break_points: vec![],
            lines: vec![],
            atom_offsets: vec![],
            text_align: node.style().text.text_align,
        };
        for child in node.children() {
            let child_entity = match child.layout_entity() {
                Some(LayoutEntity(entity)) => entity,
                None => continue,
            };

            let child_style = child.style();
            if child_style.box_.position == PositionSpec::Absolute {
                inline_formatting_context.absolutes.push(child_entity);
                continue;
            }

            if child_style.box_.display == Display::InlineBlock {
                inline_formatting_context.elements.push(child_entity);
                continue;
            }

            // merge child's InlineFormattingContext into the current context
            if let Some(InlineFormattingContext { elements, absolutes, .. }) =
                self.world.write_storage().remove(child_entity)
            {
                inline_formatting_context
                    .elements
                    .extend_from_slice(&elements);
                inline_formatting_context.absolutes.extend(absolutes);
                continue;
            }

            inline_formatting_context.elements.push(child_entity);
        }

        if inline_formatting_context.elements.is_empty() {
            // insert strut
            inline_formatting_context
                .elements
                .push(self.build_text_entity(node.style(), "").0);
        }

        let LayoutEntity(self_entity) = node.layout_entity().unwrap();
        self.world
            .write_storage()
            .insert(self_entity, inline_formatting_context)
            .unwrap();
    }

    // called on bottom-up traversal
    fn build_math_expression<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let LayoutEntity(entity) = node.layout_entity().unwrap();
        let style = node.style();
        let font = self.match_font(&style);
        let font_name = font.postscript_name();

        match node.html_data() {
            ParsedData::Element(element_data) => {
                let elem =
                    mathmlparser::match_math_element((*element_data.name.local).as_bytes())
                        .unwrap_or_default();

                let is_default_variant_mi = elem.is("mi")
                    && !element_data
                        .attributes
                        .borrow()
                        .contains_key(&QualName::new(None, ns!(), local_name!("mathvariant")));

                let mut math_expressions = WriteStorage::<MathExpression>::fetch(&self.world);
                let mut text_runs = WriteStorage::<TextRun>::fetch(&self.world);
                let children = node.children().filter_map(|child| {
                    if let Some(LayoutEntity(child_entity)) = child.layout_entity() {
                        if let Some(text_run) = text_runs.get_mut(child_entity) {
                            use math_render::mathmlparser::StringExtMathml;
                            // By default, an `mi` is only italic if it consists of a single
                            // character, longer identifiers (e.g. `sin`) are upright.
                            let variant = if is_default_variant_mi
                                && text_run.text.chars().count() != 1
                            {
                                MathFamily::Normal
                            } else {
                                style.math.math_variant
                            };
                            let text = text_run
                                .text
                                .adapt_to_family(Some(variant.into()))
                                .replace_anomalous_characters(elem);
                            text_run.text = text;

                            let field = Field::Unicode(text_run.text.clone());
                            return Some(mathmlparser::Child::Field((
                                field,
                                child_entity.id() as _,
                            )));
                        }
                        math_expressions.remove(child_entity).map(
                            |MathExpression { expression, .. }| {
                                mathmlparser::Child::Expression(expression)
                            },
                        )
                    } else {
                        None
                    }
                });

                let (flags, overrides) = style.math.operator_flags_and_overrides();

                let attributes = math_render::mathmlparser::Attributes {
                    schema: Default::default(),
                    token: math_render::mathmlparser::TokenAttributes {
                        operator_attributes: math_render::mathmlparser::OperatorAttributes {
                            form: style.math.form.into(),
                            flags,
                            user_overrides: overrides,
                            ..Default::default()
                        },
                        horizontal_space: if elem.is("mspace") {
                            element_data
                                .attributes
                                .borrow()
                                .get(&QualName::new(None, ns!(), local_name!("width")))
                                .and_then(|attr| parse_math_length(&attr.value, style.font.font_size.0))
                                .map(math_render::Length::em)
                        } else {
                            None
                        },
                        ..Default::default()
                    },
                };

                let expression = mathmlparser::build_element(
                    elem,
                    attributes,
                    children,
                    self.mathml_context.as_mut().unwrap(),
                    entity.id() as u64,
                );

                math_expressions
                    .insert(
                        node.layout_entity().unwrap().0,
                        MathExpression {
                            expression,
                            font_name,
                        },
                    )
                    .unwrap();

                if element_data.name == QualName::new(None, ns!(mathml), local_name!("math")) {
                    self.mathml_context = None;
                }
            }
            _ => {}
        }
    }
}

/// Parses a MathML length (such as the `width` of an `<mspace>`) and returns it in ems.
///
/// Supports the named spaces (`thinmathspace` etc.) and numbers with an `em`, `ex`, `px`, `pt`,
/// `pc`, `in`, `cm`, `mm` or `%` unit. `font_size` is the current font size in pixels.
fn parse_math_length(value: &str, font_size: f32) -> Option<f32> {
    let value = value.trim();
    let named = match value {
        "veryverythinmathspace" => Some(1.0),
        "verythinmathspace" => Some(2.0),
        "thinmathspace" => Some(3.0),
        "mediummathspace" => Some(4.0),
        "thickmathspace" => Some(5.0),
        "verythickmathspace" => Some(6.0),
        "veryverythickmathspace" => Some(7.0),
        "negativeveryverythinmathspace" => Some(-1.0),
        "negativeverythinmathspace" => Some(-2.0),
        "negativethinmathspace" => Some(-3.0),
        "negativemediummathspace" => Some(-4.0),
        "negativethickmathspace" => Some(-5.0),
        "negativeverythickmathspace" => Some(-6.0),
        "negativeveryverythickmathspace" => Some(-7.0),
        _ => None,
    };
    if let Some(eighteenths) = named {
        return Some(eighteenths / 18.0);
    }

    let split = value
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f32 = number.parse().ok()?;
    // CSS pixels per unit
    let px = match unit.trim() {
        "em" => return Some(number),
        "ex" => return Some(number * 0.5),
        "%" => return Some(number / 100.0),
        "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        // unitless numbers are only valid for 0
        "" if number == 0.0 => return Some(0.0),
        _ => return None,
    };
    Some(number * px / font_size)
}

#[cfg(test)]
pub mod test {
    use super::parse_math_length;

    #[test]
    fn test_parse_math_length() {
        assert_eq!(parse_math_length("2em", 16.0), Some(2.0));
        assert_eq!(parse_math_length(" 1.5em ", 16.0), Some(1.5));
        assert_eq!(parse_math_length("16px", 16.0), Some(1.0));
        assert_eq!(parse_math_length("12pt", 16.0), Some(1.0));
        assert_eq!(parse_math_length("-8px", 16.0), Some(-0.5));
        assert_eq!(parse_math_length("0", 16.0), Some(0.0));
        assert_eq!(parse_math_length("thinmathspace", 16.0), Some(3.0 / 18.0));
        assert_eq!(parse_math_length("negativethinmathspace", 16.0), Some(-3.0 / 18.0));
        assert_eq!(parse_math_length("3", 16.0), None);
        assert_eq!(parse_math_length("wide", 16.0), None);
    }

    use super::*;

    use crate::simple_dom::{div, p, span, text, Quantity::Px};

    use crate::style::properties::computed::{ComputedValues, GenericSize};

    #[test]
    fn test_block_flow_creation_from_dom_tree() {
        use BlockLevelElement::*;

        let html = div(vec![div(vec![])]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerEnd(html.entity()),
            ]
        )
    }

    #[test]
    fn test_inline_flow_creation_from_dom_tree() {
        let html = div(vec![text("a"), text("b")]);
        let world = html.construct();

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html.get(&inline_fmt);

        assert_eq!(elements, &[html[0].entity(), html[1].entity()]);
    }

    #[test]
    fn test_nested_inline_flow_creation_from_dom_tree() {
        use BlockLevelElement::*;

        let html = div(vec![div(vec![text("a"), text("b")])]);
        let world = html.construct();

        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);
        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerEnd(html.entity()),
            ]
        );

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html[0].get(&inline_fmt);
        assert_eq!(elements, &[html[0][0].entity(), html[0][1].entity()]);
    }

    #[test]
    fn test_nested_inlines() {
        let html = div(vec![
            span(vec![span(vec![text("Abc")]), span(vec![text("efg")])]),
            span(vec![text("hij")]),
        ]);
        let world = html.construct();

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html.get(&inline_fmt);
        assert_eq!(
            elements,
            &[
                html[0][0][0].entity(),
                html[0][1][0].entity(),
                html[1][0].entity()
            ]
        )
    }

    #[test]
    fn test_text_run_creation_from_text_node() {
        let html = text("Hello World!");
        let world = html.construct();
        let text_run = world.read_storage();
        let components::TextRun { text, .. } = html.get(&text_run);
        assert_eq!(text, "Hello World!");
    }

    #[test]
    fn test_anonymous_block_box_for_text_node_creation() {
        use BlockLevelElement::*;

        // <div>
        //   <div></div>
        //   Hello World
        // </div>
        let html = div(vec![div(vec![]), text("Hello World!")]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                // these are the three anonymous node
                ContainerStart(elements[4].entity()),
                Leaf(elements[5].entity()),
                ContainerEnd(elements[6].entity()),
                // end of anonymous nodes
                ContainerEnd(html.entity()),
            ]
        );

        if let Leaf(entity) = elements[5] {
            let inline_fmt = world.read_storage();
            let InlineFormattingContext { elements, .. } = inline_fmt.get(entity).unwrap();
            assert_eq!(elements, &[html[1].entity()]);
        } else {
            panic!()
        }
    }

    #[test]
    fn test_nested_block_level_elements_creation() {
        use BlockLevelElement::*;

        // <div>
        //   <p>Something</p>
        //   <div>
        //      <p>something nested</p>
        //   </div>
        //   <p>something else</p>
        // </div>
        let html = div(vec![
            p(vec![text("Something")]),
            div(vec![p(vec![text("something nested")])]),
            p(vec![text("something else")]),
        ]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                ContainerStart(html[0].entity()),
                Leaf(html[0].entity()),
                ContainerEnd(html[0].entity()),
                ContainerStart(html[1].entity()),
                ContainerStart(html[1]["p"].entity()),
                Leaf(html[1]["p"].entity()),
                ContainerEnd(html[1]["p"].entity()),
                ContainerEnd(html[1].entity()),
                ContainerStart(html[2].entity()),
                Leaf(html[2].entity()),
                ContainerEnd(html[2].entity()),
                ContainerEnd(html.entity()),
            ]
        );

        let inline_fmt = world.read_storage();
        let InlineFormattingContext { elements, .. } = html[1]["p"].get(&inline_fmt);

        assert_eq!(elements, &[html[1]["p"][0].entity()])
    }

    #[test]
    fn test_if_styled_node_has_style_component() {
        let html = div(vec![]).margin_top(Px(10.0));
        let world = html.construct();
        let styles = world.read_storage();
        let style: &ComputedValues = html.get(&styles);

        assert_eq!(style.position.margin_top, GenericSize::from(Px(10.0)))
    }
}
