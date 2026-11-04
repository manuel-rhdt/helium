//! Constraint solving for absolutely positioned boxes.
//!
//! Implements the equations of CSS 2.1 sections 10.3.7 (width and horizontal margins) and 10.6.4
//! (height and vertical margins) for non-replaced elements in a left-to-right context.
//!
//! All offsets are measured from the padding edge of the containing block; `static_*` is the
//! offset of the margin edge of the box in its static position. `extras` is the sum of padding
//! and border on the respective axis.

/// Input of one axis. `None` stands for `auto`.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Axis {
    /// Size of the containing block on this axis.
    pub container: f32,
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub size: Option<f32>,
    pub margin_start: Option<f32>,
    pub margin_end: Option<f32>,
    pub extras: f32,
    pub static_start: f32,
}

/// Used values of one axis.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AxisResult {
    /// Offset of the margin edge from the start of the containing block.
    pub start: f32,
    /// Content size.
    pub size: f32,
    pub margin_start: f32,
    pub margin_end: f32,
}

/// `fallback_size` is called when the size of the content decides the used size: with the
/// available space on the axis it must return the shrink-to-fit width (horizontal) or the height
/// of the content (vertical).
fn solve(
    axis: Axis,
    negative_free_space_goes_to_start: bool,
    fallback_size: impl FnOnce(f32) -> f32,
) -> AxisResult {
    let Axis {
        container,
        start,
        end,
        size,
        margin_start,
        margin_end,
        extras,
        static_start,
    } = axis;

    // The space that is left when the given values are subtracted from the container.
    let free = |start: f32, size: f32, end: f32, ms: f32, me: f32| {
        container - start - size - end - ms - me - extras
    };

    match (start, size, end) {
        (None, None, None) => {
            let (ms, me) = (margin_start.unwrap_or(0.0), margin_end.unwrap_or(0.0));
            let start = static_start;
            let size = fallback_size(container - start - ms - me - extras);
            AxisResult {
                start,
                size,
                margin_start: ms,
                margin_end: me,
            }
        }
        (Some(start), Some(size), Some(end)) => {
            let (ms, me) = match (margin_start, margin_end) {
                (None, None) => {
                    let free = free(start, size, end, 0.0, 0.0);
                    if free >= 0.0 {
                        (free / 2.0, free / 2.0)
                    } else if negative_free_space_goes_to_start {
                        (0.0, free)
                    } else {
                        (free, 0.0)
                    }
                }
                (None, Some(me)) => (free(start, size, end, 0.0, me), me),
                (Some(ms), None) => (ms, free(start, size, end, ms, 0.0)),
                // over-constrained: `end` is ignored
                (Some(ms), Some(me)) => (ms, me),
            };
            AxisResult {
                start,
                size,
                margin_start: ms,
                margin_end: me,
            }
        }
        _ => {
            let (ms, me) = (margin_start.unwrap_or(0.0), margin_end.unwrap_or(0.0));
            match (start, size, end) {
                (None, None, Some(end)) => {
                    let size = fallback_size(container - end - ms - me - extras);
                    AxisResult {
                        start: free(0.0, size, end, ms, me),
                        size,
                        margin_start: ms,
                        margin_end: me,
                    }
                }
                (None, Some(size), None) => AxisResult {
                    start: static_start,
                    size,
                    margin_start: ms,
                    margin_end: me,
                },
                (Some(start), None, None) => {
                    let size = fallback_size(container - start - ms - me - extras);
                    AxisResult {
                        start,
                        size,
                        margin_start: ms,
                        margin_end: me,
                    }
                }
                (None, Some(size), Some(end)) => AxisResult {
                    start: free(0.0, size, end, ms, me),
                    size,
                    margin_start: ms,
                    margin_end: me,
                },
                (Some(start), None, Some(end)) => {
                    let size = free(start, 0.0, end, ms, me).max(0.0);
                    AxisResult {
                        start,
                        size,
                        margin_start: ms,
                        margin_end: me,
                    }
                }
                (Some(start), Some(size), None) => AxisResult {
                    start,
                    size,
                    margin_start: ms,
                    margin_end: me,
                },
                _ => unreachable!("all-auto and none-auto were handled above"),
            }
        }
    }
}

/// https://www.w3.org/TR/CSS22/visudet.html#abs-non-replaced-width
///
/// `shrink_to_fit` gets the available width and returns the used width.
pub fn solve_horizontal(axis: Axis, shrink_to_fit: impl FnOnce(f32) -> f32) -> AxisResult {
    solve(axis, true, |available| shrink_to_fit(available.max(0.0)))
}

