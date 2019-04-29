use crate::layout::entity::{Decoration, Position, Text};
use crate::layout::text::{font::GlyphIndex, Font};
use crate::style::properties::computed::{PixelLength, RGBA};

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
    Color(RGBA),
    Text(Point, PixelLength, Font, Vec<GlyphIndex>),
    Translate(PixelLength, PixelLength),
    Push,
    Pop,
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

    fn append_text(&mut self, pos: Point, text: &Text) {
        self.append(DisplayItem::Text(
            Point {
                y: pos.y + text.baseline,
                ..pos
            },
            text.font_size,
            text.font.clone(),
            text.glyphs.clone(),
        ));
    }
}

#[derive(Debug, Default)]
pub struct DisplayDecoration {
    pub display_list: DisplayList,
}

impl<'a> System<'a> for DisplayDecoration {
    type SystemData = (ReadStorage<'a, Position>, ReadStorage<'a, Decoration>);

    fn run(&mut self, (positions, decorations): Self::SystemData) {
        for (
            &Position {
                x,
                y,
                width,
                height,
            },
            deco,
        ) in (&positions, &decorations).join()
        {
            use PixelLength as Px;

            if width == Px(0.0) || height == Px(0.0) {
                continue;
            }

            let &Decoration {
                padding,
                border,
                background_color,
                ..
            } = deco;

            if background_color != RGBA::transparent() {
                self.display_list.append(DisplayItem::Push);
                self.display_list
                    .append(DisplayItem::Color(background_color));
                self.display_list.append(DisplayItem::FilledRect(Rect {
                    x: x - padding.left,
                    y: y - padding.top,
                    width: width + padding.left + padding.right,
                    height: height + padding.top + padding.bottom,
                }));
                self.display_list.append(DisplayItem::Pop);
            }

            let hbt = border.top / 2.0;
            let hbb = border.bottom / 2.0;
            let hbl = border.left / 2.0;
            let hbr = border.right / 2.0;

            let xmin = x - padding.left - border.left;
            let xmax = x + width + padding.right + border.right;
            let ymin = y - padding.top - border.top;
            let ymax = y + height + padding.bottom + border.bottom;

            if border.top > Px(0.0) {
                let pt1 = Point::new(xmin, ymin + hbt);
                let pt2 = Point::new(xmax, ymin + hbt);
                self.display_list.append(DisplayItem::LineString {
                    points: vec![pt1, pt2],
                    line_width: border.top,
                });
            }
            if border.bottom > Px(0.0) {
                let pt1 = Point::new(xmin, ymax - hbb);
                let pt2 = Point::new(xmax, ymax - hbb);
                self.display_list.append(DisplayItem::LineString {
                    points: vec![pt1, pt2],
                    line_width: border.bottom,
                });
            }
            if border.left > Px(0.0) {
                let pt1 = Point::new(xmin + hbl, ymin);
                let pt2 = Point::new(xmin + hbl, ymax);
                self.display_list.append(DisplayItem::LineString {
                    points: vec![pt1, pt2],
                    line_width: border.left,
                });
            }
            if border.right > Px(0.0) {
                let pt1 = Point::new(xmax - hbr, ymin);
                let pt2 = Point::new(xmax - hbr, ymax);
                self.display_list.append(DisplayItem::LineString {
                    points: vec![pt1, pt2],
                    line_width: border.right,
                });
            }
        }
    }
}

pub struct DisplayText {
    pub display_list: DisplayList,
}

impl<'a> System<'a> for DisplayText {
    type SystemData = (ReadStorage<'a, Position>, ReadStorage<'a, Text>);

    fn run(&mut self, (positions, text): Self::SystemData) {
        for (&Position { x, y, .. }, text) in (&positions, &text).join() {
            self.display_list.append_text(Point { x, y }, text)
        }
    }
}
