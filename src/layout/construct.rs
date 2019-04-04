use super::block::BlockContainer;
use super::block_formatting_context::BlockFormattingContext;
use super::entity::{
    CanCollapse, ContainingBlock, Decoration, LayoutEntity, Position,
    StackingContextRelativePosition, Text, UnscannedText,
};
use super::inline::InlineFormattingContext;
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::Styles;
use crate::style::properties::computed::{ComputedValues, Display, StyleBuilder};
use crate::traverse::Traversal;
use crate::tree::{DomRef, TreeLinked};

use html5ever::{local_name, namespace_url, ns, Attribute, QualName};
use specs::{Builder, World};

use std::fmt;

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug, Default)]
pub struct FlowConstructor {
    styles: Styles,
    entity_constructor: EntityConstructor,
}

impl FlowConstructor {
    pub fn new() -> Self {
        FlowConstructor::default()
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
            node.set_style(Default::default());
        }

        // we insert the entities in tree order
        self.entity_constructor.build_entity(node)
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        self.entity_constructor.build_formatting_context(node)
    }
}

struct EntityConstructor {
    world: World,
}

impl fmt::Debug for EntityConstructor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("EntityConstructor").finish()
    }
}

impl Default for EntityConstructor {
    fn default() -> Self {
        let mut world = World::new();
        world.register::<ContainingBlock>();
        world.register::<Position>();
        world.register::<ComputedValues>();
        world.register::<BlockContainer>();
        world.register::<InlineFormattingContext>();
        world.register::<Text>();
        world.register::<UnscannedText>();
        world.register::<StackingContextRelativePosition>();
        world.register::<BlockFormattingContext>();
        world.register::<CanCollapse>();
        world.register::<Decoration>();
        EntityConstructor { world }
    }
}

impl EntityConstructor {
    // called on top-down traversal
    fn build_entity(&mut self, node: DomRef) {
        let style = node.style().expect("No style was assigned");

        if style.box_.display == Display::None {
            return;
        }

        let parent_entity = node.parent().and_then(|x| x.layout_entity.get());
        let entity = match &node.html_data {
            ParsedData::Element(_) => self.build_block_entity(style, parent_entity),
            ParsedData::Document(_) => self.build_root_entity(),
            ParsedData::Text(string) => {
                self.build_text_entity(style, parent_entity, &string.borrow())
            }
            _ => self.build_block_entity(style, parent_entity),
        };

        node.layout_entity.set(Some(entity));
    }

    // called on top-down traversal
    fn build_root_entity(&mut self) -> LayoutEntity {
        self.build_block_entity(Default::default(), None)
    }

    // called on top-down traversal
    fn build_block_entity(
        &mut self,
        style: ComputedValues,
        parent_entity: Option<LayoutEntity>,
    ) -> LayoutEntity {
        let entity_builer = self.world.create_entity().with(style);

        let entity_builer = if let Some(parent_entity) = parent_entity {
            entity_builer.with(ContainingBlock {
                entity: parent_entity,
            })
        } else {
            entity_builer
        };

        entity_builer.with(Position::default()).build().into()
    }

    fn build_text_entity(
        &mut self,
        style: ComputedValues,
        parent_entity: Option<LayoutEntity>,
        text: &str,
    ) -> LayoutEntity {
        let LayoutEntity(entity) = self.build_block_entity(style, parent_entity);
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

    // called on bottom-up traversal
    fn build_formatting_context(&mut self, node: DomRef) {
        if node.children().all(|child| {
            match child.style().expect("No style was assigned").box_.display {
                Display::Inline => true,
                _ => false,
            }
        }) {
            self.build_inline_formatting_context(node)
        } else {
            self.build_block_formatting_context(node)
        }
    }

    // called on bottom-up traversal
    fn build_block_formatting_context(&mut self, node: DomRef) {
        let mut block_formatting_context = BlockContainer { elements: vec![] };
        for child in node.children() {
            if let Some(child_entity) = child.layout_entity.get() {
                block_formatting_context.elements.push(child_entity.0);
            } else {
                continue;
            }
        }

        if block_formatting_context.elements.len() > 0 {
            if let Some(LayoutEntity(self_entity)) = node.layout_entity.get() {
                self.world
                    .write_storage()
                    .insert(self_entity, block_formatting_context)
                    .unwrap();
            }
        }
    }

    // called on bottom-up traversal
    fn build_inline_formatting_context(&mut self, node: DomRef) {
        let mut inline_formatting_context = InlineFormattingContext { elements: vec![] };
        for child in node.children() {
            if let Some(LayoutEntity(child_entity)) = child.layout_entity.get() {
                inline_formatting_context.elements.push(child_entity.id());
            }
        }

        if inline_formatting_context.elements.len() > 0 {
            if let Some(LayoutEntity(self_entity)) = node.layout_entity.get() {
                self.world
                    .write_storage()
                    .insert(self_entity, inline_formatting_context)
                    .unwrap();
            }
        }
    }
}
