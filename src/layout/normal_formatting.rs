//! The block formatting context of a box and the pages that the laid out boxes are put on.

use super::components::{BlockLevelElement, Position};
use crate::style::properties::computed::PixelLength;

use std::collections::BTreeMap;

use specs::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockFormattingContext {
    pub elements: Vec<BlockLevelElement>,
}

impl Component for BlockFormattingContext {
    type Storage = DenseVecStorage<Self>;
}

/// A struct that represents a page. Contains a list of all entities that are displayed on the page.
#[derive(Debug, Clone)]
pub struct Page {
    pub size: (PixelLength, PixelLength),
    elements: specs::BitSet,
}

impl Page {
    pub fn new(width: PixelLength, height: PixelLength) -> Self {
        Page {
            size: (width, height),
            elements: Default::default(),
        }
    }

    pub fn append_element(&mut self, element: Entity) {
        if cfg!(debug_assertions) && self.elements.contains(element.id()) {
            panic!(
                "Tried to append the entity {:?} to the page although it already existed",
                element
            )
        }
        self.elements.add(element.id());
    }

    pub fn add_elements_to_set(&self, set: &mut specs::BitSet) {
        for entity in &self.elements {
            set.add(entity);
        }
    }

    pub fn contains(&self, element: Entity) -> bool {
        self.elements.contains(element.id())
    }
}

/// A resource that holds a sorted vector of all page entities.
#[derive(Debug, Default, Clone)]
pub struct Pages(BTreeMap<usize, Page>);

impl Pages {
    pub fn page(&self, index: usize) -> &Page {
        &self.0[&index]
    }

    pub fn iter(&self) -> impl Iterator<Item = &Page> {
        self.0.values()
    }

    pub fn num_pages(&self) -> usize {
        self.0.keys().last().map(|x| x + 1).unwrap_or_default()
    }
}

pub struct PagesCreate {
    pub page_width: PixelLength,
    pub page_height: PixelLength,
}

impl<'a> System<'a> for PagesCreate {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        Write<'a, Option<Pages>>,
        Read<'a, super::pages::Pages>,
        Read<'a, super::pages::PageMargins>,
    );

    fn run(&mut self, (entities, positions, mut pages, contexts, margins): Self::SystemData) {
        let mut page_list = Pages::default();
        // every page has its own size, and the pages that are empty are there, too
        for (index, context) in contexts.0.iter().enumerate() {
            let (width, height) = margins.size(context, (self.page_width, self.page_height));
            page_list.0.insert(index, Page::new(width, height));
        }
        for (entity, &Position { page, .. }) in (&entities, &positions).join() {
            let page = page_list
                .0
                .entry(page)
                .or_insert_with(|| Page::new(self.page_width, self.page_height));
            page.append_element(entity);
        }
        *pages = Some(page_list);
    }
}
