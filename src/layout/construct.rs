use super::entity::{LayoutEntity, Position, UnscannedText};
use super::inline::InlineFormattingContext;
use super::normal_formatting::{BlockFormattingContext, BlockLevelElement};
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::Styles;
use crate::style::properties::computed::{ComputedValues, Display, StyleBuilder};
use crate::traverse::Traversal;
use crate::tree::{DomRef, NodeData, TreeLinked};

use html5ever::{local_name, namespace_url, ns, Attribute, QualName};
use specs::World;

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
pub struct FlowConstructor {
    styles: Styles,
    entity_constructor: entity_constructor::EntityConstructor,
}

impl FlowConstructor {
    pub fn new(world: World) -> Self {
        FlowConstructor {
            styles: Default::default(),
            entity_constructor: entity_constructor::EntityConstructor::new(world),
        }
    }

    pub fn layout_context(self) -> super::entity::LayoutContext {
        self.entity_constructor.world
    }

    fn read_stylesheet(&mut self, node: &DomRef) {
        if let ParsedData::Element(ElementData {
            name, attributes, ..
        }) = &node.html_data
        {
            if name.ns == ns!(html) && name.local == local_name!("style") {
                for child in node.children() {
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
                .and_then(|parent| parent.style());
            let mut style_builder = StyleBuilder::new(parent_style.unwrap_or_default());
            for matched_rule in self
                .styles
                .rules_iter()
                .filter(|rule| rule.matches(element_ref))
            {
                let properties = &matched_rule.properties;
                for declaration in properties.declarations() {
                    style_builder.add_declaration(declaration.clone());
                }
            }
            element_ref.as_node().set_style(style_builder.build());
        } else {
            let parent_style = node.parent().and_then(|parent| parent.style());
            let style = StyleBuilder::new(parent_style.unwrap_or_default()).build();
            node.set_style(style);
        }

        // we insert the entities in tree order
        self.entity_constructor.build_entity(node)
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        self.entity_constructor.build_formatting_context(&node)
    }
}

pub mod entity_constructor {
    use super::{
        BlockFormattingContext, BlockLevelElement, ComputedValues, Display, DomRef,
        InlineFormattingContext, LayoutEntity, NodeData, ParsedData, Position, TreeLinked,
        UnscannedText,
    };
    use specs::{World, WorldExt};

    use std::fmt;

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

    pub struct EntityConstructor {
        pub world: World,
    }

    impl fmt::Debug for EntityConstructor {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.debug_struct("EntityConstructor").finish()
        }
    }

    impl EntityConstructor {
        pub fn new(mut world: World) -> Self {
            world.register::<ComputedValues>();
            world.register::<InlineFormattingContext>();
            world.register::<BlockFormattingContext>();
            world.register::<Position>();
            world.register::<UnscannedText>();
            EntityConstructor { world }
        }

        // Called on top-down traversal. Takes a node and its style to create a corresponding
        // `LayoutEntity`, and assigns the entity to the node.
        pub fn build_entity<'a>(&mut self, mut node: impl AbstractNode<'a>) {
            let style = node.style();

            if style.box_.display == Display::None {
                return;
            }

            let entity = match node.html_data() {
                ParsedData::Element(_) => self.build_block_entity(style),
                ParsedData::Document(_) => self.build_root_entity(),
                ParsedData::Text(string) => self.build_text_entity(style, &string.borrow()),
                _ => self.build_block_entity(style),
            };

            node.assign_layout_entity(entity);
        }

        // called on top-down traversal
        fn build_root_entity(&mut self) -> LayoutEntity {
            self.build_block_entity(Default::default())
        }

        // called on top-down traversal
        fn build_block_entity(&mut self, style: ComputedValues) -> LayoutEntity {
            use specs::Builder;

            let entity_builer = self.world.create_entity().with(style);
            entity_builer.with(Position::default()).build().into()
        }

        /// builds an anonymous block box for text nodes
        fn build_text_entity(&mut self, style: ComputedValues, text: &str) -> LayoutEntity {
            let LayoutEntity(entity) = self.build_block_entity(style);
            self.world
                .write_storage()
                .insert(
                    entity,
                    UnscannedText {
                        characters: text.to_owned(),
                    },
                )
                .unwrap();
            LayoutEntity(entity)
        }

        // Called on bottom-up traversal. Traverses the node's children and decides (based on their
        // computed styles) whether to build a block formatting context or an inline formatting
        // context.
        pub fn build_formatting_context<'a, Node: AbstractNode<'a>>(&mut self, node: &'a Node) {
            let is_display_inline = |node: Node| match node.style().box_.display {
                Display::Inline => true,
                _ => false,
            };

            if node.children().all(is_display_inline) {
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
                    if let Some(BlockFormattingContext { elements }) =
                        self.world.write_storage().remove(child_entity)
                    {
                        block_formatting_context
                            .elements
                            .extend_from_slice(&elements);
                    } else {
                        block_formatting_context
                            .elements
                            .push(BlockLevelElement::Leaf(child_entity));
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
            let mut inline_formatting_context = InlineFormattingContext { elements: vec![] };
            for child in node.children() {
                if let Some(LayoutEntity(child_entity)) = child.layout_entity() {
                    if let Some(InlineFormattingContext { elements }) =
                        self.world.write_storage().remove(child_entity)
                    {
                        inline_formatting_context
                            .elements
                            .extend_from_slice(&elements);
                    } else {
                        inline_formatting_context.elements.push(child_entity);
                    }
                }
            }

            if !inline_formatting_context.elements.is_empty() {
                if let Some(LayoutEntity(self_entity)) = node.layout_entity() {
                    self.world
                        .write_storage()
                        .insert(self_entity, inline_formatting_context)
                        .unwrap();
                }
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
        fn test_unscanned_text_creation_from_text_node() {
            let html = text("Hello World!");
            let world = html.construct();
            let unscanned_texts = world.read_storage();
            let UnscannedText { characters } = html.get(&unscanned_texts);
            assert_eq!(characters, "Hello World!");
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
            let InlineFormattingContext { elements } = html[1]["p"].get(&inline_fmt);

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
}
