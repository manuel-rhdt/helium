use crate::layout::entity::{Position, Text};
use crate::style::properties::computed::{ComputedValues, PixelLength};

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
    LineString {
        points: Vec<Point>,
        line_width: PixelLength,
    },
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
        ReadStorage<'a, Text>,
        ReadStorage<'a, ComputedValues>,
    );

    fn run(&mut self, (positions, text, computed_values): Self::SystemData) {
        for (
            &Position {
                x,
                y,
                width,
                height,
            },
            text,
            style,
        ) in (&positions, text.maybe(), computed_values.maybe()).join()
        {
            if width == PixelLength(0.0) || height == PixelLength(0.0) {
                continue;
            }

            if let Some(style) = style {
                let border_top = style
                    .position
                    .border_top_width
                    .absolute(style.font.font_size);
                let border_bottom = style
                    .position
                    .border_bottom_width
                    .absolute(style.font.font_size);
                let border_left = style
                    .position
                    .border_left_width
                    .absolute(style.font.font_size);
                let border_right = style
                    .position
                    .border_right_width
                    .absolute(style.font.font_size);

                let hbt = border_top / 2.0;
                let hbb = border_bottom / 2.0;
                let hbl = border_left / 2.0;
                let hbr = border_right / 2.0;

                if border_top > PixelLength(0.0) {
                    let pt1 = Point::new(x - border_left, y - hbt);
                    let pt2 = Point::new(x + width + border_right, y - hbt);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_top,
                    });
                }
                if border_bottom > PixelLength(0.0) {
                    let pt1 = Point::new(x - border_left, y + height + hbb);
                    let pt2 = Point::new(x + width + border_right, y + height + hbb);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_bottom,
                    });
                }
                if border_left > PixelLength(0.0) {
                    let pt1 = Point::new(x - hbl, y);
                    let pt2 = Point::new(x - hbl, y + height);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_left,
                    });
                }
                if border_right > PixelLength(0.0) {
                    let pt1 = Point::new(x + width + hbr, y);
                    let pt2 = Point::new(x + width + hbr, y + height);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_right,
                    });
                }
            }

            if let Some(text) = text {
                self.append_text(Point { x, y }, text);
            }
        }
    }
}
