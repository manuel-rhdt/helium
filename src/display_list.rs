#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct Point {
    x: f32,
    y: f32,
}

pub enum DisplayItem {
    LineString(Vec<Point>)
}

pub struct DisplayList {
    items: Vec<DisplayItem>
}