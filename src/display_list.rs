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
            use PixelLength as Px;

            if width == Px(0.0) || height == Px(0.0) {
                continue;
            }

            if let Some(ComputedValues { position, font, .. }) = style {
                let padding_top = position.padding_top.absolute(Px(0.0), font.font_size);
                let padding_bottom = position.padding_bottom.absolute(Px(0.0), font.font_size);
                let padding_left = position.padding_left.absolute(Px(0.0), font.font_size);
                let padding_right = position.padding_right.absolute(Px(0.0), font.font_size);

                let border_top = position.border_top_width.absolute(font.font_size);
                let border_bottom = position.border_bottom_width.absolute(font.font_size);
                let border_left = position.border_left_width.absolute(font.font_size);
                let border_right = position.border_right_width.absolute(font.font_size);

                let hbt = border_top / 2.0;
                let hbb = border_bottom / 2.0;
                let hbl = border_left / 2.0;
                let hbr = border_right / 2.0;

                let xmin = x - padding_left - border_left;
                let xmax = x + width + padding_right + border_right;
                let ymin = y - padding_top - border_top;
                let ymax = y + height + padding_bottom + border_bottom;

                if border_top > PixelLength(0.0) {
                    let pt1 = Point::new(xmin, ymin + hbt);
                    let pt2 = Point::new(xmax, ymin + hbt);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_top,
                    });
                }
                if border_bottom > PixelLength(0.0) {
                    let pt1 = Point::new(xmin, ymax - hbb);
                    let pt2 = Point::new(xmax, ymax - hbb);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_bottom,
                    });
                }
                if border_left > PixelLength(0.0) {
                    let pt1 = Point::new(xmin + hbl, ymin);
                    let pt2 = Point::new(xmin + hbl, ymax);
                    self.display_list.append(DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border_left,
                    });
                }
                if border_right > PixelLength(0.0) {
                    let pt1 = Point::new(xmax - hbr, ymin);
                    let pt2 = Point::new(xmax - hbr, ymax);
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
