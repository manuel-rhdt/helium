use crate::layout::entity::{Position, Text};
use crate::style::properties::computed::PixelLength;

use specs::{Join, ReadStorage, System};

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Point {
    pub x: PixelLength,
    pub y: PixelLength,
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Rect {
    pub x: PixelLength,
    pub y: PixelLength,
    pub width: PixelLength,
    pub height: PixelLength,
}

impl Point {
    pub fn new(x: PixelLength, y: PixelLength) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    LineString(Vec<Point>),
    FilledRect(Rect),
    Text(Point, String),
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    items: Vec<DisplayItem>,
}

impl DisplayList {
    pub fn append(&mut self, item: DisplayItem) {
        self.items.push(item)
    }

    pub fn items(&self) -> impl ExactSizeIterator<Item = &DisplayItem> {
        self.items.iter()
    }
}

#[derive(Debug, Default)]
pub struct DisplayListBuilder {
    display_list: DisplayList,
}

impl DisplayListBuilder {
    pub fn build(self) -> DisplayList {
        self.display_list
    }
}

impl<'a> System<'a> for DisplayListBuilder {
    type SystemData = (ReadStorage<'a, Position>, ReadStorage<'a, Text>);

    fn run(&mut self, (pos, text): Self::SystemData) {
        for (pos, text) in (&pos, text.maybe()).join() {
            if pos.width == PixelLength(0.0) || pos.height == PixelLength(0.0) {
                continue;
            }
            if let Some(text) = text {
                self.display_list.append(DisplayItem::Text(
                    Point { x: pos.x, y: pos.y },
                    text.characters.clone(),
                ));
            }
            self.display_list.append(DisplayItem::FilledRect(Rect {
                x: pos.x,
                y: pos.y,
                width: pos.width,
                height: pos.height,
            }))
        }
    }
}
