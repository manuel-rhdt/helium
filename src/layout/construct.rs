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

mod entity_constructor {
    use super::{
        BlockFormattingContext, BlockLevelElement, ComputedValues, Display, DomRef,
        InlineFormattingContext, LayoutEntity, NodeData, ParsedData, Position, TreeLinked,
        UnscannedText,
    };
    use specs::World;

    use std::fmt;

    pub(super) trait AssignLayoutEntity {
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

    pub(super) trait Styled {
        fn style(&self) -> ComputedValues;
    }

    impl Styled for DomRef {
        fn style(&self) -> ComputedValues {
            NodeData::style(&self).expect("No style was assigned to DOM node.")
        }
    }

    pub(super) trait HtmlNode: Sized {
        fn html_data(&self) -> &ParsedData;
    }

    impl HtmlNode for DomRef {
        fn html_data(&self) -> &ParsedData {
            &self.html_data
        }
    }

    pub(super) trait IterChildren<'a>: Sized {
        type ChildrenIter: 'a + Iterator<Item = Self>;
        fn children(&'a self) -> Self::ChildrenIter;
    }

    impl<'a> IterChildren<'a> for DomRef {
        type ChildrenIter = crate::tree::SiblingIter<DomRef>;
        fn children(&'a self) -> Self::ChildrenIter {
            TreeLinked::children(&self)
        }
    }

    pub(super) trait AbstractNode<'a>:
        AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a>
    {
    }

    impl<'a, T: 'a + AssignLayoutEntity + Styled + HtmlNode + IterChildren<'a>> AbstractNode<'a> for T {}

    pub(super) struct EntityConstructor {
        pub(super) world: World,
    }

    impl fmt::Debug for EntityConstructor {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.debug_struct("EntityConstructor").finish()
        }
    }

    impl EntityConstructor {
        pub(super) fn new(mut world: World) -> Self {
            world.register::<ComputedValues>();
            world.register::<InlineFormattingContext>();
            world.register::<BlockFormattingContext>();
            world.register::<Position>();
            world.register::<UnscannedText>();
            EntityConstructor { world }
        }

