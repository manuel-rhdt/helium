//! Bidirectional text (https://www.w3.org/TR/css-writing-modes-3/#text-direction).
//!
//! The Unicode bidirectional algorithm (UAX #9) is run on the content of every paragraph (inline
//! formatting context) by `FindPossibleLineBreaks`:
//!
//! * The text of all text runs is joined; every other inline element (e.g. an `inline-block`)
//!   takes part as U+FFFC OBJECT REPLACEMENT CHARACTER. Inline elements with `unicode-bidi`
//!   contribute bidi control characters, which are zero-width text runs of their own (see
//!   `EntityConstructor::build_inline_formatting_context`).
//! * The resulting embedding levels are stored as runs per element
//!   (`InlineFormattingContext::bidi_runs`). Paragraphs that are completely left-to-right have
//!   no runs, and everything else works as without bidi support.
//! * Every run is shaped in its direction. Shaping returns glyphs in logical order, so line
//!   breaking, which works on the logical order, is not affected.
//! * When a line is drawn, its runs are reordered (rule L2, `visual_order`) and right-to-left
//!   runs are drawn with their glyphs reversed.
//!
//! Not supported: the line-based rule L1 except for the whitespace at the end of a line, which
//! is not drawn; mirrored glyphs and positioning of marks rely on the font (via harfbuzz).

use crate::layout::components::TextRun;
use specs::Entity;
use specs::storage::GenericReadStorage;
use unicode_bidi::{BidiInfo, Level};

/// The embedding levels of the content of a paragraph.
#[derive(Debug, Default)]
pub struct ResolvedLevels {
    /// Whether the (first) paragraph is right-to-left.
    pub base_rtl: bool,
    /// For every element: runs `(start, end, level)` of bytes of its text (`0..0` for elements
    /// that are not text runs). `None` if all of the content is left-to-right.
    pub element_runs: Option<Vec<Vec<(usize, usize, u8)>>>,
}

/// Runs the bidi algorithm on the content of a paragraph made of `elements`. `base_rtl` is the
/// base direction, or `None` to determine it from the text (`unicode-bidi: plaintext`).
pub fn resolve_levels(
    elements: &[Entity],
    text_runs: &impl GenericReadStorage<Component = TextRun>,
    base_rtl: Option<bool>,
) -> ResolvedLevels {
    let mut text = String::new();
    let mut starts = Vec::with_capacity(elements.len() + 1);
    for &element in elements {
        starts.push(text.len());
        match text_runs.get(element) {
            Some(run) => text.push_str(&run.text),
            None => text.push('\u{FFFC}'),
        }
    }
    starts.push(text.len());

    let default_level = base_rtl.map(|rtl| if rtl { Level::rtl() } else { Level::ltr() });
    let info = BidiInfo::new(&text, default_level);
    let base_rtl = info
        .paragraphs
        .first()
        .map_or(base_rtl == Some(true), |paragraph| paragraph.level.is_rtl());
    if !info.has_rtl() {
        return ResolvedLevels {
            base_rtl,
            element_runs: None,
        };
    }

    let element_runs = elements
        .iter()
        .enumerate()
        .map(|(index, &element)| {
            let (start, end) = (starts[index], starts[index + 1]);
            if text_runs.get(element).is_none() {
                return vec![(0, 0, info.levels[start].number())];
            }
            let mut runs: Vec<(usize, usize, u8)> = Vec::new();
            for (offset, _) in text[start..end].char_indices() {
                let level = info.levels[start + offset].number();
                match runs.last_mut() {
                    Some(run) if run.2 == level => run.1 = offset,
                    _ => runs.push((offset, offset, level)),
                }
            }
            // the runs end after their last character
            let mut previous_start = end - start;
            for run in runs.iter_mut().rev() {
                run.1 = previous_start;
                previous_start = run.0;
            }
            runs
        })
        .collect();
    ResolvedLevels {
        base_rtl,
        element_runs: Some(element_runs),
    }
}

/// Rule L2 of the bidi algorithm: the order in which items with the given embedding levels
/// (in logical order) are displayed from left to right, as indices into `levels`.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let (Some(&max), Some(&min)) = (levels.iter().max(), levels.iter().min()) else {
        return order;
    };
    // From the highest level to the lowest odd level, reverse every sequence of items at that
    // level or higher.
    let lowest_odd = min | 1;
    let mut level = max;
    while level >= lowest_odd {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_to_right_has_no_reordering() {
        assert_eq!(visual_order(&[0, 0, 0]), vec![0, 1, 2]);
        assert_eq!(visual_order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn right_to_left_items_are_reversed() {
        // LTR paragraph with an RTL phrase in the middle
        assert_eq!(visual_order(&[0, 1, 1, 0]), vec![0, 2, 1, 3]);
        // RTL paragraph with an LTR phrase in the middle: the phrase keeps its order
        assert_eq!(visual_order(&[1, 2, 2, 1]), vec![3, 1, 2, 0]);
        // numbers (level 2) inside RTL text inside an LTR paragraph
        assert_eq!(visual_order(&[0, 1, 2, 1, 0]), vec![0, 3, 2, 1, 4]);
    }

    fn runs_of(texts: &[&str], base_rtl: Option<bool>) -> ResolvedLevels {
        use specs::{Builder, World, WorldExt};
        let mut world = World::new();
        world.register::<TextRun>();
        let elements: Vec<Entity> = texts
            .iter()
            .map(|text| {
                world
                    .create_entity()
                    .with(TextRun {
                        text: text.to_string(),
                        ..Default::default()
                    })
                    .build()
            })
            .collect();
        let storage = world.read_storage::<TextRun>();
        resolve_levels(&elements, &storage, base_rtl)
    }

    #[test]
    fn levels_are_split_into_runs_per_element() {
        let resolved = runs_of(&["abc ", "שלום world"], Some(false));
        assert!(!resolved.base_rtl);
        let runs = resolved.element_runs.unwrap();
        assert_eq!(runs[0], vec![(0, 4, 0)]);
        // "שלום" is 8 bytes; the space after it is between R and L and gets the base level
        assert_eq!(runs[1], vec![(0, 8, 1), (8, 14, 0)]);
    }

    #[test]
    fn left_to_right_paragraphs_have_no_runs() {
        let resolved = runs_of(&["just ", "English"], Some(false));
        assert!(resolved.element_runs.is_none());
    }

    #[test]
    fn plaintext_takes_the_direction_from_the_text() {
        assert!(runs_of(&["שלום world"], None).base_rtl);
        assert!(!runs_of(&["world שלום"], None).base_rtl);
        // an explicit direction wins
        assert!(runs_of(&["world"], Some(true)).base_rtl);
    }
}
