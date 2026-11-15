//! The size of a replaced box that has a preferred ratio (an image, a `<canvas>`) when `min-` and
//! `max-width` and `-height` limit it.
//!
//! Taffy limits each side on its own, so a box that is too wide keeps its height. CSS 2.1 keeps the
//! proportions: the table of the constraint violations in section 10.4.
//! <https://www.w3.org/TR/CSS22/visudet.html#min-max-widths>

/// The content size `(width, height)` of a replaced box.
///
/// * `natural` is the size of the image, which also gives the ratio;
/// * `specified` are the `width` and `height` of the box if they are not `auto`;
/// * `min` and `max` are the limits (`0` and infinity if there are none).
///
/// A box with both sides specified has no ratio to keep: each side is limited on its own.
pub(super) fn constrain(
    natural: (f32, f32),
    specified: (Option<f32>, Option<f32>),
    min: (f32, f32),
    max: (f32, f32),
) -> (f32, f32) {
    // a limit that is below the minimum is the minimum
    let (min_w, min_h) = min;
    let (max_w, max_h) = (max.0.max(min_w), max.1.max(min_h));
    if natural.0 <= 0.0 || natural.1 <= 0.0 {
        return (
            specified.0.unwrap_or(natural.0).clamp(min_w, max_w),
            specified.1.unwrap_or(natural.1).clamp(min_h, max_h),
        );
    }
    let ratio = natural.0 / natural.1;
    let (w, h) = match specified {
        (Some(w), Some(h)) => return (w.clamp(min_w, max_w), h.clamp(min_h, max_h)),
        (Some(w), None) => (w, w / ratio),
        (None, Some(h)) => (h * ratio, h),
        (None, None) => natural,
    };
    // the table of the constraint violations
    let too_narrow = w < min_w;
    let too_wide = w > max_w;
    let too_short = h < min_h;
    let too_tall = h > max_h;
    match (too_narrow, too_wide, too_short, too_tall) {
        (false, false, false, false) => (w, h),
        (false, true, false, true) => {
            if max_w / w <= max_h / h {
                (max_w, min_h.max(max_w / ratio))
            } else {
                (min_w.max(max_h * ratio), max_h)
            }
        }
        (true, false, true, false) => {
            if min_w / w <= min_h / h {
                (max_w.min(min_h * ratio), min_h)
            } else {
                (min_w, max_h.min(min_w / ratio))
            }
        }
        (true, false, false, true) => (min_w, max_h),
        (false, true, true, false) => (max_w, min_h),
        (false, true, _, _) => (max_w, min_h.max(max_w / ratio)),
        (true, false, _, _) => (min_w, max_h.min(min_w / ratio)),
        (_, _, false, true) => (min_w.max(max_h * ratio), max_h),
        (_, _, true, false) => (max_w.min(min_h * ratio), min_h),
        // (a side cannot be both too small and too big)
        _ => (w.clamp(min_w, max_w), h.clamp(min_h, max_h)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: (f32, f32) = (f32::INFINITY, f32::INFINITY);
    const ZERO: (f32, f32) = (0.0, 0.0);

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn without_limits_the_size_follows_the_specified_side() {
        assert!(close(constrain((100.0, 50.0), (None, None), ZERO, NONE), (100.0, 50.0)));
        assert!(close(constrain((100.0, 50.0), (Some(300.0), None), ZERO, NONE), (300.0, 150.0)));
        assert!(close(constrain((100.0, 50.0), (None, Some(10.0)), ZERO, NONE), (20.0, 10.0)));
    }

    #[test]
    fn a_box_that_is_too_wide_keeps_its_proportions() {
        // `width: 300px; max-width: 200px`
        assert!(close(constrain((100.0, 100.0), (Some(300.0), None), ZERO, (200.0, f32::INFINITY)), (200.0, 200.0)));
    }

    #[test]
    fn a_box_that_is_too_tall_keeps_its_proportions() {
        // `height: 300px; max-height: 80px`
        assert!(close(constrain((100.0, 100.0), (None, Some(300.0)), ZERO, (f32::INFINITY, 80.0)), (80.0, 80.0)));
    }

    #[test]
    fn a_box_that_is_too_narrow_grows_in_both_directions() {
        // `width: 20px; min-width: 150px`
        assert!(close(constrain((100.0, 100.0), (Some(20.0), None), (150.0, 0.0), NONE), (150.0, 150.0)));
    }

    #[test]
    fn both_sides_specified_are_limited_on_their_own() {
        assert!(close(
            constrain((100.0, 100.0), (Some(300.0), Some(50.0)), ZERO, (200.0, f32::INFINITY)),
            (200.0, 50.0)
        ));
    }

    #[test]
    fn conflicting_limits_follow_the_table() {
        // too wide and too tall: the one that is further off wins
        // 400x200 with max 200x150: 200/400 = 0.5 <= 150/200 = 0.75: (200, 100)
        assert!(close(constrain((400.0, 200.0), (None, None), ZERO, (200.0, 150.0)), (200.0, 100.0)));
        // too narrow and too short: 100x50 with min 300x80: 3 > 1.6: (300, 150)
        assert!(close(constrain((100.0, 50.0), (None, None), (300.0, 80.0), NONE), (300.0, 150.0)));
        // too narrow and too tall
        assert!(close(constrain((100.0, 400.0), (None, None), (200.0, 0.0), (f32::INFINITY, 300.0)), (200.0, 300.0)));
    }

    #[test]
    fn a_maximum_below_the_minimum_is_the_minimum() {
        assert!(close(constrain((100.0, 100.0), (None, None), (150.0, 0.0), (50.0, f32::INFINITY)), (150.0, 150.0)));
    }

    #[test]
    fn an_image_without_a_size_has_no_ratio_to_keep() {
        assert!(close(constrain((0.0, 0.0), (Some(30.0), None), ZERO, NONE), (30.0, 0.0)));
    }
}