        // Called on top-down traversal. Takes a node and its style to create a corresponding
        // `LayoutEntity`, and assigns the entity to the node.
        pub(super) fn build_entity<'a>(&mut self, mut node: impl AbstractNode<'a>) {
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
        pub(super) fn build_formatting_context<'a, Node: AbstractNode<'a>>(
            &mut self,
            node: &'a Node,
        ) {
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
    mod test {
        use super::*;
        use crate::parser::ElementData;

        use html5ever::QualName;
        use specs::{Component, Entity, ReadStorage};

        use std::cell::Cell;
        use std::ops::Index;

        #[derive(Clone)]
        struct Node {
            name: &'static str,
            entity: Cell<Option<LayoutEntity>>,
            html_data: ParsedData,
            style: ComputedValues,
            contents: Vec<Node>,
        }

        impl Node {
            fn construct(&self) -> World {
                let world = World::new();
                let mut constructor = EntityConstructor::new(world);
                construct_recursive(self, &mut constructor);
                constructor.world
            }

            fn get_name(&self, name: &str) -> Option<&Node> {
                if self.name == name {
                    Some(self)
                } else {
                    for child in &self.contents {
                        if let Some(node) = child.get_name(name) {
                            return Some(node);
                        }
                    }
                    None
                }
            }

            fn entity(&self) -> Entity {
                self.entity.get().unwrap().0
            }

            // Retrieve the component corresponding to this node from the given storage.
            fn get<'a, T: Component>(&self, storage: &'a ReadStorage<'a, T>) -> &'a T {
                storage
                    .get(self.entity())
                    .expect("Node does not have the needed component")
            }
        }

        impl<'a> Index<&'a str> for Node {
            type Output = Node;

            /// Performs a depth-first search for a node with name `name` starting with `self` and
            /// descending through the children.
            /// 
            /// # Panics
            /// 
            /// Panics if no node with the given name can be found.
            fn index(&self, name: &'a str) -> &Node {
                self.get_name(name).expect("No node with name found")
            }
        }

        impl<'a> AssignLayoutEntity for &'a Node {
            fn assign_layout_entity(&mut self, entity: LayoutEntity) {
                self.entity.set(Some(entity));
            }

            fn layout_entity(&self) -> Option<LayoutEntity> {
                self.entity.get()
            }
        }


        impl<'a> Styled for &'a Node {
            fn style(&self) -> ComputedValues {
                self.style.clone()
            }
        }

        impl<'a> HtmlNode for &'a Node {
            fn html_data(&self) -> &ParsedData {
                &self.html_data
            }
        }

        impl<'a> IterChildren<'a> for &'a Node {
            type ChildrenIter = std::slice::Iter<'a, Node>;
            fn children(&'a self) -> Self::ChildrenIter {
                self.contents.iter()
            }
        }


        fn block(name: &'static str, contents: Vec<Node>) -> Node {
            use crate::style::properties::computed::Box;
            use std::sync::Arc;
            Node {
                name,
                entity: Cell::new(None),
                html_data: ParsedData::Element(ElementData {
                    name: QualName::new(None, ns!(html), name.into()),
                    attributes: Default::default(),
                    template_contents: None,
                }),
                style: ComputedValues {
                    box_: Arc::new(Box {
                        display: Display::Block,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                contents,
            }
        }

        fn inline(name: &'static str, contents: Vec<Node>) -> Node {
            Node {
                name,
                entity: Cell::new(None),
                html_data: ParsedData::Element(ElementData {
                    name: QualName::new(None, ns!(html), name.into()),
                    attributes: Default::default(),
                    template_contents: None,
                }),
                style: Default::default(),
                contents,
            }
        }

        fn text(name: &'static str, text: &'static str) -> Node {
            Node {
                name,
                entity: Cell::new(None),
                html_data: ParsedData::Text(text.to_string().into()),
                style: Default::default(),
                contents: vec![],
            }
        }

        fn construct_recursive(node: &Node, constructor: &mut EntityConstructor) {
            constructor.build_entity(node);
            for child in node.children() {
                construct_recursive(child, constructor);
            }
            constructor.build_formatting_context(&node);
        }

        #[test]
        fn test_block_flow_creation_from_dom_tree() {
            use BlockLevelElement::*;

            let html = block("a", vec![block("b", vec![])]);
            let world = html.construct();
            let block_fmt = world.read_storage();
            let BlockFormattingContext { elements } = html["a"].get(&block_fmt);

            assert_eq!(
                elements,
                &vec![
                    ContainerStart(html["a"].entity()),
                    Leaf(html["b"].entity()),
                    ContainerEnd(html["a"].entity()),
                ]
            )
        }

        #[test]
        fn test_unscanned_text_creation_from_text_node() {
            let html = text("node1", "Hello World!");
            let world = html.construct();
            let unscanned_texts = world.read_storage();
            let UnscannedText { characters } = html["node1"].get(&unscanned_texts);
            assert_eq!(characters, "Hello World!");
        }

        #[test]
        fn test_anonymous_block_box_for_text_node_creation() {
            use BlockLevelElement::*;

            let html = block(
                "div",
                vec![block("div2", vec![]), text("text", "Hello World!")],
            );
            let world = html.construct();
            let block_fmt = world.read_storage();
            let BlockFormattingContext { elements } = html["div"].get(&block_fmt);

            assert_eq!(
                elements,
                &vec![
                    ContainerStart(html["div"].entity()),
                    Leaf(html["div2"].entity()),
                    Leaf(html["text"].entity()),
                    ContainerEnd(html["div"].entity()),
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
            let html = block(
                "div",
                vec![
                    text("text1", "Something"),
                    block("div2", vec![block("p", vec![text("text2", "something nested")])]),
                    text("text3", "something else"),
                ],
            );
            let world = html.construct();
            let block_fmt = world.read_storage();
            let BlockFormattingContext { elements } = html["div"].get(&block_fmt);

            assert_eq!(
                elements,
                &vec![
                    ContainerStart(html["div"].entity()),
                    Leaf(html["text1"].entity()),
                    ContainerStart(html["div2"].entity()),
                    Leaf(html["p"].entity()),
                    ContainerEnd(html["div2"].entity()),
                    Leaf(html["text3"].entity()),
                    ContainerEnd(html["div"].entity()),
                ]
            );

            let inline_fmt = world.read_storage();
            let InlineFormattingContext { elements } = html["p"].get(&inline_fmt);

            assert_eq!(
                elements,
                &vec![html["text2"].entity()]
            )
        }
    }
}
