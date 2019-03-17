use crate::layout::entity::Position;

use specs::{Join, ReadStorage, System};

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    LineString(Vec<Point>),
    FilledRect(Rect),
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
    type SystemData = ReadStorage<'a, Position>;

    fn run(&mut self, pos: Self::SystemData) {
        for pos in pos.join() {
            if pos.width == 0.0 || pos.height == 0.0 {
                continue;
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
