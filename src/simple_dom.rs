//! A simple implementation of a dom tree that is mostly useful for tests right now.

use crate::layout::construct::entity_constructor::{
    AssignLayoutEntity, EntityConstructor, HtmlNode, IterChildren, Styled,
};
use crate::layout::entity::LayoutEntity;
use crate::parser::{ElementData, ParsedData};
use crate::style::properties::computed::{ComputedValues, Display, PixelLength, CssFloat, GenericSize, Percentage};

use html5ever::QualName;
use specs::{Component, Entity, ReadStorage, World};

use std::cell::Cell;
use std::ops::Index;
use std::sync::Arc;

/// A node of a simple dom tree.
///
/// A `Node` owns all of its children and additionally carries
/// - an associated style
/// - associated html_data
/// - the layout entity created from the node on construction.
#[derive(Clone)]
pub struct Node {
    name: &'static str,
    entity: Cell<Option<LayoutEntity>>,
    html_data: ParsedData,
    style: ComputedValues,
    contents: Vec<Node>,
}

impl Node {
    /// Construct a `World` using this node as a root node.
    ///
    /// This creates a world and populates it with entities for every node descending from this
    /// node (including this node itself). The layout step consists of various functions (called
    /// "systems" in the context of an ECS) that get mutable references to the data stored in the
    /// world and manipulate it somehow.
    pub fn construct(&self) -> World {
        let world = World::new();
        let mut constructor = EntityConstructor::new(world);
        construct_recursive(self, &mut constructor);
        constructor.world
    }

    fn get_child_with_name(&self, name: &str) -> Option<&Node> {
        for child in &self.contents {
            if child.name == name {
                return Some(child);
            }
        }
        None
    }

    /// Get the `Entity` that corresponds to this node.
    ///
    /// # Panics
    ///
    /// Node that this function can only be called, after `construct` was called. Else this function
    /// will **`panic!`**.
    pub fn entity(&self) -> Entity {
        self.entity.get().unwrap().0
    }

    // Retrieve the component corresponding to this node from the given storage.
    pub fn get<'a, T: Component>(&self, storage: &'a ReadStorage<'a, T>) -> &'a T {
        storage
            .get(self.entity())
            .expect("Node does not have the needed component")
    }

    pub fn margin_top(mut self, margin_top: Quantity) -> Self {
        Arc::make_mut(&mut self.style.position).margin_top = margin_top.into();
        self
    }

    pub fn margin_bottom(mut self, margin_bottom: Quantity) -> Self {
        Arc::make_mut(&mut self.style.position).margin_bottom = margin_bottom.into();
        self
    }

    pub fn margin_left(mut self, margin_left: Quantity) -> Self {
        Arc::make_mut(&mut self.style.position).margin_left = margin_left.into();
        self
    }

    pub fn margin_right(mut self, margin_right: Quantity) -> Self {
        Arc::make_mut(&mut self.style.position).margin_right = margin_right.into();
        self
    }

    pub fn height(mut self, height: Quantity) -> Self {
        Arc::make_mut(&mut self.style.position).height = height.into();
        self
    }
}

impl<'a> Index<&'a str> for Node {
    type Output = Node;

    /// Returns the first child whose tag name matches `name`.
    ///
    /// # Panics
    ///
    /// Panics if no node with the given name can be found among the children.
    fn index(&self, name: &'a str) -> &Node {
        self.get_child_with_name(name)
            .unwrap_or_else(|| panic!("Index Error: No child with name = {:?} found", name))
    }
}

impl Index<usize> for Node {
    type Output = Node;

    /// Returns the n-th child of `self`.
    ///
    /// # Panics
    ///
    /// Panics if `self` has less than `n - 1` children.
    fn index(&self, idx: usize) -> &Node {
        &self.contents[idx]
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

/// Create a block-level html element.
///
/// This function returns a Node with default element style except it being `display: block;`.
pub fn block(name: &'static str, contents: Vec<Node>) -> Node {
    use crate::style::properties::computed::Box;
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

pub fn html(contents: Vec<Node>) -> Node {
    block("html", contents)
}

pub fn div(contents: Vec<Node>) -> Node {
    block("div", contents)
}

pub fn p(contents: Vec<Node>) -> Node {
    block("p", contents)
}

/// Create a block-level html element.
///
/// This function returns a Node with default element style. This means it is `display: inline;`.
pub fn inline(name: &'static str, contents: Vec<Node>) -> Node {
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

pub fn span(contents: Vec<Node>) -> Node {
    inline("span", contents)
}

/// Create a html text node.
pub fn text(text: &'static str) -> Node {
    Node {
        name: &"text",
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

pub enum Quantity {
    Px(CssFloat),
    Pt(CssFloat),
    Percent(CssFloat),
    Auto
}

impl From<Quantity> for GenericSize {
    fn from(quantity: Quantity) -> GenericSize {
        use Quantity::*;
        match quantity {
            Px(val) => PixelLength(val).into(),
            Pt(val) => PixelLength::from_pt(val).into(),
            Percent(val) => Percentage(val).into(),
            Auto => GenericSize::Auto
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::style::properties::computed::{GenericSize, PixelLength};
    use super::Quantity::*;

    #[test]
    fn test_apply_margin_top_and_bottom() {
        let html = html(vec![div(vec![])
            .margin_top(Px(10.0))
            .margin_bottom(Px(20.0))]);
        assert_eq!(
            html["div"].style.position.margin_top,
            GenericSize::from(PixelLength(10.0))
        );
        assert_eq!(
            html["div"].style.position.margin_bottom,
            GenericSize::from(PixelLength(20.0))
        );
    }

}
