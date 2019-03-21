use crate::layout::entity::{Position, StackingContextRelativePosition, Text};
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
    Text(Point, PixelLength, String),
    Translate(PixelLength, PixelLength),
    PushTransform,
    RestoreTransform,
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

    fn append_text(&mut self, pos: Point, text: &Text) {
        self.display_list.append(DisplayItem::Text(
            Point {
                y: pos.y + text.baseline,
                ..pos
            },
            text.font_size,
            text.characters.clone(),
        ));
    }
}

impl<'a> System<'a> for DisplayListBuilder {
    type SystemData = (
        ReadStorage<'a, Position>,
        ReadStorage<'a, StackingContextRelativePosition>,
        ReadStorage<'a, Text>,
    );

    fn run(&mut self, (positions, global_pos, text): Self::SystemData) {
        for (&StackingContextRelativePosition { x, y }, &Position { width, height, .. }, text) in
            (&global_pos, &positions, text.maybe()).join()
        {
            if width == PixelLength(0.0) || height == PixelLength(0.0) {
                continue;
            }

            // self.display_list.append(DisplayItem::FilledRect(Rect {
            //     x,
            //     y,
            //     width,
            //     height,
            // }));

            if let Some(text) = text {
                self.append_text(Point { x, y }, text);
            }
        }
    }
}
