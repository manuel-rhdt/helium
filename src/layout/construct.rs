use super::components::{self, BlockLevelElement, LayoutEntity, MathExpression, Position};
use super::inline::InlineFormattingContext;
use super::normal_formatting::BlockFormattingContext;
use super::{text::Font, FontMap};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::Styles;
use crate::style::properties::computed::{ComputedValues, Display, LineHeight, StyleBuilder};
use crate::traverse::Traversal;
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{local_name, namespace_url, ns, Attribute, QualName};
use math_render::{mathmlparser, Field};
use specs::{Builder, World, WorldExt};

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
            if name.ns == ns!(html) && name.local == local_name!("style") {
                for child in TreeLinked::children(node) {
                    if let ParsedData::Text(css) = &child.html_data {
                        self.styles.parse_css(&css.borrow())
                    }
                }
            }
            if name.ns == ns!(html) && name.local == local_name!("link") {
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

        // we insert the entities in tree order
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
            ParsedData::Text(string) => self.build_text_entity(style, &string.borrow()),
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

        if is_display_inline(node) || node.children().all(|n| is_display_inline(&n)) {
            self.build_inline_formatting_context(node)
        } else {
            self.build_block_formatting_context(node)
        }
    }

    // called on bottom-up traversal
    fn build_block_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let mut block_formatting_context = BlockFormattingContext { elements: vec![] };

        if let Some(LayoutEntity(self_entity)) = node.layout_entity() {
            block_formatting_context
                .elements
                .push(BlockLevelElement::ContainerStart(self_entity))
        }

        for child in node.children() {
            if let Some(LayoutEntity(child_entity)) = child.layout_entity() {
                let child_bfc = self.world.write_storage().remove(child_entity);
                if let Some(BlockFormattingContext { elements }) = child_bfc {
                    block_formatting_context
                        .elements
                        .extend_from_slice(&elements);
                } else {
                    // insert anonymous block level box
                    // see https://www.w3.org/TR/CSS22/visuren.html#anonymous-block-level
                    block_formatting_context.elements.extend_from_slice(&[
                        BlockLevelElement::ContainerStart(child_entity),
                        BlockLevelElement::Leaf(child_entity),
                        BlockLevelElement::ContainerEnd(child_entity),
                    ]);
                }
            }
        }

        if let Some(LayoutEntity(self_entity)) = node.layout_entity() {
            block_formatting_context
                .elements
                .push(BlockLevelElement::ContainerEnd(self_entity))
        }

        if !block_formatting_context.elements.is_empty() {
            if let Some(LayoutEntity(self_entity)) = node.layout_entity() {
                self.world
                    .write_storage()
                    .insert(self_entity, block_formatting_context)
                    .unwrap();
            }
        }
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context<'a>(&mut self, node: &'a impl AbstractNode<'a>) {
        let mut new_contexts = vec![];

        let mut inline_formatting_context = InlineFormattingContext {
            elements: vec![],
            break_points: vec![],
            lines: vec![],
            text_align: node.style().text.text_align,
        };
        for child in node.children() {
            let child_entity = child.layout_entity().unwrap().0;
            if let Some(InlineFormattingContext { elements, .. }) =
                self.world.write_storage().remove(child_entity)
            {
                inline_formatting_context
                    .elements
                    .extend_from_slice(&elements);
                continue;
            }
            if child.style().box_.display == Display::Block
                || self
                    .world
                    .read_storage::<BlockFormattingContext>()
                    .contains(child_entity)
            {
                if inline_formatting_context.elements.is_empty() {
                    // insert strut
                    inline_formatting_context
                        .elements
                        .push(self.build_text_entity(node.style(), "").0);
                }
                let new_ifc = self
                    .world
                    .create_entity()
                    .with(std::mem::replace(
                        &mut inline_formatting_context,
                        InlineFormattingContext {
                            elements: vec![],
                            break_points: vec![],
                            lines: vec![],
                            text_align: node.style().text.text_align,
                        },
                    ))
                    .with(node.style())
                    .build();
                new_contexts.push(new_ifc);
                new_contexts.push(child_entity);
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
        if new_contexts.is_empty() {
            self.world
                .write_storage()
                .insert(self_entity, inline_formatting_context)
                .unwrap();
        } else {
            let last_ifc = self
                .world
                .create_entity()
                .with(inline_formatting_context)
                .with(node.style())
                .build();
            new_contexts.push(last_ifc);

            let mut block_formatting_context = BlockFormattingContext { elements: vec![] };

            block_formatting_context
                .elements
                .push(BlockLevelElement::ContainerStart(self_entity));

            for child_entity in new_contexts {
                let child_bfc = self.world.write_storage().remove(child_entity);
                if let Some(BlockFormattingContext { elements }) = child_bfc {
                    block_formatting_context
                        .elements
                        .extend_from_slice(&elements);
                } else {
                    // insert anonymous block level box
                    // see https://www.w3.org/TR/CSS22/visuren.html#anonymous-block-level
                    block_formatting_context.elements.extend_from_slice(&[
                        BlockLevelElement::ContainerStart(child_entity),
                        BlockLevelElement::Leaf(child_entity),
                        BlockLevelElement::ContainerEnd(child_entity),
                    ]);
                }
            }

            block_formatting_context
                .elements
                .push(BlockLevelElement::ContainerEnd(self_entity));

            self.world
                .write_storage()
                .insert(self_entity, block_formatting_context)
                .unwrap();
        }
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

                let world = &mut self.world;
                let children = node.children().filter_map(|child| {
                    if let ParsedData::Text(string) = child.html_data() {
                        let field = Field::Unicode(string.borrow().clone());
                        return Some(mathmlparser::Child::Field(field));
                    }
                    let LayoutEntity(entity) = child.layout_entity().unwrap();
                    if let Some(MathExpression { expression, .. }) =
                        world.write_storage::<MathExpression>().remove(entity)
                    {
                        Some(mathmlparser::Child::Expression(expression))
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

                world
                    .write_storage()
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
                Leaf(html[0].entity()),
                ContainerEnd(html.entity()),
            ]
        )
    }

    #[test]
    fn test_text_run_creation_from_text_node() {
        let html = text("Hello World!");
        let world = html.construct();
        let unscanned_texts = world.read_storage();
        let components::TextRun { text, .. } = html.get(&unscanned_texts);
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
                Leaf(html[0].entity()),
                Leaf(html[1].entity()),
                ContainerEnd(html.entity()),
            ]
        )
    }

    #[test]
    fn test_nested_block_level_elements_creation() {
        use BlockLevelElement::*;

        // <div>
        //   Something
        //   <div>
        //      <p>something nested</p>
        //   </div>
        //   something else
        // </div>
        let html = div(vec![
            text("Something"),
            div(vec![p(vec![text("something nested")])]),
            text("something else"),
        ]);
        let world = html.construct();
        let block_fmt = world.read_storage();
        let BlockFormattingContext { elements } = html.get(&block_fmt);

        assert_eq!(
            elements,
            &vec![
                ContainerStart(html.entity()),
                Leaf(html[0].entity()),
                ContainerStart(html[1].entity()),
                Leaf(html[1]["p"].entity()),
                ContainerEnd(html[1].entity()),
                Leaf(html[2].entity()),
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