/// https://www.w3.org/TR/CSS22/visudet.html#abs-non-replaced-height
///
/// `content_height` is the height of the laid out content, used when `height` is `auto` and the
/// height is not determined by `top` and `bottom`.
pub fn solve_vertical(axis: Axis, content_height: f32) -> AxisResult {
    solve(axis, false, |_| content_height)
}

/// The shrink-to-fit width: `min(max(preferred minimum width, available width), preferred width)`.
pub fn shrink_to_fit(min_content: f32, max_content: f32, available: f32) -> f32 {
    min_content.max(available).min(max_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis() -> Axis {
        Axis {
            container: 200.0,
            start: None,
            end: None,
            size: None,
            margin_start: None,
            margin_end: None,
            extras: 0.0,
            static_start: 7.0,
        }
    }

    fn horizontal(axis: Axis) -> AxisResult {
        solve_horizontal(axis, |available| shrink_to_fit(30.0, 120.0, available))
    }

    #[test]
    fn all_auto_uses_static_position_and_shrink_to_fit() {
        let r = horizontal(axis());
        assert_eq!(
            r,
            AxisResult {
                start: 7.0,
                size: 120.0,
                margin_start: 0.0,
                margin_end: 0.0
            }
        );
        // narrow container: the available width wins over the preferred width
        let r = horizontal(Axis {
            container: 100.0,
            ..axis()
        });
        assert_eq!(r.size, 93.0);
    }

    #[test]
    fn left_and_right_with_auto_width_fill_the_container() {
        let r = horizontal(Axis {
            start: Some(10.0),
            end: Some(20.0),
            extras: 6.0,
            ..axis()
        });
        assert_eq!(r.size, 164.0);
        assert_eq!(r.start, 10.0);
    }

    #[test]
    fn auto_margins_center_the_box() {
        let r = horizontal(Axis {
            start: Some(0.0),
            end: Some(0.0),
            size: Some(100.0),
            ..axis()
        });
        assert_eq!((r.margin_start, r.margin_end), (50.0, 50.0));
    }

    #[test]
    fn negative_free_space_is_given_to_the_end_margin_horizontally() {
        let r = horizontal(Axis {
            start: Some(0.0),
            end: Some(0.0),
            size: Some(300.0),
            ..axis()
        });
        assert_eq!((r.margin_start, r.margin_end), (0.0, -100.0));
    }

    #[test]
    fn over_constrained_ignores_end() {
        let r = horizontal(Axis {
            start: Some(10.0),
            end: Some(10.0),
            size: Some(50.0),
            margin_start: Some(5.0),
            margin_end: Some(5.0),
            ..axis()
        });
        assert_eq!(
            (r.start, r.size, r.margin_start, r.margin_end),
            (10.0, 50.0, 5.0, 5.0)
        );
    }

    #[test]
    fn only_end_set_solves_start() {
        let r = horizontal(Axis {
            end: Some(20.0),
            size: Some(50.0),
            ..axis()
        });
        assert_eq!(r.start, 130.0);
        // with auto size the width is shrink-to-fit
        let r = horizontal(Axis {
            end: Some(20.0),
            ..axis()
        });
        assert_eq!((r.size, r.start), (120.0, 60.0));
    }

    #[test]
    fn only_start_and_size_set() {
        let r = horizontal(Axis {
            start: Some(20.0),
            size: Some(50.0),
            ..axis()
        });
        assert_eq!((r.start, r.size), (20.0, 50.0));
    }

    #[test]
    fn vertical_auto_height_uses_content_height() {
        let r = solve_vertical(
            Axis {
                start: Some(10.0),
                ..axis()
            },
            42.0,
        );
        assert_eq!((r.start, r.size), (10.0, 42.0));
        let r = solve_vertical(axis(), 42.0);
        assert_eq!((r.start, r.size), (7.0, 42.0));
    }

    #[test]
    fn vertical_top_and_bottom_determine_height() {
        let r = solve_vertical(
            Axis {
                start: Some(10.0),
                end: Some(30.0),
                ..axis()
            },
            5.0,
        );
        assert_eq!(r.size, 160.0);
    }

    #[test]
    fn vertical_bottom_only_positions_from_the_bottom() {
        let r = solve_vertical(
            Axis {
                end: Some(10.0),
                ..axis()
            },
            40.0,
        );
        assert_eq!((r.start, r.size), (150.0, 40.0));
    }

    #[test]
    fn vertical_negative_free_space_goes_to_the_start_margin() {
        let r = solve_vertical(
            Axis {
                start: Some(0.0),
                end: Some(0.0),
                size: Some(300.0),
                ..axis()
            },
            0.0,
        );
        assert_eq!((r.margin_start, r.margin_end), (-100.0, 0.0));
    }
}
