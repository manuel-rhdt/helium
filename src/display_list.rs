use crate::layout::fragment::Fragment;

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
    pub height: f32
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    LineString(Vec<Point>),
    FilledRect(Rect)
}

#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    items: Vec<DisplayItem>,
}

impl DisplayList {
    pub fn append(&mut self, item: DisplayItem) {
        self.items.push(item)
    }

    pub fn append_fragment(&mut self, fragment: Fragment) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: fragment.content_size.width,
            height: fragment.content_size.height
        };
        self.append(DisplayItem::FilledRect(rect))
    }

    pub fn items(&self) -> impl ExactSizeIterator<Item=&DisplayItem> {
        self.items.iter()
    }
}

use crate::layout::flow::FlowRef;
use crate::traverse::Traversal;

#[derive(Debug, Default)]
pub struct DisplayListBuilder {
    display_list: DisplayList
}

impl DisplayListBuilder {
    pub fn build(self) -> DisplayList {
        self.display_list
    }
}

impl Traversal<FlowRef> for DisplayListBuilder {
    /// Compute the inline sizes of all elements.
    fn preorder_traverse(&mut self, node: FlowRef) {
        node.build_display_list(&mut self.display_list)
    }

    fn postorder_traverse(&mut self, _node: FlowRef) {}
}