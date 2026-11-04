use crate::layout::components::{
    Decoration, Glyphs, GraphicsOp, Hyperlink, Hyperlinks, Paragraph, Position, VectorGraphics,
    ZIndex,
};
use crate::layout::normal_formatting::Pages;
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

impl Rect {
    pub fn xmax(self) -> PixelLength {
        self.x + self.width
    }

    pub fn ymax(self) -> PixelLength {
        self.y + self.height
    }

    /// Create a rect exactly large enough to encompass `self` and `other`.
    pub fn union(self, other: Rect) -> Rect {
        let xmin = if self.x <= other.x { self.x } else { other.x };
        let ymin = if self.y <= other.y { self.y } else { other.y };
        let xmax = if self.xmax() >= other.xmax() {
            self.xmax()
        } else {
            other.xmax()
        };
        let ymax = if self.ymax() >= other.ymax() {
            self.ymax()
        } else {
            other.ymax()
        };
        Rect {
            x: xmin,
            y: ymin,
            width: xmax - xmin,
            height: ymax - ymin,
        }
    }
}

impl Point {
    pub fn new(x: PixelLength, y: PixelLength) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone)]
pub enum DisplayItem {
    LineString {
        points: Vec<Point>,
        line_width: PixelLength,
    },
    FilledRect(Rect),
    Color(RGBA),
    StrokeColor(RGBA),
    Text(Point, PixelLength, String, Glyphs),
    Paragraph(Paragraph),
    Translate(PixelLength, PixelLength),
    Push,
    Pop,
}

#[derive(Debug, Clone)]
struct DisplayElement {
    z_index: ZIndex,
    entity: Entity,
    item: DisplayItem,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    items: Vec<DisplayElement>,
}

impl DisplayList {
    pub fn append(&mut self, item: DisplayItem, entity: Entity, z_index: ZIndex) {
        self.items.push(DisplayElement {
            item,
            entity,
            z_index,
        })
    }

    pub fn items(&mut self) -> impl ExactSizeIterator<Item = &DisplayItem> {
        self.items.sort_by_key(|elem| ZIndex(elem.z_index.0));
        self.items.iter().map(|elem| &elem.item)
    }

    fn stroked_line(
        &mut self,
        start: Point,
        end: Point,
        width: PixelLength,
        color: RGBA,
        entity: Entity,
        z: ZIndex,
    ) {
        self.append(DisplayItem::Push, entity, z);
        self.append(DisplayItem::StrokeColor(color), entity, z);
        self.append(
            DisplayItem::LineString {
                points: vec![start, end],
                line_width: width,
            },
            entity,
            z,
        );
        self.append(DisplayItem::Pop, entity, z);
    }

    // fn append_text(&mut self, pos: Point, text: &components::Text, z_index: ZIndex) {
    //     self.append(
    //         DisplayItem::Text(
    //             Point {
    //                 y: pos.y + text.baseline,
    //                 ..pos
    //             },
    //             text.font_size,
    //             text.font.clone(),
    //             text.glyphs.clone(),
    //         ),
    //         z_index,
    //     );
    // }
}

#[derive(Debug, Clone)]
pub struct PrintPage {
    pub display_list: DisplayList,
    pub width: PixelLength,
    pub height: PixelLength,
    pub content_rect: Rect,
}

pub struct PrintPages {
    pub font_map: crate::layout::FontMap,
    pub pages: Vec<PrintPage>,
    /// All hyperlinks in the document, in page order.
    pub hyperlinks: Vec<Hyperlink>,
}

impl PrintPages {
    pub fn append_page(&mut self, page: PrintPage) {
        self.pages.push(page)
    }
}

#[derive(Debug, Default)]
pub struct DisplayDecoration {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayDecoration {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, Decoration>,
        ReadStorage<'a, crate::layout::components::Fragmented>,
    );

