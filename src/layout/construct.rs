use super::components::{self, BlockLevelElement, LayoutEntity, MathExpression, Position};
use super::inline::InlineFormattingContext;
use super::normal_formatting::BlockFormattingContext;
use super::{text::Font, FontMap};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::Styles;
use crate::style::properties::computed::{ComputedValues, Display, LineHeight, StyleBuilder, PositionSpec};
use crate::traverse::Traversal;
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{local_name, namespace_url, ns, Attribute, QualName};
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
                        log::error!("<style> element did not contain any style information.")
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
        if let Some(element_ref) = &node.clone().into_element_ref() {
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
            element_ref.as_node().set_style(style_builder.build());
        } else {
            let parent_style = node.parent().and_then(|parent| NodeData::style(&parent));
            let style = StyleBuilder::new(parent_style.unwrap_or_default()).build();
            node.set_style(style);
        }

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
}

impl<'a> IterChildren<'a> for DomRef {
    type ChildrenIter = crate::tree::SiblingIter<DomRef>;
    fn children(&'a self) -> Self::ChildrenIter {
        TreeLinked::children(&self)
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
        world.register::<components::Decoration>();
        world.register::<MathExpression>();
        world.insert(FontMap::new());
        EntityConstructor {
            world,
            font_match_cache: Default::default(),
            mathml_context: None,
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

                self.build_default_entity(style)
            }
            ParsedData::Document(_) => self.build_root_entity(),
            ParsedData::Text(string) => {
                let text = &string.borrow() as &str;
                if text.trim().is_empty() {
                    return;
                } else {
                    self.build_text_entity(style, text)
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
                    break_points: vec![],
                    lines: vec![],
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
             block_formatting_contexts: &mut WriteStorage<BlockFormattingContext>| {
                if let Some(child_bfc) = block_formatting_contexts.remove(entity) {
                    ble.extend_from_slice(&child_bfc.elements);
                } else {
                    ble.extend_from_slice(&[
                        BlockLevelElement::ContainerStart(entity),
                        BlockLevelElement::Leaf(entity),
                        BlockLevelElement::ContainerEnd(entity),
                    ])
                }
            };

        for elem in elem_iter {
            let child_style = computed_values.get(elem);
            let display = child_style.map(|s| s.box_.display).unwrap_or(Display::Inline);
            let position = child_style.map(|s| s.box_.position).unwrap_or_default();

            if position == PositionSpec::Absolute {
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
                    );
                    inline_level_elements.clear();
                }
                push_block(block_level_elements, elem, block_formatting_contexts);
            } else {
                if let Some(InlineFormattingContext { elements, .. }) =
                    inline_formatting_contexts.remove(elem)
                {
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

        if block_level_elements.is_empty() {
            // no block-level elements were found, so we create a inline_formatting_context
            inline_formatting_contexts
                .insert(
                    entity,
                    InlineFormattingContext {
                        elements: inline_level_elements,
                        text_align: style.text.text_align,
                        break_points: vec![],
                        lines: vec![],
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
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let mut inline_formatting_context = InlineFormattingContext {
            elements: vec![],
            break_points: vec![],
            lines: vec![],
            text_align: node.style().text.text_align,
        };
        for child in node.children() {
            let child_entity = match child.layout_entity() {
                Some(LayoutEntity(entity)) => entity,
                None => continue,
            };

            let child_style = child.style();
            if child_style.box_.position == PositionSpec::Absolute {
                // TODO: handle absoulte elements
                continue;
            }

            if let Some(InlineFormattingContext { elements, .. }) =
                self.world.write_storage().remove(child_entity)
            {
                inline_formatting_context
                    .elements
                    .extend_from_slice(&elements);
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
                    mathmlparser::match_math_element(element_data.name.local.as_ref().as_bytes())
                        .unwrap_or_default();

                let attributes = element_data.attributes.borrow();
                let attr_iter = attributes.values().map(|attr| {
                    let name = attr.name.local.as_ref();
                    let value = attr.value.as_ref();
                    (name, value)
                });

                let mut math_expressions = WriteStorage::<MathExpression>::fetch(&self.world);
                let mut text_runs = WriteStorage::<TextRun>::fetch(&self.world);
                let children = node.children().filter_map(|child| {
                    if let Some(LayoutEntity(entity)) = child.layout_entity() {
                        if let Some(text_run) = text_runs.get_mut(entity) {
                            use math_render::mathmlparser::StringExtMathml;
                            let text = text_run
                                .text
                                .adapt_to_family(Some(style.math.math_variant.into()))
                                .replace_anomalous_characters(elem);
                            text_run.text = text;

                            let field = Field::Unicode(text_run.text.clone());
                            return Some(mathmlparser::Child::Field((field, entity.id() as _)));
                        }
                        math_expressions.remove(entity).map(
                            |MathExpression { expression, .. }| {
                                mathmlparser::Child::Expression(expression)
                            },
                        )
                    } else {
                        None
                    }
                });

                let expression = mathmlparser::build_element(
                    elem,
                    attr_iter,
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

#[cfg(test)]
pub mod test {
    use super::*;

    use crate::simple_dom::{div, p, text, Quantity::Px};

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
                // these are the anonymous node
                elements[4],
                elements[5],
                elements[6],
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
