//! Writing modes: how the logical axes of a box (inline and block) map to the physical ones.
//!
//! Layout works in logical coordinates: `x` runs in the inline direction, along the lines, and `y`
//! in the block direction, in which the lines stack. In `horizontal-tb` these are the physical
//! `x` and `y`, and in the vertical writing modes the axes are swapped:
//!
//! | mode          | inline direction | block direction |
//! | ------------- | ---------------- | --------------- |
//! | `horizontal-tb` | left to right  | top to bottom   |
//! | `vertical-rl` | top to bottom    | right to left   |
//! | `vertical-lr` | top to bottom    | left to right   |
//!
//! Everything that depends on the writing mode goes through [`Flow`]: the conversion of the
//! physical properties of a style to logical ones (`taffy_layout::style`), the conversion of the
//! logical position of a box to its physical one (`taffy_layout`, `Writer`) and the placement of
//! the lines (`inline`). A box whose flow has another orientation than the one of its parent is
//! an orthogonal flow: it is laid out on its own, in its own logical coordinates, with the
//! available space of its parent turned around.
//!
//! Only the vertical modes that CSS needs for CJK and Mongolian are supported, and only inside a
//! horizontal flow (see `taffy_layout::tree`).

use crate::layout::components::{AbsoluteGroups, Position};
use crate::style::properties::specified::{TextOrientation, WritingMode};
use specs::prelude::*;
use taffy::geometry::Rect;

/// The flow of the content of a box: its logical axes in physical terms.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum Flow {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
}

impl From<WritingMode> for Flow {
    fn from(mode: WritingMode) -> Flow {
        match mode {
            WritingMode::HorizontalTb => Flow::HorizontalTb,
            // `sideways-rl` differs from `vertical-rl` in the orientation of the glyphs only
            WritingMode::VerticalRl | WritingMode::SidewaysRl => Flow::VerticalRl,
            // `sideways-lr` has lines that run upwards, which is not supported
            WritingMode::VerticalLr | WritingMode::SidewaysLr => Flow::VerticalLr,
        }
    }
}

impl Flow {
    /// Whether the lines are vertical.
    pub fn is_vertical(self) -> bool {
        self != Flow::HorizontalTb
    }

    /// Turns a pair of values around that belongs to the physical axes (like the width and the
    /// height) to the logical ones (the inline and the block size), or back: the swap is its own
    /// inverse.
    pub fn swap<T>(self, (first, second): (T, T)) -> (T, T) {
        if self.is_vertical() { (second, first) } else { (first, second) }
    }

    /// The physical rectangle `(x, y, width, height)` of a logical one that is given relative to
    /// the border box of the box it is in, which is `inline_size` by `block_size` in logical
    /// terms. The result is relative to the physical border box.
    pub fn rect_to_physical(
        self,
        (x, y, width, height): (f32, f32, f32, f32),
        (_inline_size, block_size): (f32, f32),
    ) -> (f32, f32, f32, f32) {
        match self {
            Flow::HorizontalTb => (x, y, width, height),
            Flow::VerticalRl => (block_size - y - height, x, height, width),
            Flow::VerticalLr => (y, x, height, width),
        }
    }

    /// The logical edges `{left: inline-start, right: inline-end, top: block-start, bottom:
    /// block-end}` of physical ones (margins, borders, padding and insets).
    pub fn edges_to_logical<T: Copy>(self, edges: Rect<T>) -> Rect<T> {
        match self {
            Flow::HorizontalTb => edges,
            Flow::VerticalRl => Rect {
                left: edges.top,
                right: edges.bottom,
                top: edges.right,
                bottom: edges.left,
            },
            Flow::VerticalLr => Rect {
                left: edges.top,
                right: edges.bottom,
                top: edges.left,
                bottom: edges.right,
            },
        }
    }

    /// The physical edges of logical ones: the inverse of [`Flow::edges_to_logical`].
    pub fn edges_to_physical<T: Copy>(self, edges: Rect<T>) -> Rect<T> {
        match self {
            Flow::HorizontalTb => edges,
            Flow::VerticalRl => Rect {
                top: edges.left,
                bottom: edges.right,
                right: edges.top,
                left: edges.bottom,
            },
            Flow::VerticalLr => Rect {
                top: edges.left,
                bottom: edges.right,
                left: edges.top,
                right: edges.bottom,
            },
        }
    }

    /// The matrix `[a, b, c, d]` of the text of a line, in the convention of the PDF text matrix
    /// (the y axis points up), whose `x` axis is the inline direction and whose `y` axis is "up"
    /// for the glyphs: horizontal text is not turned, the text of the vertical modes is turned
    /// by a quarter clockwise, so that the lines run downwards.
    pub fn text_matrix(self) -> [f32; 4] {
        match self {
            Flow::HorizontalTb => [1.0, 0.0, 0.0, 1.0],
            Flow::VerticalRl | Flow::VerticalLr => [0.0, -1.0, 1.0, 0.0],
        }
    }

