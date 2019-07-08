use crate::layout::entity::{Decoration, Position, Text, ZIndex};
use crate::layout::inline::LineBox;
use crate::layout::normal_formatting::{Page, Pages};
use crate::layout::text::{font::GlyphIndex, Font};
use crate::style::properties::computed::{PixelLength, RGBA};

use specs::prelude::*;

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

#[derive(Debug, Clone, PartialEq)]
struct DisplayElement {
    z_index: ZIndex,
    item: DisplayItem,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    items: Vec<DisplayElement>,
}

impl DisplayList {
    pub fn append(&mut self, item: DisplayItem, z_index: ZIndex) {
        self.items.push(DisplayElement { item, z_index })
    }

    pub fn items(&mut self) -> impl ExactSizeIterator<Item = &DisplayItem> {
        self.items.sort_by_key(|elem| ZIndex(elem.z_index.0));
        self.items.iter().map(|elem| &elem.item)
    }

    fn append_text(&mut self, pos: Point, text: &Text, z_index: ZIndex) {
        self.append(
            DisplayItem::Text(
                Point {
                    y: pos.y + text.baseline,
                    ..pos
                },
                text.font_size,
                text.font.clone(),
                text.glyphs.clone(),
            ),
            z_index,
        );
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct PrintPages {
    pub pages: Vec<DisplayList>,
}

impl PrintPages {
    fn append_page(&mut self, page: DisplayList) {
        self.pages.push(page)
    }
}

#[derive(Debug, Default)]
pub struct DisplayDecoration {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
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
                z,
            },
            deco,
            _,
        ) in (&positions, &decorations, &self.bitset).join()
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
                self.display_list.append(DisplayItem::Push, z);
                self.display_list
                    .append(DisplayItem::Color(background_color), z);
                self.display_list.append(
                    DisplayItem::FilledRect(Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    }),
                    z,
                );
                self.display_list.append(DisplayItem::Pop, z);
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
                self.display_list.append(
                    DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border.top,
                    },
                    z,
                );
            }
            if border.bottom > Px(0.0) {
                let pt1 = Point::new(xmin, ymax - hbb);
                let pt2 = Point::new(xmax, ymax - hbb);
                self.display_list.append(
                    DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border.bottom,
                    },
                    z,
                );
            }
            if border.left > Px(0.0) {
                let pt1 = Point::new(xmin + hbl, ymin);
                let pt2 = Point::new(xmin + hbl, ymax);
                self.display_list.append(
                    DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border.left,
                    },
                    z,
                );
            }
            if border.right > Px(0.0) {
                let pt1 = Point::new(xmax - hbr, ymin);
                let pt2 = Point::new(xmax - hbr, ymax);
                self.display_list.append(
                    DisplayItem::LineString {
                        points: vec![pt1, pt2],
                        line_width: border.right,
                    },
                    z,
                );
            }
        }
    }
}

pub struct DisplayText {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayText {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, Text>,
        ReadStorage<'a, LineBox>,
    );

    fn run(&mut self, (entities, positions, text, line_boxes): Self::SystemData) {
        for (line_box, _) in (&line_boxes, &self.bitset).join() {
            for &fragment in &line_box.fragments {
                let joint_comps = (&positions, &text).join().get(fragment, &entities);
                if let Some((&Position { x, y, z, .. }, text)) = joint_comps {
                    self.display_list.append_text(Point { x, y }, text, z)
                }
            }
        }
    }
}

pub fn generate_display_list(world: &mut specs::World) -> PrintPages {
    let pages = world.read_resource::<Pages>();
    let page_data = world.read_storage::<Page>();
    let mut print_pages = PrintPages::default();
    for &page in &pages.0 {
        let page_data = if let Some(page) = page_data.get(page) {
            page
        } else {
            continue;
        };

        let display_list = DisplayList::default();
        let mut bitset = specs::BitSet::new();
        page_data.add_elements_to_set(&mut bitset);

        let mut decorations_builder = DisplayDecoration {
            display_list,
            bitset,
        };
        decorations_builder.run_now(world);

        let display_list = decorations_builder.display_list;
        let bitset = decorations_builder.bitset;

        let mut text_builder = DisplayText {
            display_list,
            bitset,
        };
        text_builder.run_now(world);
        let display_list = text_builder.display_list;

        print_pages.append_page(display_list);
    }

    print_pages
}