    fn run(&mut self, (entities, positions, decorations, fragmented): Self::SystemData) {
        for (
            entity,
            &Position {
                x,
                y,
                width,
                height,
                z,
                ..
            },
            deco,
            _,
            _,
        ) in (
            &entities,
            &positions,
            &decorations,
            &self.bitset,
            !&fragmented,
        )
            .join()
        {
            use PixelLength as Px;

            let &Decoration {
                padding,
                border,
                background_color,
                border_color,
                ..
            } = deco;

            if background_color != RGBA::transparent() {
                self.display_list.append(DisplayItem::Push, entity, z);
                self.display_list
                    .append(DisplayItem::Color(background_color), entity, z);
                self.display_list.append(
                    DisplayItem::FilledRect(Rect {
                        x: x - padding.left,
                        y: y - padding.top,
                        width: width + padding.left + padding.right,
                        height: height + padding.top + padding.bottom,
                    }),
                    entity,
                    z,
                );
                self.display_list.append(DisplayItem::Pop, entity, z);
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

                self.display_list
                    .stroked_line(pt1, pt2, border.top, border_color.top, entity, z);
            }
            if border.bottom > Px(0.0) {
                let pt1 = Point::new(xmin, ymax - hbb);
                let pt2 = Point::new(xmax, ymax - hbb);
                self.display_list.stroked_line(
                    pt1,
                    pt2,
                    border.bottom,
                    border_color.bottom,
                    entity,
                    z,
                );
            }
            if border.left > Px(0.0) {
                let pt1 = Point::new(xmin + hbl, ymin);
                let pt2 = Point::new(xmin + hbl, ymax);
                self.display_list
                    .stroked_line(pt1, pt2, border.left, border_color.left, entity, z);
            }
            if border.right > Px(0.0) {
                let pt1 = Point::new(xmax - hbr, ymin);
                let pt2 = Point::new(xmax - hbr, ymax);
                self.display_list.stroked_line(
                    pt1,
                    pt2,
                    border.right,
                    border_color.right,
                    entity,
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
        ReadStorage<'a, Paragraph>,
    );

    fn run(&mut self, (entities, positions, paragraphs): Self::SystemData) {
        for (entity, pos, paragraph, _) in (&entities, &positions, &paragraphs, &self.bitset).join()
        {
            self.display_list
                .append(DisplayItem::Paragraph(paragraph.clone()), entity, pos.z)
        }
    }
}

pub struct DisplayVectorGraphics {
    pub display_list: DisplayList,
    pub bitset: specs::BitSet,
}

impl<'a> System<'a> for DisplayVectorGraphics {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, Position>,
        ReadStorage<'a, VectorGraphics>,
    );

    fn run(&mut self, (entities, positions, vector_graphics): Self::SystemData) {
        for (entity, pos, vector_graphic, _) in
            (&entities, &positions, &vector_graphics, &self.bitset).join()
        {
            if let Some(color) = vector_graphic.color {
                self.display_list.append(DisplayItem::Push, entity, pos.z);
                self.display_list
                    .append(DisplayItem::StrokeColor(color), entity, pos.z);
            }
            for op in &vector_graphic.ops {
                match op {
                    GraphicsOp::Line {
                        from_x,
                        from_y,
                        to_x,
                        to_y,
                        thickness,
                    } => self.display_list.append(
                        DisplayItem::LineString {
                            points: vec![
                                Point {
                                    x: *from_x,
                                    y: *from_y,
                                },
                                Point { x: *to_x, y: *to_y },
                            ],
                            line_width: *thickness,
                        },
                        entity,
                        pos.z,
                    ),
                }
            }
            if vector_graphic.color.is_some() {
                self.display_list.append(DisplayItem::Pop, entity, pos.z);
            }
        }
    }
}

pub struct ContentRectCalculator {
    pub bitset: specs::BitSet,
    pub content_rect: Option<Rect>,
}

impl<'a> System<'a> for ContentRectCalculator {
    type SystemData = (ReadStorage<'a, Position>,);

    fn run(&mut self, (positions,): Self::SystemData) {
        for (
            &Position {
                x,
                y,
                width,
                height,
                ..
            },
            ..,
        ) in (&positions, &self.bitset).join()
        {
            if width == PixelLength(0.0) || height == PixelLength(0.0) {
                continue;
            }
            if let Some(r) = self.content_rect {
                self.content_rect = Some(r.union(Rect {
                    x,
                    y,
                    width,
                    height,
                }))
            } else {
                self.content_rect = Some(Rect {
                    x,
                    y,
                    width,
                    height,
                })
            }
        }
    }
}

pub fn generate_display_list(world: &mut specs::World) -> PrintPages {
    <DisplayText as System>::SystemData::setup(world);
    <DisplayVectorGraphics as System>::SystemData::setup(world);

    let pages = world.read_resource::<Option<Pages>>();
    let pages = pages.as_ref().unwrap();
    let mut page_vec = vec![];
    for page in pages.iter() {
        let display_list = DisplayList::default();
        let mut bitset = specs::BitSet::new();
        page.add_elements_to_set(&mut bitset);

        let mut decorations_builder = DisplayDecoration {
            display_list,
            bitset,
        };
        decorations_builder.run_now(world);

        let display_list = decorations_builder.display_list;
        let bitset = decorations_builder.bitset;

        let mut graphics_builder = DisplayVectorGraphics {
            display_list,
            bitset,
        };
        graphics_builder.run_now(world);

        let display_list = graphics_builder.display_list;
        let bitset = graphics_builder.bitset;

        let mut text_builder = DisplayText {
            display_list,
            bitset,
        };
        text_builder.run_now(world);
        let display_list = text_builder.display_list;

        let bitset = text_builder.bitset;

        let mut content_rect_calculator = ContentRectCalculator {
            bitset,
            content_rect: None,
        };
        content_rect_calculator.run_now(world);
        let content_rect = content_rect_calculator.content_rect.unwrap_or_default();

        page_vec.push(PrintPage {
            display_list,
            width: page.size.0,
            height: page.size.1,
            content_rect,
        });
    }

    PrintPages {
        font_map: (*world.read_resource::<crate::layout::FontMap>()).clone(),
        pages: page_vec,
        hyperlinks: world.read_resource::<Hyperlinks>().0.clone(),
    }
}