    /// The physical position of the origin of a line, where the glyphs of a line sit on the
    /// baseline: `(x, y, width, height)` is the physical box of the line and `baseline` the
    /// distance of the baseline from the over side of the line (the top of a horizontal line).
    pub fn line_origin(
        self,
        (x, y, width, _height): (f32, f32, f32, f32),
        baseline: f32,
    ) -> (f32, f32) {
        match self {
            Flow::HorizontalTb => (x, y + baseline),
            // The over side of the line, where the tops of the glyphs are, is on the right in both
            // vertical modes (the glyphs are turned clockwise), whichever side the next line is
            // on. In `vertical-rl` that is also the block start edge.
            Flow::VerticalRl | Flow::VerticalLr => (x + width - baseline, y),
        }
    }
}

/// Whether a character stands upright in vertical lines (`text-orientation`) instead of being
/// turned by a quarter, by the Unicode Vertical_Orientation property (UAX #50).
pub fn is_upright(ch: char, orientation: TextOrientation) -> bool {
    use unicode_vo::Orientation;
    match orientation {
        TextOrientation::Upright => true,
        TextOrientation::Sideways => false,
        TextOrientation::Mixed => matches!(
            unicode_vo::char_orientation(ch),
            Orientation::Upright | Orientation::TransformedOrUpright
        ),
    }
}

/// Moves the content of the orthogonal flows to where the box of the flow is after pagination
/// (see [`crate::layout::components::AbsoluteGroup::anchor`]): the content was laid out with the
/// box at its place before the pages were broken.
pub struct PlaceAnchoredGroups;

impl<'a> System<'a> for PlaceAnchoredGroups {
    type SystemData = (Read<'a, AbsoluteGroups>, WriteStorage<'a, Position>);

    fn run(&mut self, (groups, mut positions): Self::SystemData) {
        for group in groups.groups.iter() {
            let Some((anchor, before)) = group.anchor else {
                continue;
            };
            let Some(&after) = positions.get(anchor) else {
                continue;
            };
            let (dx, dy) = (after.x.0 - before.x, after.y.0 - before.y);
            for &entity in &group.entities {
                if let Some(position) = positions.get_mut(entity) {
                    position.x.0 += dx;
                    position.y.0 += dy;
                    position.page = after.page;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges() -> Rect<f32> {
        Rect { left: 1.0, right: 2.0, top: 3.0, bottom: 4.0 }
    }

    #[test]
    fn edges_make_a_round_trip() {
        for flow in [Flow::HorizontalTb, Flow::VerticalRl, Flow::VerticalLr] {
            let there_and_back = flow.edges_to_physical(flow.edges_to_logical(edges()));
            assert_eq!(there_and_back, edges(), "{flow:?}");
        }
    }

    #[test]
    fn vertical_rl_block_start_is_the_right_edge() {
        let logical = Flow::VerticalRl.edges_to_logical(edges());
        // block start (top) is physical right, inline start (left) is physical top
        assert_eq!((logical.top, logical.bottom), (2.0, 1.0));
        assert_eq!((logical.left, logical.right), (3.0, 4.0));
    }

    #[test]
    fn vertical_lr_block_start_is_the_left_edge() {
        let logical = Flow::VerticalLr.edges_to_logical(edges());
        assert_eq!((logical.top, logical.bottom), (1.0, 2.0));
    }

    #[test]
    fn a_line_at_the_block_start_is_on_the_right_in_vertical_rl() {
        // a box that is 100 long (inline) and 60 wide (block): the first line is 20 wide
        let first = Flow::VerticalRl.rect_to_physical((0.0, 0.0, 100.0, 20.0), (100.0, 60.0));
        assert_eq!(first, (40.0, 0.0, 20.0, 100.0));
        let second = Flow::VerticalRl.rect_to_physical((0.0, 20.0, 100.0, 20.0), (100.0, 60.0));
        assert_eq!(second, (20.0, 0.0, 20.0, 100.0));
    }

    #[test]
    fn a_line_at_the_block_start_is_on_the_left_in_vertical_lr() {
        let second = Flow::VerticalLr.rect_to_physical((0.0, 20.0, 100.0, 20.0), (100.0, 60.0));
        assert_eq!(second, (20.0, 0.0, 20.0, 100.0));
    }

    #[test]
    fn han_characters_stand_upright_and_latin_ones_are_turned() {
        assert!(is_upright('本', TextOrientation::Mixed));
        assert!(!is_upright('A', TextOrientation::Mixed));
        // the Han punctuation has vertical forms
        assert!(is_upright('、', TextOrientation::Mixed));
        assert!(is_upright('A', TextOrientation::Upright));
        assert!(!is_upright('本', TextOrientation::Sideways));
    }

    #[test]
    fn sizes_swap() {
        assert_eq!(Flow::VerticalRl.swap((1, 2)), (2, 1));
        assert_eq!(Flow::HorizontalTb.swap((1, 2)), (1, 2));
    }

    #[test]
    fn the_baseline_is_measured_from_the_block_start_edge() {
        // a line 20 wide at x = 40, baseline 15 from the right edge
        assert_eq!(Flow::VerticalRl.line_origin((40.0, 5.0, 20.0, 100.0), 15.0), (45.0, 5.0));
        assert_eq!(Flow::HorizontalTb.line_origin((40.0, 5.0, 100.0, 20.0), 15.0), (40.0, 20.0));
    }
}
