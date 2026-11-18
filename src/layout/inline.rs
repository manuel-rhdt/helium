use super::components::{self, EntityHrefs, Hyperlink, Hyperlinks, Paragraph, PlacedColorGlyph, TextOperator, take_color_glyphs};
use crate::tagging::{ContentTag, StructNodeId, StructureTree};
use super::{
    FontMap,
    text::{Font, font::GlyphIndex},
};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px, PaintColor, TextAlign};
use crate::style::properties::specified::{Direction, UnicodeBidi};

use crate::display_list::Rect;
use crate::layout::text::{
    hyphen_character, is_invisible_formatting, shape_directed_into, shape_hyphenated_into, shape_with_levels, with_shaped,
    with_shaped_hyphenated,
};

use components::{GlyphRange, GraphicsOp, IntrinsicSize, KernedGlyph, PieceKind, Position, TextOps};
use hyphenation::{Hyphenator, Language, Load, Standard};
use lemon_pdf::font::FontUnit;
use specs::storage::GenericReadStorage;
use specs::{Component, DenseVecStorage, Entity};
use specs::{Entities, Join, Read, ReadStorage, System, Write, WriteStorage};

use helium_paginate::paragraph::LineStart;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Copy, Clone)]
pub struct Line {
    pub height: Px,
    pub baseline: Px,
    pub from_break: usize,
    pub to_break: usize,
    pub entity: Entity,
}

/// A line computed by [`InlineFormattingContext::compute_lines`], which has no entity yet.
#[derive(Debug, Copy, Clone)]
pub struct MeasuredLine {
    pub height: Px,
    pub baseline: Px,
    pub from_break: usize,
    pub to_break: usize,
    /// How the line fits (`helium_paginate::paragraph::LineStart::fitness`): the breaking of the
    /// rest of the paragraph after it counts lines that are far apart in it.
    pub fitness: u8,
    /// The demerits of the line as the breaker counts them (`ParagraphLines::demerits`).
    pub demerits: f64,
    /// The badness of the line (`ParagraphLines::badness`).
    pub badness: f64,
}

#[derive(Debug, Clone)]
pub struct InlineFormattingContext {
    pub text_align: crate::style::properties::specified::TextAlign,
    pub elements: Vec<Entity>,
    /// Absolutely positioned boxes whose static position is in this context. They are not part
    /// of `elements` because they do not take part in inline layout.
    pub absolutes: Vec<Entity>,
    pub break_points: Vec<BreakPoint>,
    pub lines: Vec<Line>,
    /// The `inline-block` boxes in `elements` and the advance at which they start. Before the
    /// boxes are measured they have no width.
    pub atom_offsets: Vec<(Entity, Px)>,
    /// The `direction` of the block: the base direction of the paragraph.
    pub direction: Direction,
    /// The flow of the lines: `writing-mode`.
    pub flow: super::writing_mode::Flow,
    /// Whether the base direction is found from the text (`unicode-bidi: plaintext`) instead of
    /// `direction`.
    pub plaintext: bool,
    /// Whether the (first) paragraph is right-to-left, set by `FindPossibleLineBreaks`.
    pub base_rtl: bool,
    /// The runs of the paragraph with the same bidi embedding level in logical order, set by
    /// `FindPossibleLineBreaks`. Empty if all of the text is left-to-right.
    pub bidi_runs: Vec<BidiRun>,
    /// The ascent and descent of the strut: a zero-width inline box with the block container's
    /// font and line height. Every line box starts with it (CSS 2.1 §10.8.1), so an empty line
    /// is exactly one line of the block's font tall.
    pub strut: (Px, Px),
}

/// A part of the content of a paragraph with the same bidi embedding level, inside of one
/// element of an inline formatting context.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BidiRun {
    /// The index of the element in `InlineFormattingContext::elements`.
    pub element: usize,
    /// The range of bytes in the text of the element (`0..0` for other elements).
    pub start: usize,
    pub end: usize,
    /// The embedding level; odd levels are right-to-left.
    pub level: u8,
    /// The advance (like the advances of the break points) at the start and end of the run.
    pub advance_start: Px,
    pub advance_end: Px,
}

/// A part of a line with one bidi embedding level: a range of the text of a text run or another
/// element (with an empty range).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct LinePiece {
    /// The index of the element in `InlineFormattingContext::elements`.
    pub element: usize,
    pub start: usize,
    pub end: usize,
    pub level: u8,
}

impl LinePiece {
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

impl BidiRun {
    pub fn is_rtl(&self) -> bool {
        self.level % 2 == 1
    }
}

impl InlineFormattingContext {
    pub fn new(
        elements: Vec<Entity>,
        absolutes: Vec<Entity>,
        style: &ComputedValues,
        strut: (Px, Px),
    ) -> Self {
        InlineFormattingContext {
            text_align: style.text.text_align,
            elements,
            absolutes,
            break_points: vec![],
            lines: vec![],
            atom_offsets: vec![],
            direction: style.text.direction,
            flow: style.text.writing_mode.into(),
            plaintext: style.box_.unicode_bidi == UnicodeBidi::Plaintext,
            base_rtl: style.text.direction == Direction::Rtl,
            bidi_runs: vec![],
            strut,
        }
    }

    /// The extra space after every space on a justified line (all but the last line of a
    /// paragraph and the lines that end at a forced break, with `text-align: justify`), given
    /// the width of the line box. A line that is not stretched but too wide, because the line
    /// breaking let its spaces shrink to make it fit, shrinks its spaces too (a negative
    /// extra space).
    pub fn word_spacing(&self, line_num: usize, line_width: Px) -> Px {
        let line = &self.lines[line_num];
        let num_spaces = self.space_count(line.from_break, line.to_break);
        if self.text_align != TextAlign::Justify || num_spaces == 0 {
            return Px(0.0);
        }
        let is_last_line =
            line_num == self.lines.len() - 1 || self.break_points[line.to_break].forced;
        let slack = line_width - self.content_width(line.from_break, line.to_break);
        if is_last_line && slack >= Px(0.0) {
            Px(0.0)
        } else {
            slack / num_spaces as f32
        }
    }

    /// Where the content of a line starts in its line box. The line box of a justified line
    /// is as wide as the paragraph; a line that is not stretched (the last one) starts at the
    /// right in a right-to-left paragraph.
    pub fn line_start_offset(&self, line_num: usize, line_width: Px, word_spacing: Px) -> Px {
        let line = &self.lines[line_num];
        if self.base_rtl && self.text_align == TextAlign::Justify && word_spacing == Px(0.0) {
            (line_width - self.content_width(line.from_break, line.to_break)).max(Px(0.0))
        } else {
            Px(0.0)
        }
    }

    /// The pieces of a line in logical order: the parts of its elements, split where the bidi
    /// embedding level changes. The whitespace at the end of the line is not part of them.
    pub fn line_pieces(
        &self,
        line: &Line,
        text_runs: &impl GenericReadStorage<Component = components::TextRun>,
    ) -> Vec<LinePiece> {
        let from_break = self.break_points[line.from_break];
        let to_break = self.break_points[line.to_break];
        let Some((first, last)) = self.element_span(from_break, to_break) else {
            return vec![];
        };
        let mut pieces = Vec::new();
        for element in first..=last {
            let entity = self.elements[element];
            let runs_start = self.bidi_runs.partition_point(|run| run.element < element);
            let runs = self.bidi_runs[runs_start..]
                .iter()
                .take_while(|run| run.element == element);
            match text_runs.get(entity) {
                Some(text_run) => {
                    let start = if entity == from_break.entity {
                        from_break.char_index
                    } else {
                        0
                    };
                    let mut end = if entity == to_break.entity {
                        to_break.char_index
                    } else {
                        text_run.text.len()
                    };
                    if entity == to_break.entity && to_break.space && start < end {
                        end = start + text_run.text[start..end].trim_end().len();
                    }
                    if start >= end {
                        continue;
                    }
                    if self.bidi_runs.is_empty() {
                        pieces.push(LinePiece {
                            element,
                            start,
                            end,
                            level: 0,
                        });
                        continue;
                    }
                    for run in runs {
                        let (start, end) = (run.start.max(start), run.end.min(end));
                        if start < end {
                            pieces.push(LinePiece {
                                element,
                                start,
                                end,
                                level: run.level,
                            });
                        }
                    }
                }
                None => {
                    let level = runs.map(|run| run.level).next().unwrap_or(0);
                    pieces.push(LinePiece {
                        element,
                        start: 0,
                        end: 0,
                        level,
                    });
                }
            }
        }
        pieces
    }

    /// The x offsets from the left edge of the line box of the `inline-block` boxes on a line of
    /// a paragraph with bidi runs, computed from the advances of the runs.
    fn visual_atom_offsets(&self, line_num: usize, line_width: Px) -> Vec<(Entity, Px)> {
        let line = &self.lines[line_num];
        let from = self.break_points[line.from_break];
        let to = self.break_points[line.to_break];
        let word_spacing = self.word_spacing(line_num, line_width);
        let spaces = &self.break_points[line.from_break + 1..line.to_break];

        // the parts of the runs on the line in logical order as (run, width)
        let mut parts: Vec<(&BidiRun, Px)> = self
            .bidi_runs
            .iter()
            .filter_map(|run| {
                let start = run.advance_start.max(from.advance_after);
                let end = run.advance_end.min(to.advance_before);
                if end <= start {
                    return None;
                }
                let num_spaces = spaces
                    .iter()
                    .filter(|bp| {
                        (bp.space || bp.inter_letter)
                            && bp.advance_before >= start
                            && bp.advance_before < end
                    })
                    .count();
                Some((run, end - start + word_spacing * num_spaces as f32))
            })
            .collect();
        if let Some(last) = parts.last_mut() {
            last.1 += to.hyphen_width;
        }

        let mut x = self.line_start_offset(line_num, line_width, word_spacing);
        let mut offsets = Vec::new();
        let levels: Vec<u8> = parts.iter().map(|(run, _)| run.level).collect();
        for index in super::bidi::visual_order(&levels) {
            let (run, width) = parts[index];
            if run.start == run.end {
                offsets.push((self.elements[run.element], x));
            }
            x += width;
        }
        offsets
    }

    /// `text-align` with `start` and `end` resolved for the direction of the paragraph.
    pub fn resolved_text_align(&self) -> TextAlign {
        self.text_align.resolve(self.base_rtl)
    }

    /// The `text-indent` of the first line moves it away from the start edge: the left edge in
    /// left-to-right and the right edge in right-to-left paragraphs. Returns the offset of the
    /// first line box from the left edge.
    pub fn indent_offset(&self, indent: Px) -> Px {
        if self.base_rtl { Px(0.0) } else { indent }
    }

    /// Breaks the paragraph into lines using adjustment-ratio badness and breakpoint demerits,
    /// creates one entity per line and stores the lines in the context.
    pub fn break_lines(
        &mut self,
        line_width: Px,
        indent: Px,
        entities: &Entities,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
        aligns: &ReadStorage<components::VerticalAlignment>,
    ) -> Result<(), String> {
        let lines = self.compute_lines(line_width, indent, text_runs, intrinsic_sizes, aligns)?;
        self.lines = lines
            .into_iter()
            .map(|line| Line {
                from_break: line.from_break,
                to_break: line.to_break,
                height: line.height,
                baseline: line.baseline,
                entity: entities.create(),
            })
            .collect();
        Ok(())
    }

    /// Adds lines that were computed for the paragraph, one entity for each. Returns the number
    /// of the first of them.
    pub fn push_lines(&mut self, lines: Vec<MeasuredLine>, entities: &Entities) -> usize {
        let first = self.lines.len();
        self.lines.extend(lines.into_iter().map(|line| Line {
            from_break: line.from_break,
            to_break: line.to_break,
            height: line.height,
            baseline: line.baseline,
            entity: entities.create(),
        }));
        first
    }

    /// Breaks the paragraph into lines without storing them or creating entities for them, so
    /// that it can be used to measure the paragraph for a given line width.
    pub fn compute_lines(
        &self,
        line_width: Px,
        indent: Px,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
        aligns: &ReadStorage<components::VerticalAlignment>,
    ) -> Result<Vec<MeasuredLine>, String> {
        self.compute_lines_from(0, LineStart::DECENT, None, line_width, indent, text_runs, intrinsic_sizes, aligns)
    }

    /// Breaks the part of the paragraph that follows the break point `from` into lines. The first
    /// line is indented by `indent` only if `from` is the start of the paragraph. `fitness` is that
    /// of the line that ends at `from`, which the breaker counts like a line in front of the first
    /// one in a paragraph (see [`MeasuredLine::fitness`]); it is not used at the start of the
    /// paragraph. If `until` is a break point, the part of the paragraph from `from` to it is
    /// broken: its last line ends there, and is a line like the others and not the last line
    /// of the paragraph.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_lines_from(
        &self,
        from: usize,
        fitness: u8,
        until: Option<usize>,
        line_width: Px,
        indent: Px,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
        aligns: &ReadStorage<components::VerticalAlignment>,
    ) -> Result<Vec<MeasuredLine>, String> {
        let mut variants =
            self.compute_variants_from(from, fitness, until, 0, line_width, indent, text_runs, intrinsic_sizes, aligns)?;
        Ok(if variants.is_empty() { vec![] } else { variants.swap_remove(0) })
    }

    /// The ways to break the part of the paragraph that [`compute_lines_from`] breaks: the optimum
    /// first, and then, if `extra` is not 0, the breakings with the least demerits for the
    /// numbers of lines that are up to `extra` fewer and more than the optimum has (see
    /// `helium_paginate::paragraph::compute_paragraph_variants`).
    ///
    /// [`compute_lines_from`]: Self::compute_lines_from
    #[allow(clippy::too_many_arguments)]
    pub fn compute_variants_from(
        &self,
        from: usize,
        fitness: u8,
        until: Option<usize>,
        extra: u32,
        line_width: Px,
        indent: Px,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
        aligns: &ReadStorage<components::VerticalAlignment>,
    ) -> Result<Vec<Vec<MeasuredLine>>, String> {
        let breakpoints = &self.break_points;
        if breakpoints.is_empty() || from >= breakpoints.len() {
            return Ok(vec![vec![]]);
        }
        // (a cut that is not inside the paragraph is the end of it)
        let until = until.filter(|&to| to + 1 < breakpoints.len());
        if until.is_some_and(|to| to <= from) {
            return Ok(vec![vec![]]);
        }
        if breakpoints.len() < 2 {
            return Err(format!(
                "inline context needs at least 2 break points, found {}",
                breakpoints.len()
            ));
        }

        let opportunities: Vec<_> = breakpoints
            .iter()
            .map(|breakpoint| helium_paginate::paragraph::BreakOpportunity {
                forced: breakpoint.forced,
                hyphen: breakpoint.hyphen,
                penalty: breakpoint.penalty,
            })
            .collect();
        let mut glue_prefix = Vec::with_capacity(breakpoints.len() + 1);
        glue_prefix.push(helium_paginate::paragraph::GlueTotals::default());
        for breakpoint in breakpoints {
            let previous = *glue_prefix.last().unwrap();
            glue_prefix.push(helium_paginate::paragraph::GlueTotals {
                stretch: previous.stretch + f64::from(breakpoint.glue_stretch.0),
                shrink: previous.shrink + f64::from(breakpoint.glue_shrink.0),
            });
        }
        let justified = self.text_align == TextAlign::Justify;
        let metrics = |from: usize, to: usize| {
            let natural_width = breakpoints[to].advance_before
                - breakpoints[from].advance_after
                + breakpoints[to].hyphen_width;
            let start_glue = glue_prefix[from + 1];
            let end_glue = glue_prefix[to];
            // Justified text stretches and shrinks its spaces. Ragged text has rigid spaces
            // and, like TeX's \raggedright, a finite stretch of 2em at the end of every line, so
            // lines are balanced by the same algorithm without ever being squeezed.
            let glue = if justified {
                helium_paginate::paragraph::GlueTotals {
                    stretch: end_glue.stretch - start_glue.stretch,
                    shrink: end_glue.shrink - start_glue.shrink,
                }
            } else {
                let font_size = text_runs
                    .get(breakpoints[to].entity)
                    .map_or(16.0, |run| run.face.font_size.0);
                helium_paginate::paragraph::GlueTotals {
                    stretch: f64::from(2.0 * font_size),
                    shrink: 0.0,
                }
            };
            Some((f64::from(natural_width.0), glue))
        };
        let width_of = |at: usize| {
            f64::from((line_width - if at + from == 0 { indent } else { Px(0.0) }).0)
        };
        // the paragraph from the break point `from` on, which is the start of its first line; it
        // follows the line that ends there, which is hyphenated if the break is
        let start = if from == 0 {
            LineStart::PARAGRAPH
        } else {
            LineStart { fitness, hyphen: breakpoints[from].hyphen }
        };
        let end = until.map_or(opportunities.len(), |to| to + 1);
        let variants = helium_paginate::paragraph::compute_paragraph_variants(
            &opportunities[from..end],
            |start, end| metrics(from + start, from + end),
            width_of,
            start,
            until.is_none(),
            extra,
        );

        variants
            .iter()
            .map(|broken| {
                broken
                    .breaks
                    .windows(2)
                    .zip(
                        broken
                            .fitness
                            .iter()
                            .copied()
                            .zip(broken.demerits.iter().copied().zip(broken.badness.iter().copied())),
                    )
                    .map(|(window, (fitness, (demerits, badness)))| {
                        let (window_from, window_to) = (from + window[0], from + window[1]);
                        let (baseline, height) =
                            self.line_height(window_from, window_to, text_runs, intrinsic_sizes, aligns);
                        // a line can have no height (see `line_height`)
                        Ok(MeasuredLine {
                            from_break: window_from,
                            to_break: window_to,
                            height,
                            baseline,
                            fitness,
                            demerits,
                            badness,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .collect()
    }

    /// The min-content and max-content width of the paragraph: the widest piece that cannot be
    /// broken, and the widest line that only breaks where a break is forced. The first line is
    /// indented by `indent`.
    pub fn intrinsic_widths(&self, indent: Px) -> (Px, Px) {
        let breaks = &self.break_points;
        if breaks.len() < 2 {
            return (Px(0.0), Px(0.0));
        }
        let mut min = Px(0.0);
        let mut max = Px(0.0);
        let mut line_start = 0;
        for i in 1..breaks.len() {
            let piece_indent = if i == 1 { indent } else { Px(0.0) };
            min = min.max(self.content_width(i - 1, i) + piece_indent);
            if breaks[i].forced || i == breaks.len() - 1 {
                let line_indent = if line_start == 0 { indent } else { Px(0.0) };
                max = max.max(self.content_width(line_start, i) + line_indent);
                line_start = i;
            }
        }
        (min, max)
    }

    /// The width of the content between two break points.
    pub fn content_width(&self, from_break: usize, to_break: usize) -> Px {
        self.break_points[to_break].advance_before - self.break_points[from_break].advance_after
            + self.break_points[to_break].hyphen_width
    }

    fn space_count(&self, from_break: usize, to_break: usize) -> usize {
        self.break_points[from_break + 1..to_break]
            .iter()
            .filter(|break_point| break_point.space || break_point.inter_letter)
            .count()
    }

    /// The `inter_letter` break points inside `piece` of a line, as byte offsets into the
    /// text of the piece.
    fn inter_letter_offsets(&self, line: &Line, piece: &LinePiece) -> Vec<usize> {
        let entity = self.elements[piece.element];
        self.break_points[line.from_break + 1..line.to_break]
            .iter()
            .filter(|point| {
                point.inter_letter
                    && point.entity == entity
                    && piece.start < point.char_index
                    && point.char_index < piece.end
            })
            .map(|point| point.char_index - piece.start)
            .collect()
    }

    /// The indices into `elements` of the first and the last element that have a part on the
    /// line between two break points, or `None` if there is none. A box is not on the line if
    /// the line starts behind it or ends in front of it.
    fn element_span(&self, from: BreakPoint, to: BreakPoint) -> Option<(usize, usize)> {
        let position = |entity| self.elements.iter().position(|&e| e == entity);
        let is_atom = |entity| self.atom_offsets.iter().any(|&(atom, _)| atom == entity);
        let mut start = position(from.entity)?;
        let mut end = position(to.entity)?;
        if is_atom(from.entity) && from.char_index >= ATOM_BREAK_AFTER {
            start += 1;
        }
        if is_atom(to.entity) && to.char_index == ATOM_BREAK_BEFORE {
            end = end.checked_sub(1)?;
        }
        (start <= end).then_some((start, end))
    }

    pub fn line_height(
        &self,
        from_index: usize,
        to_index: usize,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
        aligns: &ReadStorage<components::VerticalAlignment>,
    ) -> (Px, Px) {
        let from = self.break_points[from_index];
        let to = self.break_points[to_index];

        let span = self
            .element_span(from, to)
            .map_or(&self.elements[0..0], |(start, end)| &self.elements[start..=end]);

        let (mut max_ascent, mut max_descent) = self.strut;
        // the height that the boxes that are aligned with the top and the bottom of the line need
        let (mut need_top, mut need_bottom) = (Px(0.0), Px(0.0));
        for &ent in span {
            let Some(own) = own_extent(ent, text_runs, intrinsic_sizes) else {
                log::error!("cannot layout element (entity {}) in line (every inline element must have an intrinsic size)", ent.id());
                continue;
            };
            let placement = aligns
                .get(ent)
                .map_or(components::LinePlacement::Raise(Px(0.0)), |align| align.place(own));
            match placement {
                components::LinePlacement::Raise(raise) => {
                    max_ascent = max_ascent.max(own.0 + raise);
                    max_descent = max_descent.max(own.1 - raise);
                }
                components::LinePlacement::Top { ascent, descent, .. } => {
                    need_top = need_top.max(ascent + descent)
                }
                components::LinePlacement::Bottom { ascent, descent, .. } => {
                    need_bottom = need_bottom.max(ascent + descent)
                }
            }
        }
        // the line box is as high as the boxes that are aligned with its edges need; what is
        // missing is added below the baseline for the ones at the top and above it for the ones at
        // the bottom
        let need = need_top.max(need_bottom);
        if need > max_ascent + max_descent {
            let missing = need - (max_ascent + max_descent);
            if need_top >= need_bottom {
                max_descent += missing;
            } else {
                max_ascent += missing;
            }
        }

        // The line has no height if the font size and the line height of everything on it are
        // zero (`font-size: 0` is used for spacers). That is what browsers do too.
        (max_ascent, max_ascent + max_descent)
    }
}

impl InlineFormattingContext {
    /// Takes the widths of the `inline-block` boxes into account, which were not known when the
    /// break points were found. Must be called once with the widths of all boxes of
    /// `atom_offsets`.
    pub fn apply_atom_widths(&mut self, widths: &HashMap<Entity, Px>) {
        let position = |entity: Entity| self.elements.iter().position(|&e| e == entity);
        let atoms: Vec<(usize, Px)> = self
            .atom_offsets
            .iter()
            .filter_map(|&(entity, _)| Some((position(entity)?, widths.get(&entity).copied()?)))
            .collect();
        let shifts: Vec<Px> = self
            .break_points
            .iter()
            .enumerate()
            .map(|(index, bp)| {
                if index == 0 {
                    return Px(0.0);
                }
                let element = position(bp.entity).unwrap_or(0);
                // a break in front of a box does not include the box
                atoms
                    .iter()
                    .filter(|&&(atom, _)| {
                        atom < element || (atom == element && bp.char_index > ATOM_BREAK_BEFORE)
                    })
                    .fold(Px(0.0), |sum, &(_, width)| sum + width)
            })
            .collect();
        for (bp, shift) in self.break_points.iter_mut().zip(shifts) {
            bp.advance_before += shift;
            bp.advance_after += shift;
        }
        // The runs are shifted by the boxes in front of them; the run of a box itself gets its
        // width.
        let width_of_atoms = |include: &dyn Fn(usize) -> bool| {
            atoms
                .iter()
                .filter(|&&(atom, _)| include(atom))
                .fold(Px(0.0), |sum, &(_, width)| sum + width)
        };
        for run in &mut self.bidi_runs {
            let element = run.element;
            run.advance_start += width_of_atoms(&|atom| atom < element);
            run.advance_end += width_of_atoms(&|atom| atom <= element);
        }
        let mut shift = Px(0.0);
        for (entity, offset) in &mut self.atom_offsets {
            *offset += shift;
            shift += widths.get(entity).copied().unwrap_or_default();
        }
    }
}

impl Component for InlineFormattingContext {
    type Storage = DenseVecStorage<Self>;
}

pub struct FindPossibleLineBreaks;

/// The `char_index` of the break point in front of an inline-level box.
const ATOM_BREAK_BEFORE: usize = 0;
/// The `char_index` of the break point behind an inline-level box.
const ATOM_BREAK_AFTER: usize = 1;

#[derive(Debug, Copy, Clone)]
pub struct BreakPoint {
    pub entity: Entity,
    pub char_index: usize,
    pub advance_before: Px,
    pub advance_after: Px,
    pub forced: bool,
    pub space: bool,
    /// A break between two letters (without a space) where a justified line is stretched, like
    /// between Chinese characters. The extra space goes in front of the character at
    /// `char_index`.
    pub inter_letter: bool,
    pub hyphen: bool,
    pub hyphen_width: Px,
    pub glue_stretch: Px,
    pub glue_shrink: Px,
    pub penalty: i32,
}

// TODO: test this
impl<'a> System<'a> for FindPossibleLineBreaks {
    type SystemData = (
        WriteStorage<'a, InlineFormattingContext>,
        WriteStorage<'a, components::TextRun>,
        ReadStorage<'a, components::IntrinsicSize>,
        ReadStorage<'a, ComputedValues>,
        Read<'a, FontMap>,
    );

    fn run(
        &mut self,
        (mut inline_formatting_contexts, mut text_runs, intrinsic_sizes, computed_values, font_map): Self::SystemData,
    ) {
        let mut dictionaries = HyphenationDictionaries::default();

        for InlineFormattingContext {
            elements,
            break_points,
            atom_offsets,
            direction,
            plaintext,
            base_rtl,
            bidi_runs,
            ..
        } in (&mut inline_formatting_contexts).join()
        {
            // an empty block (e.g. `<div></div>`) has an inline formatting context without content
            if elements.is_empty() {
                continue;
            }

            let levels = super::bidi::resolve_levels(
                elements,
                &text_runs,
                (!*plaintext).then_some(*direction == Direction::Rtl),
            );
            *base_rtl = levels.base_rtl;
            let element_runs = levels.element_runs;
            // the runs of an element, or none if the paragraph is left-to-right
            let runs_of = |index: usize| -> &[(usize, usize, u8)] {
                element_runs.as_ref().map_or(&[], |runs| &runs[index])
            };

            // insert first breakpoint at beginning
            break_points.push(BreakPoint {
                entity: elements[0],
                char_index: 0,
                advance_before: Px(0.0),
                advance_after: Px(0.0),
                forced: false,
                space: false,
                inter_letter: false,
                hyphen: false,
                hyphen_width: Px(0.0),
                glue_stretch: Px(0.0),
                glue_shrink: Px(0.0),
                penalty: 0,
            });

            // whether an element has content (text, or an inline-level box) behind it
            let mut content_after = vec![false; elements.len()];
            for index in (0..elements.len().saturating_sub(1)).rev() {
                let later = elements[index + 1];
                content_after[index] = content_after[index + 1]
                    || text_runs.get(later).is_some_and(|run| !run.text.trim().is_empty())
                    || intrinsic_sizes.get(later).is_some()
                    || computed_values
                        .get(later)
                        .is_some_and(|style| style.box_.display.is_atomic_inline());
            }

            let mut advance = Pen::default();

            for (index, &entity) in elements.iter().enumerate() {
                let element_start = advance;
                if let Some(text_run) = text_runs.get_mut(entity) {
                    let font = &font_map[&text_run.face.font_name];
                    let levels = runs_of(index);
                    // the glyphs of the run as `(cluster, width)`, for the advances of the bidi
                    // runs
                    let mut glyph_widths: Vec<(usize, Px)> = Vec::new();
                    let white_space = computed_values.get(entity).map(|style| style.text.white_space).unwrap_or_default();
                    if white_space.preserves_newlines() {
                        // The text is set in segments between its new lines, which are line
                        // breaks. Where the lines wrap (`pre-wrap` and `pre-line`) they break
                        // after the spaces. Not here: hyphenation, and the breaks between the
                        // letters of CJK text.
                        let wraps = white_space.wraps();
                        let mut segments = Vec::new();
                        let mut segment_start = 0;
                        for (newline_index, _) in text_run.text.match_indices('\n') {
                            segments.push((segment_start, newline_index));
                            segment_start = newline_index + 1;
                        }
                        segments.push((segment_start, text_run.text.len()));
                        for (number, &(segment_start, segment_end)) in segments.iter().enumerate() {
                            let last = number + 1 == segments.len();
                            // the spaces that the glyphs are in: where they start, and how
                            // much they stretch and shrink
                            let mut spaces: Option<(Px, Px, Px)> = None;
                            let mut push_spaces = |spaces: (Px, Px, Px), end: usize, advance: Pen| {
                                break_points.push(BreakPoint {
                                    entity,
                                    char_index: end,
                                    advance_before: spaces.0,
                                    advance_after: advance.px(),
                                    forced: false,
                                    space: true,
                                    inter_letter: false,
                                    hyphen: false,
                                    hyphen_width: Px(0.0),
                                    glue_stretch: spaces.1,
                                    glue_shrink: spaces.2,
                                    penalty: 0,
                                });
                            };
                            for (position, info) in shape_with_levels(
                                &text_run.text[segment_start..segment_end],
                                segment_start,
                                levels,
                                font,
                                &text_run.face.fallback,
                            ) {
                                let cluster = segment_start + info.cluster as usize;
                                let width = font.scale_to_px(position.x_advance, text_run.face.font_size);
                                let is_space = wraps && text_run.text[cluster..].starts_with(char::is_whitespace);
                                if !is_space && let Some(run) = spaces.take() {
                                    push_spaces(run, cluster, advance);
                                }
                                if is_space {
                                    let size = position.x_advance.0 as f32 * text_run.face.font_size.0;
                                    let run = spaces.get_or_insert((advance.px(), Px(0.0), Px(0.0)));
                                    run.1 += Px(size / 2000.0);
                                    run.2 += Px(size / 3000.0);
                                }
                                glyph_widths.push((cluster, width));
                                advance += width;
                            }
                            // spaces at the end of a line that a new line ends hang there: no
                            // break is needed, and one would make an empty line
                            if let Some(run) = spaces.take()
                                && last
                            {
                                push_spaces(run, segment_end, advance);
                            }
                            if !last {
                                break_points.push(BreakPoint {
                                    entity,
                                    char_index: segment_end + 1,
                                    advance_before: advance.px(),
                                    advance_after: advance.px(),
                                    forced: true,
                                    space: false,
                                    inter_letter: false,
                                    hyphen: false,
                                    hyphen_width: Px(0.0),
                                    glue_stretch: Px(0.0),
                                    glue_shrink: Px(0.0),
                                    penalty: -10_000,
                                });
                            }
                        }
                        push_bidi_runs(bidi_runs, index, levels, &glyph_widths, element_start.px());
                        continue;
                    }
                    let shaped = shape_with_levels(&text_run.text, 0, levels, font, &text_run.face.fallback);
                    if !levels.is_empty() {
                        glyph_widths = shaped
                            .iter()
                            .map(|(position, info)| {
                                (
                                    info.cluster as usize,
                                    font.scale_to_px(position.x_advance, text_run.face.font_size),
                                )
                            })
                            .collect();
                    }
                    let run_start_advance = advance;
                    // `white-space: nowrap` text has no soft wrap opportunities at all, except
                    // behind a space at its end, which keeps the space from adding to the width
                    // of a line that ends there.
                    let wraps = computed_values
                        .get(entity)
                        .is_none_or(|style| style.text.white_space.wraps());
                    let mut other_breaks = break_opportunities_between_letters(&text_run.text);
                    for (pos, glyph_info) in &shaped {
                        let cluster = glyph_info.cluster as usize;
                        if other_breaks.peek() == Some(&cluster) {
                            other_breaks.next();
                            // (the glyph still advances the pen when there is no break here)
                            if wraps {
                                // Justification stretches the gaps next to CJK characters. Not at
                                // a change of the bidi level, where the gap would be between two
                                // pieces of the line (see `shape_piece`).
                                let inter_letter = is_justified_between(&text_run.text, cluster)
                                    && !levels
                                        .iter()
                                        .any(|&(start, end, _)| start == cluster || end == cluster);
                                break_points.push(BreakPoint {
                                    entity,
                                    char_index: cluster,
                                    advance_before: advance.px(),
                                    advance_after: advance.px(),
                                    forced: false,
                                    space: false,
                                    inter_letter,
                                    hyphen: false,
                                    hyphen_width: Px(0.0),
                                    glue_stretch: if inter_letter {
                                        text_run.face.font_size / 8.0
                                    } else {
                                        Px(0.0)
                                    },
                                    glue_shrink: Px(0.0),
                                    penalty: 0,
                                });
                            }
                        }
                        // a break inside a cluster (e.g. a ligature) is not possible
                        while other_breaks.peek().is_some_and(|&index| index <= cluster) {
                            other_breaks.next();
                        }
                        let cur_char = &text_run.text[glyph_info.cluster as usize..];
                        // Only a space at the very end of the paragraph is a break in `nowrap`
                        // text: one in front of more content (an inline-level box, more text)
                        // is not.
                        let ends_run =
                            cur_char.trim_start().is_empty() && !content_after[index];
                        if cur_char.starts_with(|chr: char| chr.is_whitespace())
                            && (wraps || ends_run)
                        {
                            let advance_before = advance;
                            advance += font.scale_to_px(pos.x_advance, text_run.face.font_size);
                            let advance_after = advance;

                            let ws_len = cur_char.len() - cur_char.trim_start().len();

                            break_points.push(BreakPoint {
                                entity,
                                char_index: glyph_info.cluster as usize + ws_len,
                                advance_before: advance_before.px(),
                                advance_after: advance_after.px(),
                                forced: false,
                                space: true,
                                inter_letter: false,
                                hyphen: false,
                                hyphen_width: Px(0.0),
                                glue_stretch: Px(
                                    pos.x_advance.0 as f32 * text_run.face.font_size.0 / 2000.0
                                ),
                                glue_shrink: Px(
                                    pos.x_advance.0 as f32 * text_run.face.font_size.0 / 3000.0
                                ),
                                penalty: 0,
                            })
                        } else {
                            advance += font.scale_to_px(pos.x_advance, text_run.face.font_size);
                        }
                    }
                    let hyphenation = match text_run.hyphens {
                        _ if !wraps => Vec::new(),
                        crate::style::properties::specified::Hyphens::None => Vec::new(),
                        crate::style::properties::specified::Hyphens::Manual
                            if text_run.manual_hyphen_points.is_empty() =>
                        {
                            Vec::new()
                        }
                        crate::style::properties::specified::Hyphens::Manual => {
                            text_run.manual_hyphen_points.clone()
                        }
                        crate::style::properties::specified::Hyphens::Auto
                            if !text_run.manual_hyphen_points.is_empty() =>
                        {
                            text_run.manual_hyphen_points.clone()
                        }
                        crate::style::properties::specified::Hyphens::Auto => {
                            // without a dictionary for the language, words are not hyphenated
                            if dictionaries.get(&text_run.lang).is_none() {
                                Vec::new()
                            } else {
                                let mut points = Vec::new();
                                for (word_start, word_end) in word_ranges(&text_run.text) {
                                    let word = &text_run.text[word_start..word_end];
                                    points.extend(
                                        dictionaries
                                            .points(&text_run.lang, word)
                                            .iter()
                                            .map(|offset| word_start + offset),
                                    );
                                }
                                points
                            }
                        }
                    };
                    if !hyphenation.is_empty() {
                        // The width of the text in front of a glyph, from the shaping of the
                        // whole run above: `(cluster, width of all glyphs before it)`.
                        let mut prefix_widths = Vec::with_capacity(shaped.len());
                        let mut width = Pen::default();
                        for (position, glyph_info) in &shaped {
                            prefix_widths.push((glyph_info.cluster as usize, width.px()));
                            width += font.scale_to_px(position.x_advance, text_run.face.font_size);
                        }
                        let run_width = width.px();
                        // Clusters are monotone, so the width in front of a cluster boundary is
                        // found by a binary search.
                        let width_before = |char_index: usize| {
                            let glyphs_before =
                                prefix_widths.partition_point(|&(cluster, _)| cluster < char_index);
                            prefix_widths
                                .get(glyphs_before)
                                .map_or(run_width, |&(_, width)| width)
                        };
                        for char_index in hyphenation {
                            if !text_run.text.is_char_boundary(char_index) || char_index == 0 {
                                continue;
                            }
                            // Only the part of the word in front of the break is shaped again:
                            // a break can be inside of a ligature, which the shaping of the
                            // whole run does not split. Shaping the whole prefix of the run
                            // instead made this quadratic in the length of the run.
                            let prefix = &text_run.text[..char_index];
                            let word_start = prefix.rfind(char::is_whitespace).map_or(0, |index| {
                                index + prefix[index..].chars().next().map_or(1, char::len_utf8)
                            });
                            // The hyphen is shaped with the end of the word, as it is drawn, so that it
                            // is kerned with the letter in front of it: the kern is part of the advance
                            // of that letter.
                            let word_prefix = &prefix[word_start..];
                            let last = prefix.char_indices().next_back().map(|(index, _)| index);
                            let hyphen = hyphen_character(font, &text_run.face.fallback, last);
                            let (prefix_advance, hyphen_width) = with_shaped_hyphenated(
                                word_prefix,
                                hyphen,
                                font,
                                &text_run.face.fallback,
                                Some(word_start),
                                |glyphs| {
                                    let (mut before, mut after) = (Px(0.0), Px(0.0));
                                    for (position, glyph_info) in glyphs {
                                        let advance = font.scale_to_px(position.x_advance, text_run.face.font_size);
                                        if (glyph_info.cluster as usize) < word_prefix.len() {
                                            before += advance;
                                        } else {
                                            after += advance;
                                        }
                                    }
                                    (before, after)
                                },
                            );
                            let prefix_width = width_before(word_start) + prefix_advance;
                            // The line after the break starts with the rest of the word, shaped
                            // alone too (its first glyph is not kerned with the last one of the
                            // line before, and a ligature across the break is split). The rest
                            // is measured back from the end of the word in the shaping of the
                            // whole run, so that the widths of the two lines add up to it.
                            let word_end = char_index
                                + text_run.text[char_index..]
                                    .find(char::is_whitespace)
                                    .unwrap_or(text_run.text.len() - char_index);
                            let suffix_width = with_shaped(
                                &text_run.text[char_index..word_end],
                                font,
                                &text_run.face.fallback,
                                Some(char_index),
                                |glyphs| {
                                    glyphs
                                        .iter()
                                        .map(|(position, _)| {
                                            font.scale_to_px(position.x_advance, text_run.face.font_size)
                                        })
                                        .fold(Px(0.0), |sum, width| sum + width)
                                },
                            );
                            let word_end_width = width_before(word_end);
                            break_points.push(BreakPoint {
                                entity,
                                char_index,
                                advance_before: (run_start_advance + prefix_width).px(),
                                advance_after: (run_start_advance + (word_end_width - suffix_width)).px(),
                                forced: false,
                                space: false,
                                inter_letter: false,
                                hyphen: true,
                                hyphen_width,
                                glue_stretch: Px(0.0),
                                glue_shrink: Px(0.0),
                                penalty: 50,
                            });
                        }
                    }
                    push_bidi_runs(bidi_runs, index, levels, &glyph_widths, element_start.px());
                } else if let Some(intrinsic_size) = intrinsic_sizes.get(entity) {
                    advance += intrinsic_size.width().unwrap_or_default();
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start.px());
                    if let Some(run) = bidi_runs.last_mut().filter(|run| run.element == index) {
                        run.advance_end = advance.px();
                    }
                } else if computed_values
                    .get(entity)
                    .map_or(false, |style| style.box_.display.is_atomic_inline())
                {
                    // the width is only known once the box has been laid out
                    atom_offsets.push((entity, advance.px()));
                    // A line may break in front of and behind an inline-level box even if there
                    // is no space next to it. The break in front is at the end of the previous
                    // element if that already provides a break there.
                    let atom_break = |char_index: usize, advance: Px| BreakPoint {
                        entity,
                        char_index,
                        advance_before: advance,
                        advance_after: advance,
                        forced: false,
                        space: false,
                        inter_letter: false,
                        hyphen: false,
                        hyphen_width: Px(0.0),
                        glue_stretch: Px(0.0),
                        glue_shrink: Px(0.0),
                        penalty: 0,
                    };
                    // (the white space of the parent decides, not the one of the box itself)
                    let wraps = computed_values
                        .get(entity)
                        .is_none_or(|style| !style.box_.parent_nowrap);
                    if wraps
                        && index > 0
                        && break_points.last().is_some_and(|last| last.advance_after != advance.px())
                    {
                        break_points.push(atom_break(ATOM_BREAK_BEFORE, advance.px()));
                    }
                    if wraps && index + 1 < elements.len() {
                        break_points.push(atom_break(ATOM_BREAK_AFTER, advance.px()));
                    }
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start.px());
                } else {
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start.px());
                }
                if let Some(style) = computed_values.get(entity) {
                    if style.text.line_break
                        == crate::style::properties::specified::LineBreak::Force
                    {
                        break_points.push(BreakPoint {
                            entity,
                            char_index: 0,
                            advance_before: advance.px(),
                            advance_after: advance.px(),
                            forced: true,
                            space: false,
                            inter_letter: false,
                            hyphen: false,
                            hyphen_width: Px(0.0),
                            glue_stretch: Px(0.0),
                            glue_shrink: Px(0.0),
                            penalty: -10_000,
                        })
                    }
                }
            }

            // insert last breakpoint at the end
            let last_entity = *elements.last().unwrap();
            let char_index = if let Some(text_run) = text_runs.get(last_entity) {
                text_run.text.len()
            } else {
                1
            };

            break_points.push(BreakPoint {
                entity: last_entity,
                char_index,
                advance_before: advance.px(),
                advance_after: advance.px(),
                forced: false,
                space: false,
                inter_letter: false,
                hyphen: false,
                hyphen_width: Px(0.0),
                glue_stretch: Px(0.0),
                glue_shrink: Px(0.0),
                penalty: 0,
            });

            let element_order = |entity| {
                elements
                    .iter()
                    .position(|&item| item == entity)
                    .unwrap_or(usize::MAX)
            };
            break_points.sort_by_key(|break_point| {
                (element_order(break_point.entity), break_point.char_index)
            });
            // A break behind a box is redundant if a space follows it directly.
            let is_atom = |entity: Entity| atom_offsets.iter().any(|&(atom, _)| atom == entity);
            let mut index = 0;
            while index + 1 < break_points.len() {
                let point = break_points[index];
                let next = break_points[index + 1];
                if is_atom(point.entity)
                    && point.char_index == ATOM_BREAK_AFTER
                    && next.space
                    && next.advance_before == point.advance_before
                {
                    break_points.remove(index);
                } else {
                    index += 1;
                }
            }
            // a space at the end of the paragraph hangs: it is not part of the width of the
            // last line (nor of the intrinsic width)
            let count = break_points.len();
            if count >= 2 {
                let (before, last) = (break_points[count - 2], break_points[count - 1]);
                let only_space = before.entity == last.entity
                    && text_runs
                        .get(last.entity)
                        .is_some_and(|run| {
                            // (a run of white space only follows something else on the line)
                            !run.text.trim().is_empty()
                                && run.text.get(before.char_index..).is_some_and(|rest| rest.chars().all(char::is_whitespace))
                        });
                if before.space && !before.forced && only_space && before.advance_after == last.advance_before {
                    break_points[count - 1].advance_before = before.advance_before;
                }
            }
        }
    }
}

/// Appends the bidi runs of the element at `index` (with the level runs `levels` of its text) to
/// `bidi_runs`. `glyph_widths` are the glyphs of the text in logical order as `(cluster, width)`
/// and `element_start` is the advance at the start of the element.
fn push_bidi_runs(
    bidi_runs: &mut Vec<BidiRun>,
    index: usize,
    levels: &[(usize, usize, u8)],
    glyph_widths: &[(usize, Px)],
    element_start: Px,
) {
    let mut advance = element_start;
    let mut glyphs = glyph_widths.iter().peekable();
    for &(start, end, level) in levels {
        let advance_start = advance;
        while let Some(&&(cluster, width)) = glyphs.peek() {
            if cluster >= end {
                break;
            }
            advance += width;
            glyphs.next();
        }
        bidi_runs.push(BidiRun {
            element: index,
            start,
            end,
            level,
            advance_start,
            advance_end: advance,
        });
    }
}

/// The hyphenation dictionaries of the languages of a document, loaded when they are first
/// needed. Only the dictionaries that are embedded into Helium (Cargo features `embed_*` of the
/// `hyphenation` crate, currently US English) are available. The hyphenation points of the words
/// that were looked up are kept: words repeat, and the dictionary allocates for every lookup.
#[derive(Default)]
struct HyphenationDictionaries {
    dictionaries: HashMap<Language, Option<Standard>>,
    /// The points of a word (see `limit_hyphenation_chars`) by language.
    words: HashMap<Language, HashMap<String, Vec<usize>>>,
}

impl HyphenationDictionaries {
    /// Loads the dictionary of `language` if it has not been.
    fn load(&mut self, language: Language) {
        self.dictionaries.entry(language).or_insert_with(|| {
            Standard::from_embedded(language).ok().or_else(|| {
                // British English is hyphenated as American English without its dictionary
                (language == Language::EnglishGB)
                    .then(|| Standard::from_embedded(Language::EnglishUS).ok())
                    .flatten()
            })
        });
    }

    /// The dictionary for text in `lang`. Text of an unknown language is hyphenated as English,
    /// text in a language without a dictionary not at all.
    fn get(&mut self, lang: &crate::style::properties::computed::Language) -> Option<&Standard> {
        let language = hyphenation_language(lang)?;
        self.load(language);
        self.dictionaries[&language].as_ref()
    }

    /// The byte offsets in `word` of the hyphenation points that leave enough characters on both
    /// sides (see `limit_hyphenation_chars`), in text in `lang`.
    fn points(&mut self, lang: &crate::style::properties::computed::Language, word: &str) -> &[usize] {
        // a word that is too short has no points: the dictionary does not need to be asked
        if word.chars().count() < HYPHENATE_MIN_WORD {
            return &[];
        }
        let Some(language) = hyphenation_language(lang) else {
            return &[];
        };
        self.load(language);
        let words = self.words.entry(language).or_default();
        if !words.contains_key(word) {
            let points = match &self.dictionaries[&language] {
                Some(dictionary) => limit_hyphenation_chars(word, dictionary.hyphenate(word).breaks),
                None => Vec::new(),
            };
            words.insert(word.to_owned(), points);
        }
        &words[word]
    }
}

/// The shortest word that is hyphenated, and the fewest characters in front of and after the
/// hyphen: the default of `hyphenate-limit-chars`, which is `auto` (5 2 2) in browsers. Words
/// are not broken after a single letter, which looks as bad as it reads.
/// https://www.w3.org/TR/css-text-4/#hyphenate-char-limits
const HYPHENATE_MIN_WORD: usize = 5;
const HYPHENATE_MIN_BEFORE: usize = 2;
const HYPHENATE_MIN_AFTER: usize = 2;

/// The hyphenation points of `word` (byte offsets) that leave enough characters on both sides.
fn limit_hyphenation_chars(word: &str, breaks: Vec<usize>) -> Vec<usize> {
    let length = word.chars().count();
    if length < HYPHENATE_MIN_WORD {
        return Vec::new();
    }
    breaks
        .into_iter()
        .filter(|&offset| {
            let before = word[..offset].chars().count();
            before >= HYPHENATE_MIN_BEFORE && length - before >= HYPHENATE_MIN_AFTER
        })
        .collect()
}

/// The language of the hyphenation dictionaries for text in `lang`.
fn hyphenation_language(lang: &crate::style::properties::computed::Language) -> Option<Language> {
    let Some(primary) = lang.primary() else {
        return Some(Language::EnglishUS);
    };
    let region = lang.region();
    let language = match primary {
        "en" => match region {
            Some("gb" | "au" | "nz" | "ie" | "za" | "in") => Language::EnglishGB,
            _ => Language::EnglishUS,
        },
        "de" | "gsw" => match region {
            Some("ch" | "li") => Language::GermanSwiss,
            _ => Language::German1996,
        },
        "el" => Language::GreekMono,
        "sr" if lang.script() != Some("latn") => Language::SerbianCyrillic,
        "mn" => Language::Mongolian,
        "no" => Language::NorwegianBokmal,
        _ => {
            return lang
                .tag()
                .and_then(Language::try_from_code)
                .or_else(|| Language::try_from_code(primary));
        }
    };
    Some(language)
}

/// The byte indices in `text` where a line may break although there is no space in front, by the
/// line breaking algorithm of Unicode (UAX #14): between the ideographs of Chinese and Japanese
/// (but not in front of their closing punctuation), after hyphens and dashes, and so on.
///
/// Breaks after spaces are not included; they are break points of their own that stretch for
/// justification. Neither are mandatory breaks, which are handled by the `white-space` processing.
fn break_opportunities_between_letters(
    text: &str,
) -> std::iter::Peekable<impl Iterator<Item = usize> + '_> {
    static SEGMENTER: std::sync::LazyLock<icu_segmenter::LineSegmenterBorrowed<'static>> =
        std::sync::LazyLock::new(|| icu_segmenter::LineSegmenter::new_auto(Default::default()));
    SEGMENTER
        .segment_str(text)
        .filter(move |&index| {
            index > 0 && index < text.len() && !text[..index].ends_with(char::is_whitespace)
        })
        .peekable()
}

/// Whether a justified line is stretched at the break opportunity at `index` of `text`: when
/// the character before or after it is a Chinese, Japanese or Korean character (or full-width
/// punctuation), which are set without spaces.
fn is_justified_between(text: &str, index: usize) -> bool {
    let before = text[..index].chars().next_back();
    let after = text[index..].chars().next();
    [before, after].into_iter().flatten().any(is_cjk)
}

fn is_cjk(ch: char) -> bool {
    use unicode_script::{Script, UnicodeScript};
    matches!(
        ch.script(),
        Script::Han | Script::Hiragana | Script::Katakana | Script::Hangul | Script::Bopomofo
    ) || matches!(ch, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}')
}

fn word_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character.is_alphabetic() {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            ranges.push((word_start, index));
        }
    }
    if let Some(word_start) = start {
        ranges.push((word_start, text.len()));
    }
    ranges
}

/// Creates the paragraphs that draw the markers of list items. Runs after the line boxes are
/// created, because the marker is aligned with the first line of the item.
pub struct ListMarkerCreator;

impl<'a> System<'a> for ListMarkerCreator {
    type SystemData = (
        Entities<'a>,
        Read<'a, components::TextList>,
        ReadStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, components::ListMarker>,
        WriteStorage<'a, components::Position>,
        WriteStorage<'a, Paragraph>,
        WriteStorage<'a, components::VectorGraphics>,
        Read<'a, FontMap>,
        Read<'a, StructureTree>,
    );

    fn run(
        &mut self,
        (entities, text_list, ifcs, markers, mut positions, mut paragraphs, mut graphics, font_map, structure): Self::SystemData,
    ) {
        use components::BlockLevelElement;
        // the markers whose item has not yet produced its first line
        let mut pending: Vec<Entity> = vec![];
        let mut created = vec![];
        let baselines: HashMap<Entity, Px> = ifcs
            .join()
            .flat_map(|ifc| ifc.lines.iter().map(|line| (line.entity, line.baseline)))
            .collect();
        for &elem in &text_list.list {
            match elem {
                BlockLevelElement::ContainerStart(entity) => {
                    if markers.get(entity).is_some() {
                        pending.push(entity);
                    }
                }
                BlockLevelElement::Leaf(leaf) => {
                    if pending.is_empty() {
                        continue;
                    }
                    // the leaves of inline content are the line boxes
                    let baseline = match baselines.get(&leaf) {
                        Some(&baseline) => baseline,
                        None => continue,
                    };
                    let line_pos = *positions.get(leaf).unwrap();
                    for item in pending.drain(..) {
                        let marker = markers.get(item).unwrap();
                        let item_x = positions.get(item).map_or(line_pos.x, |pos| pos.x);
                        created.push((item, item_x, line_pos, baseline, marker.clone()));
                    }
                }
                _ => {}
            }
        }

        for (item, item_x, line_pos, baseline, marker) in created {
            let size = marker.face.font_size;
            // These shapes are not available in most fonts, so they are drawn instead.
            let shape = match marker.text.as_str() {
                "\u{25E6}" => Some((size * 0.36, false)),
                "\u{25AA}" => Some((size * 0.3, true)),
                _ => None,
            };
            if let Some((diameter, filled)) = shape {
                let center_x = item_x - size * 0.4 - diameter / 2.0;
                let center_y = line_pos.y + baseline - size * 0.3;
                let mut ops = vec![];
                if filled {
                    // a horizontal line as thick as it is wide
                    ops.push(GraphicsOp::Line {
                        from_x: center_x - diameter / 2.0,
                        from_y: center_y,
                        to_x: center_x + diameter / 2.0,
                        to_y: center_y,
                        thickness: diameter,
                    });
                } else {
                    let stroke = size * 0.06;
                    let radius = (diameter - stroke) / 2.0;
                    const SEGMENTS: usize = 24;
                    let point = |i: usize| {
                        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                        (
                            center_x + radius * angle.cos(),
                            center_y + radius * angle.sin(),
                        )
                    };
                    for i in 0..SEGMENTS {
                        let (from_x, from_y) = point(i);
                        let (to_x, to_y) = point(i + 1);
                        ops.push(GraphicsOp::Line {
                            from_x,
                            from_y,
                            to_x,
                            to_y,
                            thickness: stroke,
                        });
                    }
                }
                entities
                    .build_entity()
                    .with(
                        components::Position {
                            y: line_pos.y,
                            height: Px(0.0),
                            ..line_pos
                        },
                        &mut positions,
                    )
                    .with(
                        components::VectorGraphics {
                            ops,
                            color: Some(marker.face.color),
                        },
                        &mut graphics,
                    )
                    .build();
                continue;
            }
            let font = &font_map[&marker.face.font_name];
            let ShapedLine { mut ops, width, color_glyphs } = shape_line(&marker.text, font, &marker.face, false);
            // the marker is the label (`Lbl`) of the list item
            if let Some(tag) = structure.content_tag(item, 0) {
                ops.insert(0, TextOperator::BeginTag(tag));
                ops.push(TextOperator::EndMarkedContent);
            }
            let gap = if marker.explicit { Px(0.0) } else { marker.face.font_size * 0.4 };
            let start_pos = (item_x - gap - width, line_pos.y + baseline);
            let color_glyphs = color_glyphs
                .into_iter()
                .map(|glyph| PlacedColorGlyph {
                    x: glyph.x + start_pos.0,
                    baseline: glyph.baseline + start_pos.1,
                    ..glyph
                })
                .collect();
            entities
                .build_entity()
                .with(
                    components::Position {
                        y: line_pos.y,
                        height: Px(0.0),
                        ..line_pos
                    },
                    &mut positions,
                )
                .with(
                    Paragraph {
                        start_pos,
                        ops,
                        color_glyphs,
                    },
                    &mut paragraphs,
                )
                .build();
        }
    }
}

/// Moves the contents of `inline-block` boxes to the place of the box on its line.
///
/// The contents of a box are laid out relative to the margin box of the box, see
/// [`components::AbsoluteGroup::owner`]. This runs after pagination because the line the box is
/// on can be on any page, and before the line boxes are created, because everything that is
/// drawn is derived from the final positions.
/// The ascent and descent of the box of an inline-level entity: a piece of text or an atomic box.
fn own_extent(
    entity: Entity,
    text_runs: &ReadStorage<components::TextRun>,
    intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
) -> Option<(Px, Px)> {
    if let Some(run) = text_runs.get(entity) {
        return Some((run.baseline, run.line_height - run.baseline));
    }
    match intrinsic_sizes.get(entity) {
        Some(&IntrinsicSize::SizeWithBaseline { height, baseline, .. }) => {
            Some((baseline, height - baseline))
        }
        _ => None,
    }
}

/// How far the box of `entity` is raised above the baseline of its line (which is
/// `line_baseline` from the top of a line of `line_height`) by its `vertical-align`.
fn align_raise(
    entity: Entity,
    own: (Px, Px),
    aligns: &ReadStorage<components::VerticalAlignment>,
    line_baseline: Px,
    line_height: Px,
) -> Px {
    let Some(align) = aligns.get(entity) else {
        return Px(0.0);
    };
    match align.place(own) {
        components::LinePlacement::Raise(raise) => raise,
        components::LinePlacement::Top { inner, ascent, .. } => line_baseline - ascent + inner,
        components::LinePlacement::Bottom { inner, descent, .. } => {
            descent - (line_height - line_baseline) + inner
        }
    }
}

pub struct PlaceInlineBlocks;

impl<'a> System<'a> for PlaceInlineBlocks {
    type SystemData = (
        Read<'a, components::AbsoluteGroups>,
        ReadStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, IntrinsicSize>,
        ReadStorage<'a, components::RelativeOffset>,
        ReadStorage<'a, components::VerticalAlignment>,
        WriteStorage<'a, Position>,
    );

    fn run(
        &mut self,
        (groups, ifcs, intrinsic_sizes, relative_offsets, aligns, mut positions): Self::SystemData,
    ) {
        const EPSILON: f32 = 0.01;

        // where the margin box of every box is, as (x, y, page)
        let mut placements: HashMap<Entity, (Px, Px, usize)> = HashMap::new();

        for ifc in ifcs.join() {
            if ifc.atom_offsets.is_empty() {
                continue;
            }
            for (line_num, line) in ifc.lines.iter().enumerate() {
                let line_pos = match positions.get(line.entity) {
                    Some(&pos) => pos,
                    None => continue,
                };
                let from = ifc.break_points[line.from_break];
                let to = ifc.break_points[line.to_break];

                // The top left corner of the margin box of a box that is `along` the line from
                // its start and whose baseline is at `baseline` of its `height`. In a vertical
                // flow the line runs down and the block start edge of the line is on the
                // right, so the box hangs from there.
                let flow = ifc.flow;
                let line_size = flow.swap((line_pos.width, line_pos.height)).0;
                // `position: relative` moves the box after it is placed
                let shift = |atom: Entity| {
                    let (mut dx, mut dy) = relative_offsets
                        .get(atom)
                        .map_or((Px(0.0), Px(0.0)), |offset| {
                            offset.resolve_in_line(flow, line_size, ifc.base_rtl)
                        });
                    // `vertical-align`
                    if let Some(&IntrinsicSize::SizeWithBaseline { height, baseline, .. }) =
                        intrinsic_sizes.get(atom)
                    {
                        let raise = align_raise(
                            atom,
                            (baseline, height - baseline),
                            &aligns,
                            line.baseline,
                            line.height,
                        );
                        if flow.is_vertical() {
                            dx += raise;
                        } else {
                            dy = dy - raise;
                        }
                    }
                    (dx, dy)
                };
                let place = |along: Px, baseline: Px, height: Px| {
                    if flow.is_vertical() {
                        let right = line_pos.x + line_pos.width;
                        (right - (line.baseline - baseline) - height, line_pos.y + along)
                    } else {
                        (line_pos.x + along, line_pos.y + line.baseline - baseline)
                    }
                };

                if !ifc.bidi_runs.is_empty() {
                    // the boxes are reordered with the text
                    for (atom, offset) in ifc.visual_atom_offsets(line_num, line_size) {
                        let Some(&IntrinsicSize::SizeWithBaseline { baseline, height, .. }) =
                            intrinsic_sizes.get(atom)
                        else {
                            continue;
                        };
                        if !ifc.atom_offsets.iter().any(|&(entity, _)| entity == atom) {
                            continue;
                        }
                        let (x, y) = place(offset, baseline, height);
                        let (dx, dy) = shift(atom);
                        placements.insert(atom, (x + dx, y + dy, line_pos.page));
                    }
                    continue;
                }

                // the same word spacing as the line box creator uses
                let word_spacing = ifc.word_spacing(line_num, line_size);

                for &(atom, offset) in &ifc.atom_offsets {
                    let (width, baseline, height) = match intrinsic_sizes.get(atom) {
                        Some(&IntrinsicSize::SizeWithBaseline {
                            width, baseline, height,
                        }) => (width, baseline, height),
                        _ => continue,
                    };
                    let _ = width;
                    if offset.0 + EPSILON < from.advance_after.0
                        || offset.0 >= to.advance_before.0 - EPSILON
                    {
                        continue;
                    }
                    // the gaps that justification stretches in front of the box
                    let spaces = ifc.break_points[line.from_break + 1..line.to_break]
                        .iter()
                        .filter(|point| {
                            (point.space || point.inter_letter)
                                && point.advance_before.0 < offset.0 - EPSILON
                        })
                        .count();
                    let along = (offset - from.advance_after) + word_spacing * spaces as f32;
                    let (x, y) = place(along, baseline, height);
                    let (dx, dy) = shift(atom);
                    placements.insert(atom, (x + dx, y + dy, line_pos.page));
                }
            }
        }

        // The contents of a box that is inside of another box move with that box, so the outer
        // boxes are placed first.
        let depth = |mut group: usize| {
            let mut depth = 0;
            while let Some(parent) = groups.groups[group].parent {
                group = parent;
                depth += 1;
            }
            depth
        };
        let mut order: Vec<usize> = (0..groups.groups.len())
            .filter(|&g| {
                groups.groups[g]
                    .owner
                    .map_or(false, |owner| placements.contains_key(&owner))
            })
            .collect();
        order.sort_by_key(|&g| depth(g));

        for group in order {
            let owner = groups.groups[group].owner.unwrap();
            let (dx, dy, page) = placements[&owner];
            // the entities of the box, and of the absolutely positioned boxes in it (those
            // have no owner: their containing block is the box or a box in it)
            let mut moving = vec![group];
            let mut next = 0;
            while let Some(&current) = moving.get(next) {
                next += 1;
                for &entity in &groups.groups[current].entities {
                    if let Some(pos) = positions.get_mut(entity) {
                        pos.x += dx;
                        pos.y += dy;
                        pos.page = page;
                    }
                }
                for (child, other) in groups.groups.iter().enumerate() {
                    if other.parent != Some(current) || child == current {
                        continue;
                    }
                    match other.owner {
                        // (a group that is anchored to a box is placed with that box)
                        None if other.anchor.is_some() => {}
                        None => moving.push(child),
                        Some(owner) => {
                            // an inline-block box in the box moves with it
                            if let Some(placement) = placements.get_mut(&owner) {
                                placement.0 += dx;
                                placement.1 += dy;
                                placement.2 = page;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct LineBoxCreator;

impl<'a> System<'a> for LineBoxCreator {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, components::TextRun>,
        ReadStorage<'a, components::IntrinsicSize>,
        ReadStorage<'a, ComputedValues>,
        ReadStorage<'a, components::RelativeOffset>,
        ReadStorage<'a, components::VerticalAlignment>,
        Read<'a, EntityHrefs>,
        Write<'a, Hyperlinks>,
        WriteStorage<'a, components::Position>,
        WriteStorage<'a, Paragraph>,
        WriteStorage<'a, components::VectorGraphics>,
        Read<'a, FontMap>,
        Read<'a, StructureTree>,
    );

    fn run(
        &mut self,
        (
            entities,
            mut inline_formatting_contexts,
            text_runs,
            intrinsic_sizes,
            computed_values,
            relative_offsets,
            aligns,
            entity_hrefs,
            mut hyperlinks,
            mut positions,
            mut paragraphs,
            mut graphics,
            font_map,
            structure,
        ): Self::SystemData,
    ) {
        // The break points are only needed to make the line boxes of their context. They are
        // freed as soon as it is done (when the next context starts), so that they are not alive
        // together with the text of all the other contexts.
        let contexts: Vec<Entity> = (&entities, &inline_formatting_contexts).join().map(|(entity, _)| entity).collect();
        let mut previous: Option<Entity> = None;
        for context in contexts {
            if let Some(done) = previous.replace(context) {
                inline_formatting_contexts.get_mut(done).expect("the context exists").break_points = Vec::new();
            }
            let ifc = inline_formatting_contexts.get(context).expect("the context exists");
            let InlineFormattingContext {
                break_points,
                lines,
                elements,
                ..
            } = ifc;
            if elements.is_empty() {
                // an empty block has nothing to draw, but other contexts still do
                continue;
            }

            let mut ops = TextOps::default();
            let mut color_glyphs: Vec<components::PlacedColorGlyph> = vec![];
            let mut previous_font: Option<(String, Px)> = None;
            let mut previous_color: Option<PaintColor> = None;
            let mut paragraph_pos: Option<components::Position> = None;

            // position of the first baseline of the paragraph
            let mut paragraph_start = (Px(0.0), Px(0.0));

            // The x-intervals on the current line of the hyperlinks that have text on it, as
            // `(href, link node, start, end, shift down)`. Adjacent intervals of the same link are merged as
            // the pieces of the line are visited from left to right.
            let mut link_intervals: Vec<(&str, Option<StructNodeId>, Px, Px, Px)> = Vec::new();

            // The tag of the open marked-content sequence and the end of the text it covers so
            // far. Consecutive pieces of the same text node share a sequence as long as they
            // are in logical order; it ends at page breaks because every page has its own
            // content stream.
            let mut open_tag: Option<(ContentTag, usize)> = None;

            for (line_num, line) in lines.iter().enumerate() {
                let to = line.to_break;

                let line_pos = *positions
                    .get(line.entity)
                    .expect("every line should have been assigned a position by now");

                if line_num == 0 {
                    paragraph_start = (line_pos.x, line_pos.y + line.baseline);
                }

                // In a vertical flow the lines run downwards: the size of a line along the line is
                // its physical height.
                let flow = ifc.flow;
                let line_size = flow.swap((line_pos.width, line_pos.height)).0;
                // The physical place of the start of the line on the baseline, which is where
                // the line matrix of vertical text goes.
                let line_origin = flow.line_origin(
                    (line_pos.x.0, line_pos.y.0, line_pos.width.0, line_pos.height.0),
                    line.baseline.0,
                );

                // if we want to justify the text we have to add word-spacing
                let word_spacing = ifc.word_spacing(line_num, line_size);

                // Check if a page break happened between this line and the previous. If a page
                // break happend we have to add the existing (partial) paragraph to the previous
                // page and create a fresh paragraph for the remaining lines.
                if paragraph_pos.map(|pos| pos.page) != Some(line_pos.page) {
                    if open_tag.take().is_some() {
                        ops.push(TextOperator::EndMarkedContent);
                    }
                    if let Some(pos) = paragraph_pos {
                        entities
                            .build_entity()
                            .with(pos, &mut positions)
                            .with(
                                Paragraph {
                                    start_pos: paragraph_start,
                                    ops: std::mem::take(&mut ops),
                                    color_glyphs: std::mem::take(&mut color_glyphs),
                                },
                                &mut paragraphs,
                            )
                            .build();
                    }
                    ops.clear();
                    paragraph_start = (line_pos.x, line_pos.y + line.baseline);
                    if let Some(previous_font) = &previous_font {
                        ops.push(TextOperator::SetFont(
                            previous_font.0.clone(),
                            previous_font.1,
                        ))
                    }
                    if let Some(previous_color) = previous_color {
                        ops.push(TextOperator::SetColor(previous_color));
                    }
                    paragraph_pos = Some(components::Position {
                        y: line_pos.y,
                        height: Px(0.0),
                        ..line_pos
                    });
                }

                let to_break = break_points[to];
                let pieces = ifc.line_pieces(line, &text_runs);
                let last_piece = pieces.len().checked_sub(1);

                if flow.is_vertical() {
                    // the text is turned: its x axis is the inline direction, which runs down
                    let [a, b, c, d] = flow.text_matrix();
                    ops.push(TextOperator::SetTransform(components::TransformMatrix {
                        a,
                        b,
                        c,
                        d,
                        e: Px(line_origin.0),
                        f: Px(line_origin.1),
                    }));
                }

                // the x position on the line and the x position of the text line matrix
                let mut advance = ifc.line_start_offset(line_num, line_size, word_spacing);
                let mut text_cursor = Px(0.0);
                if advance != Px(0.0) {
                    ops.push(TextOperator::NewLine {
                        x_offset: advance,
                        leading: Px(0.0),
                    });
                    text_cursor = advance;
                }

                for piece_index in super::bidi::visual_order(
                    &pieces.iter().map(|piece| piece.level).collect::<Vec<_>>(),
                ) {
                    let piece = pieces[piece_index];
                    let entity = elements[piece.element];
                    // The x position of the current piece on the line, before its width is
                    // added to `advance`.
                    let piece_start = advance;
                    // `position: relative` on an inline box moves the text in it, and the boxes
                    // that are atomic inlines (those are shifted when they are placed); the
                    // space that they take up on the line stays. (Not in vertical text: the text
                    // matrix is turned there.)
                    let (mut physical_x, mut physical_y) = relative_offsets
                        .get(entity)
                        .map_or((Px(0.0), Px(0.0)), |offset| {
                            offset.resolve_in_line(flow, line_size, ifc.base_rtl)
                        });
                    // `vertical-align` raises the box (to the right in vertical text, where the
                    // tops of the glyphs are)
                    if let Some(own) = own_extent(entity, &text_runs, &intrinsic_sizes) {
                        let raise = align_raise(entity, own, &aligns, line.baseline, line.height);
                        if flow.is_vertical() {
                            physical_x += raise;
                        } else {
                            physical_y = physical_y - raise;
                        }
                    }
                    // the same move along the line and across it: along the line is to the
                    // right (down in vertical text) and across it is down (to the right)
                    let (shift_x, shift_y) = if flow.is_vertical() {
                        (physical_y, Px(0.0) - physical_x)
                    } else {
                        (physical_x, physical_y)
                    };
                    let shifted = shift_x != Px(0.0) || shift_y != Px(0.0);
                    // the move across the line, in physical terms
                    let cross_shift = if flow.is_vertical() { physical_x } else { physical_y };
                    let tag = text_runs
                        .get(entity)
                        .and_then(|_| structure.content_tag(entity, piece.start));
                    match (open_tag, tag) {
                        (Some((open, end)), Some(tag))
                            if open.slot == tag.slot && piece.start >= end =>
                        {
                            open_tag = Some((open, piece.end));
                        }
                        _ => {
                            if open_tag.take().is_some() {
                                ops.push(TextOperator::EndMarkedContent);
                            }
                            if let Some(tag) = tag {
                                ops.push(TextOperator::BeginTag(tag));
                                open_tag = Some((tag, piece.end));
                            }
                        }
                    }
                    let piece_width = if let Some(run) = text_runs.get(entity) {
                        if previous_font != Some((run.face.font_name.clone(), run.face.font_size)) {
                            ops.push(TextOperator::SetFont(run.face.font_name.clone(), run.face.font_size));
                            previous_font = Some((run.face.font_name.clone(), run.face.font_size));
                        }

                        if previous_color != Some(run.face.color) {
                            ops.push(TextOperator::SetColor(run.face.color));
                            previous_color = Some(run.face.color);
                        }

                        let characters = &run.text[piece.start..piece.end];
                        let font = &font_map[&run.face.font_name];
                        let rtl = piece.is_rtl();

                        // the hyphen of a hyphenated word goes at its end
                        let hyphen = to_break.hyphen && Some(piece_index) == last_piece;
                        let mut piece_width = Px(0.0);
                        if shifted && text_runs.get(entity).is_some() {
                            ops.push(TextOperator::NewLine {
                                x_offset: piece_start + shift_x - text_cursor,
                                leading: shift_y,
                            });
                            text_cursor = piece_start + shift_x;
                        }
                        let vertical = flow.is_vertical().then_some(VerticalPiece {
                            flow,
                            baseline_x: Px(line_origin.0),
                            start_y: line_pos.y + piece_start,
                        });
                        let ops_before = ops.len();
                        // the hyphen is drawn with the font of the last letter in front of it
                        let hyphen = hyphen.then(|| {
                            let last = characters.char_indices().next_back().map(|(index, _)| piece.start + index);
                            hyphen_character(font, &run.face.fallback, last)
                        });
                        piece_width += shape_piece_in_flow(
                            characters,
                            font,
                            &run.face.fallback,
                            Some(piece.start),
                            run.face.font_size,
                            word_spacing,
                            &ifc.inter_letter_offsets(line, &piece),
                            rtl,
                            hyphen,
                            vertical,
                            &mut ops,
                        );
                        // the colour glyphs are painted over the text, at their own positions
                        color_glyphs.extend(take_color_glyphs(
                            &mut ops,
                            ops_before,
                            (line_pos.x + piece_start + physical_x, line_pos.y + line.baseline + physical_y),
                            run.face.color,
                        ));
                        if shifted {
                            // back to where the next piece starts
                            ops.push(TextOperator::NewLine {
                                x_offset: piece_width - shift_x,
                                leading: Px(0.0) - shift_y,
                            });
                            text_cursor = piece_start + piece_width;
                        }
                        if (run.background.is_some() || run.edges.is_some()) && piece_width > Px(0.0) && !flow.is_vertical() {
                            // the background of an inline box is the em box of its font, with the
                            // vertical padding and border
                            let ascent = font.scale_to_px(font.ascent(), run.face.font_size);
                            let descent = Px(0.0) - font.scale_to_px(font.descent(), run.face.font_size);
                            let x = line_pos.x + advance + physical_x;
                            let baseline_y = line_pos.y + line.baseline + physical_y;
                            let (above, below) = run.edges.map_or((Px(0.0), Px(0.0)), |edges| {
                                (Px(edges.padding_top + edges.border[0].0), Px(edges.padding_bottom + edges.border[2].0))
                            });
                            let (top, bottom) = (baseline_y - ascent - above, baseline_y + descent + below);
                            let mut fill = |from: (Px, Px), to: (Px, Px), thickness: Px, color| {
                                entities
                                    .build_entity()
                                    .with(
                                        components::Position { y: line_pos.y, height: Px(0.0), ..line_pos },
                                        &mut positions,
                                    )
                                    .with(
                                        components::VectorGraphics {
                                            ops: vec![GraphicsOp::Line {
                                                from_x: from.0,
                                                from_y: from.1,
                                                to_x: to.0,
                                                to_y: to.1,
                                                thickness,
                                            }],
                                            color: Some(color),
                                        },
                                        &mut graphics,
                                    )
                                    .build();
                            };
                            let middle = (top + bottom) / 2.0;
                            if let Some(background) = run.background {
                                fill((x, middle), (x + piece_width, middle), bottom - top, background);
                            }
                            if let Some(edges) = run.edges {
                                let [(top_width, top_color), (right_width, right_color), (bottom_width, bottom_color), (left_width, left_color)] =
                                    edges.border;
                                if top_width > 0.0 {
                                    let y = top + Px(top_width / 2.0);
                                    fill((x, y), (x + piece_width, y), Px(top_width), top_color);
                                }
                                if bottom_width > 0.0 {
                                    let y = bottom - Px(bottom_width / 2.0);
                                    fill((x, y), (x + piece_width, y), Px(bottom_width), bottom_color);
                                }
                                // the border at the start of the box is at the start of the run
                                let (left_edge, right_edge) = match (edges.start, edges.end, rtl) {
                                    (true, _, false) | (_, true, true) => (true, false),
                                    (true, _, true) | (_, true, false) => (false, true),
                                    _ => (false, false),
                                };
                                if left_edge && left_width > 0.0 {
                                    let at = x + Px(left_width / 2.0);
                                    fill((at, top), (at, bottom), Px(left_width), left_color);
                                }
                                if right_edge && right_width > 0.0 {
                                    let at = x + piece_width - Px(right_width / 2.0);
                                    fill((at, top), (at, bottom), Px(right_width), right_color);
                                }
                            }
                        }
                        if !run.decoration.is_none() && piece_width > Px(0.0) {
                            let page_pos = components::Position {
                                y: line_pos.y,
                                height: Px(0.0),
                                ..line_pos
                            };
                            let x = line_pos.x + advance + physical_x;
                            let baseline = line_pos.y + line.baseline + physical_y;
                            for ((offset, thickness), enabled) in
                                decoration_lines(run.face.font_size, run.decoration)
                            {
                                if !enabled {
                                    continue;
                                }
                                let y = baseline + offset;
                                // `offset` is down from the baseline; in a vertical flow that
                                // is to the left
                                let line_op = if flow.is_vertical() {
                                    GraphicsOp::Line {
                                        from_x: Px(line_origin.0) - offset + physical_x,
                                        from_y: line_pos.y + advance + physical_y,
                                        to_x: Px(line_origin.0) - offset + physical_x,
                                        to_y: line_pos.y + advance + physical_y + piece_width,
                                        thickness,
                                    }
                                } else {
                                    GraphicsOp::Line {
                                        from_x: x,
                                        from_y: y,
                                        to_x: x + piece_width,
                                        to_y: y,
                                        thickness,
                                    }
                                };
                                entities
                                    .build_entity()
                                    .with(page_pos, &mut positions)
                                    .with(
                                        components::VectorGraphics {
                                            ops: vec![line_op],
                                            color: Some(run.face.color),
                                        },
                                        &mut graphics,
                                    )
                                    .build();
                            }
                        }
                        piece_width
                    } else if let Some(&IntrinsicSize::SizeWithBaseline {
                        width,
                        height,
                        baseline,
                    }) = intrinsic_sizes.get(entity)
                    {
                        // The contents of an `inline-block` box are placed by `PlaceInlineBlocks`.
                        let is_inline_block = computed_values
                            .get(entity)
                            .map_or(false, |style| style.box_.display.is_atomic_inline());
                        if is_inline_block {
                            ops.push(TextOperator::NewLine {
                                x_offset: advance + width - text_cursor,
                                leading: Px(0.0),
                            });
                        } else {
                            // the content of a formula is painted here, in logical order
                            ops.push(TextOperator::Embed {
                                entity,
                                x: line_pos.x + advance + width,
                                y: line_pos.y + line.baseline,
                            });
                            previous_font = None;
                            previous_color = None;
                        }
                        text_cursor = advance + width;

                        if !is_inline_block {
                            positions
                                .insert(
                                    entity,
                                    Position {
                                        x: line_pos.x + advance + physical_x,
                                        y: line_pos.y + line.baseline - baseline + physical_y,
                                        width,
                                        height,
                                        ..paragraph_pos.unwrap()
                                    },
                                )
                                .unwrap();
                        }
                        width
                    } else {
                        log::error!("unsopported inline element type");
                        Px(0.0)
                    };
                    advance += piece_width;

                    // Extend the interval of the hyperlink this piece belongs to, if any.
                    if piece_width > Px(0.0) {
                        if let Some(href) = entity_hrefs.href_of(entity) {
                            let node = structure.link_node(entity);
                            match link_intervals.last_mut() {
                                Some(interval)
                                    if interval.0 == href
                                        && interval.1 == node
                                        && interval.3 == piece_start + shift_x
                                        && interval.4 == cross_shift =>
                                {
                                    interval.3 = advance + shift_x;
                                }
                                _ => link_intervals.push((
                                    href,
                                    node,
                                    piece_start + shift_x,
                                    advance + shift_x,
                                    cross_shift,
                                )),
                            }
                        }
                    }
                }

                if let Some(next_line) = lines.get(line_num + 1).filter(|_| !flow.is_vertical()) {
                    let next_line_pos = *positions.get(next_line.entity).unwrap();
                    // The next line is below this one, unless a column ends between them: then it
                    // is at the top of the next column, where its position says. (On another
                    // page it starts a paragraph of its own.)
                    let below = next_line_pos.page != line_pos.page
                        || (next_line_pos.y - (line_pos.y + line.height)).0.abs() < 0.01;
                    let leading = if below {
                        (line.height - line.baseline) + next_line.baseline
                    } else {
                        (next_line_pos.y + next_line.baseline) - (line_pos.y + line.baseline)
                    };
                    ops.push(TextOperator::NewLine {
                        x_offset: next_line_pos.x - line_pos.x - text_cursor,
                        leading,
                    });
                }

                // Convert the intervals of this line into clickable areas for the hyperlinks.
                for (href, node, start, end, down) in link_intervals.drain(..) {
                    if end <= start {
                        continue;
                    }
                    hyperlinks.0.push(Hyperlink {
                        page: line_pos.page,
                        rect: if flow.is_vertical() {
                            Rect {
                                x: line_pos.x + down,
                                y: line_pos.y + start,
                                width: line_pos.width,
                                height: end - start,
                            }
                        } else {
                            Rect {
                                x: line_pos.x + start,
                                y: line_pos.y + down,
                                width: end - start,
                                height: line.height,
                            }
                        },
                        href: href.to_owned(),
                        destination: None, // resolved later, once all positions are final
                        node,
                    });
                }
            }
            if open_tag.take().is_some() {
                ops.push(TextOperator::EndMarkedContent);
            }
            if let Some(pos) = paragraph_pos {
                entities
                    .build_entity()
                    .with(pos, &mut positions)
                    .with(
                        Paragraph {
                            start_pos: paragraph_start,
                            ops,
                            color_glyphs,
                        },
                        &mut paragraphs,
                    )
                    .build();
            }
        }
        if let Some(done) = previous {
            inline_formatting_contexts.get_mut(done).expect("the context exists").break_points = Vec::new();
        }
    }
}

/// The vertical offset from the baseline (positive is down) and the thickness of the lines drawn
/// for a text decoration, together with whether each of the lines is enabled.
fn decoration_lines(
    font_size: Px,
    decoration: crate::style::properties::specified::TextDecoration,
) -> [((Px, Px), bool); 3] {
    let thickness = (font_size * 0.06).max(Px(1.0));
    [
        (
            (font_size * 0.12 + thickness / 2.0, thickness),
            decoration.underline,
        ),
        ((-(font_size * 0.3), thickness), decoration.line_through),
        ((-(font_size * 0.8), thickness), decoration.overline),
    ]
}

/// The text of an inserted hyphen as far as extraction is concerned: a hyphen that exists only
/// because of a line break must not be taken for a hyphen of the word.
const SOFT_HYPHEN: &str = "\u{AD}";

/// The position of the pen along a paragraph while its break points are computed. A paragraph
/// can be tens of thousands of pixels long, where an `f32` is only precise to a few thousandths
/// of a pixel; summing the width of thousands of glyphs in it drifts by pixels (the same
/// glyphs round the same way every time). The sum is kept in `f64`; the break points get the
/// rounded position.
#[derive(Debug, Copy, Clone, Default)]
struct Pen(f64);

impl Pen {
    fn px(self) -> Px {
        Px(self.0 as f32)
    }
}

impl std::ops::AddAssign<Px> for Pen {
    fn add_assign(&mut self, width: Px) {
        self.0 += f64::from(width.0);
    }
}

impl std::ops::Add<Px> for Pen {
    type Output = Pen;

    fn add(self, width: Px) -> Pen {
        Pen(self.0 + f64::from(width.0))
    }
}

/// A text that is set on one line in one go.
pub(super) struct ShapedLine {
    /// The operators that draw the text, from the font and the color to the last glyph.
    pub ops: TextOps,
    pub width: Px,
    /// The colour glyphs of the text, relative to the start of the text on the baseline.
    pub color_glyphs: Vec<PlacedColorGlyph>,
}

/// Sets `text` in the face `face` on one line: the text is reordered by the bidirectional
/// algorithm (`rtl` is the direction of the paragraph), every run is shaped in its direction and
/// the runs are drawn in visual order. The markers of list items and the margin boxes of pages
/// are set like this.
pub(super) fn shape_line(text: &str, font: &Font, face: &components::TextFace, rtl: bool) -> ShapedLine {
    use unicode_bidi::{BidiInfo, Level};

    let mut ops = TextOps::from_ops(vec![
        TextOperator::SetFont(face.font_name.clone(), face.font_size),
        TextOperator::SetColor(face.color),
    ]);
    let info = BidiInfo::new(text, Some(if rtl { Level::rtl() } else { Level::ltr() }));
    let mut width = Px(0.0);
    let mut color_glyphs = Vec::new();
    for paragraph in &info.paragraphs {
        let (levels, runs) = info.visual_runs(paragraph, paragraph.range.clone());
        for run in runs {
            let ops_before = ops.len();
            let run_width = shape_piece(
                &text[run.clone()],
                font,
                &face.fallback,
                Some(run.start),
                face.font_size,
                Px(0.0),
                &[],
                levels[run.start].is_rtl(),
                &mut ops,
            );
            color_glyphs.extend(take_color_glyphs(&mut ops, ops_before, (width, Px(0.0)), face.color));
            width += run_width;
        }
    }
    ShapedLine { ops, width, color_glyphs }
}

/// Draws `characters`, which start at the byte `at` of the text that `fonts` is for (if they are
/// part of it), with `font` (and its fallback fonts) by appending the text operators to
/// `ops` and returns the width of the text. `word_spacing` is added to every space and in front
/// of the characters at the byte offsets `inter_letter` (see `BreakPoint::inter_letter`), which
/// are inside of `characters`. Right-to-left text (`rtl`) is shaped right-to-left and its glyphs
/// are drawn in reverse logical order.
pub(super) fn shape_piece(
    characters: &str,
    font: &Font,
    fonts: &crate::layout::text::fallback::TextFonts,
    at: Option<usize>,
    font_size: Px,
    word_spacing: Px,
    inter_letter: &[usize],
    rtl: bool,
    ops: &mut TextOps,
) -> Px {
    shape_piece_in_flow(characters, font, fonts, at, font_size, word_spacing, inter_letter, rtl, None, None, ops)
}

/// Where a piece of a vertical line is on the page, to draw its upright characters.
#[derive(Debug, Copy, Clone)]
pub(super) struct VerticalPiece {
    pub flow: super::writing_mode::Flow,
    /// The physical `x` of the baseline of the line.
    pub baseline_x: Px,
    /// The physical `y` of the start of the piece.
    pub start_y: Px,
}

/// The distance of the center of the em box of an ideograph above its baseline, in em.
/// (The em box of the fonts of the CJK scripts goes from 0.12 em below the baseline to 0.88 em
/// above it.)
const IDEOGRAPHIC_CENTER: f32 = 0.38;

/// Like [`shape_piece`]. In a vertical line (`vertical`), the characters that stand upright are
/// drawn with a text matrix of their own, whose origin puts the center of their em box on the
/// center of the line, while the others are turned with the line.
///
/// A `hyphen` is the hyphen of a line that breaks inside of a word (see `hyphen_character`),
/// drawn behind the characters: it is shaped with them, so that it is kerned with the letters in
/// front of it, and is part of the width that is returned. It is not part of the text: the
/// glyph stands for a soft hyphen.
pub(super) fn shape_piece_in_flow(
    characters: &str,
    font: &Font,
    fonts: &crate::layout::text::fallback::TextFonts,
    at: Option<usize>,
    font_size: Px,
    word_spacing: Px,
    inter_letter: &[usize],
    rtl: bool,
    hyphen: Option<char>,
    vertical: Option<VerticalPiece>,
    ops: &mut TextOps,
) -> Px {
    // the characters and the hyphen: the offsets of the clusters are in this text
    let mut with_hyphen = String::new();
    let (characters_end, text) = match hyphen {
        Some(hyphen) => {
            with_hyphen.reserve(characters.len() + hyphen.len_utf8());
            with_hyphen.push_str(characters);
            with_hyphen.push(hyphen);
            (characters.len(), with_hyphen.as_str())
        }
        None => (characters.len(), characters),
    };
    let is_hyphen = |cluster: usize| hyphen.is_some() && cluster >= characters_end;
    // lines that run upwards or right-to-left in a vertical flow are not supported
    let vertical = vertical.filter(|_| !rtl);
    let is_upright = |cluster: usize| {
        vertical.is_some()
            && fonts.orientation.is_some_and(|orientation| {
                text[cluster..]
                    .chars()
                    .next()
                    .is_some_and(|ch| super::writing_mode::is_upright(ch, orientation))
            })
    };
    let is_invisible = |info: &super::text::GlyphInfo| {
        text[info.cluster as usize..]
            .chars()
            .next()
            .is_some_and(is_invisible_formatting)
    };
    // the glyphs go to a buffer that is reused for the next piece
    let mut shaped = SHAPED_SCRATCH.with(|scratch| std::mem::take(&mut *scratch.borrow_mut()));
    shaped.clear();
    match hyphen {
        Some(hyphen) => shape_hyphenated_into(characters, hyphen, font, fonts, at, rtl, &mut shaped),
        None => shape_directed_into(characters, font, fonts, at, rtl, &mut shaped),
    }
    // (an invisible character can have an advance, from `letter-spacing`: it makes the room for
    // the edges of an inline box)
    let invisible_advance: f64 = shaped.iter().filter(|(_, info)| is_invisible(info)).map(|(pos, _)| pos.x_advance.0).sum();
    shaped.retain(|(_, info)| !is_invisible(info));

    // Glyphs of characters that `font` has no glyph for come from its fallback fonts. The font
    // is switched for them and switched back to `font` at the end of the piece.
    let fallbacks = font.fallbacks(&fonts.key);
    let font_at = |index: u16| -> &Font {
        match index {
            0 => font,
            n => &fallbacks[n as usize - 1],
        }
    };

    // the glyphs of this piece start here: they are the ones that the piece's text is for
    let first_glyph = ops.glyphs.len();

    let word_spacing_units = FontUnit((word_spacing / font_size.0).0 as f64 * 1000.0);

    // First the glyphs are placed in visual order (left to right) to find where each is drawn.
    struct Placed {
        glyph: GlyphIndex,
        font: u16,
        cluster: usize,
        /// Where the pen is when the glyph is drawn, relative to the start of the piece.
        x: FontUnit,
        rise: FontUnit,
    }
    let mut placed: Vec<Placed> = Vec::with_capacity(shaped.len());
    let mut advance = Px(0.0);
    let mut pen = FontUnit(0.0);
    let mut previous_cluster: Option<usize> = None;
    let visual_order: Box<dyn Iterator<Item = &(_, _)>> = if rtl {
        Box::new(shaped.iter().rev())
    } else {
        Box::new(shaped.iter())
    };
    for (pos, glyph_info) in visual_order {
        // The gap in front of the character at an inter-letter break is between this glyph and
        // the one before it (on the left), whichever the direction of the text.
        let cluster = glyph_info.cluster as usize;
        if let Some(previous) = previous_cluster.filter(|_| word_spacing != Px(0.0)) {
            let (low, high) = (previous.min(cluster), previous.max(cluster));
            let gaps = inter_letter
                .iter()
                .filter(|&&offset| low < offset && offset <= high)
                .count();
            for _ in 0..gaps {
                advance += word_spacing;
                pen.0 += word_spacing_units.0;
            }
        }
        previous_cluster = Some(cluster);

        advance += font_at(glyph_info.font).scale_to_px(pos.x_advance, font_size);
        placed.push(Placed {
            glyph: GlyphIndex(glyph_info.codepoint),
            font: glyph_info.font,
            cluster,
            x: FontUnit(pen.0 + pos.x_offset.0),
            rise: pos.y_offset,
        });
        pen.0 += pos.x_advance.0;
        // adjust the width of space glyphs to allow text justification
        if text[cluster..].starts_with(char::is_whitespace) {
            advance += word_spacing;
            pen.0 += word_spacing_units.0;
        }
    }
    if invisible_advance != 0.0 {
        advance += font.scale_to_px(FontUnit(invisible_advance), font_size);
        pen.0 += invisible_advance;
    }
    let piece_end = pen;

    // The glyphs are written in visual order, and the piece says in which direction its text
    // goes. Each kern moves the pen to where its glyph is drawn.
    // The clusters of the glyphs are offsets in the text of their piece: the hyphen is a piece
    // of its own that starts at 0.
    let piece_cluster = |cluster: usize| (if is_hyphen(cluster) { cluster - characters_end } else { cluster }) as u32;
    let mut out = GlyphRunWriter::new(ops);
    let mut current_font = 0u16;
    let mut rise = FontUnit(0.0);
    // where the pen is after the last glyph
    let mut pen = FontUnit(0.0);
    for glyph in &placed {
        if glyph.font != current_font {
            out.flush(ops);
            current_font = glyph.font;
            ops.push(TextOperator::SetFont(
                font_at(current_font).postscript_name(),
                font_size,
            ));
        }
        let glyph_font = font_at(current_font);

        if let (Some(piece), true) = (vertical, is_upright(glyph.cluster)) {
            out.flush(ops);
            let advance = glyph_font.advance(glyph.glyph);
            let size = font_size.0;
            let advance_px = advance.0 as f32 * size / 1000.0;
            let along = piece.start_y.0 + glyph.x.0 as f32 * size / 1000.0;
            // the center of the em box goes to the center of the line, on the baseline plus
            // the distance of the center of the em box above it
            let center_x = piece.baseline_x.0 + IDEOGRAPHIC_CENTER * size;
            ops.push(TextOperator::SetTransform(components::TransformMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: Px(center_x - advance_px / 2.0),
                f: Px(along + advance_px / 2.0 + IDEOGRAPHIC_CENTER * size),
            }));
            ops.push_glyph(glyph.glyph, piece_cluster(glyph.cluster));
            out.skip_pushed(ops);
            // back to the line, behind the glyph
            let [a, b, c, d] = piece.flow.text_matrix();
            pen = FontUnit(glyph.x.0 + advance.0);
            ops.push(TextOperator::SetTransform(components::TransformMatrix {
                a,
                b,
                c,
                d,
                e: piece.baseline_x,
                f: Px(piece.start_y.0 + pen.0 as f32 * size / 1000.0),
            }));
            continue;
        }

        // In `TJ`, a positive kern moves the pen to the left.
        out.kern(FontUnit(pen.0 - glyph.x.0));
        // fonts have rounding noise in their anchors; below 1/1000 em a rise is invisible
        if (glyph.rise - rise).0.abs() >= 1.0 {
            out.flush(ops);
            rise = glyph.rise;
            ops.push(TextOperator::SetRise(font_size * (rise.0 as f32 / 1000.0)));
        }
        out.glyph(ops, glyph.glyph, piece_cluster(glyph.cluster));
        if vertical.is_none() && glyph_font.is_color_glyph(glyph.glyph) {
            ops.push(TextOperator::ColorGlyph {
                font: glyph_font.postscript_name(),
                glyph: glyph.glyph,
                size: font_size,
                dx: Px(glyph.x.0 as f32 * font_size.0 / 1000.0),
                rise: Px(glyph.rise.0 as f32 * font_size.0 / 1000.0),
            });
        }
        pen = FontUnit(glyph.x.0 + glyph_font.advance(glyph.glyph).0);
    }
    // the text that follows continues from the end of the piece on the baseline
    out.kern(FontUnit(pen.0 - piece_end.0));
    out.flush(ops);
    // The glyphs of the hyphen are at the end of the glyphs of the piece, or at its start in
    // right-to-left text. They are a piece of their own that stands for a soft hyphen.
    let hyphen_glyphs = placed.iter().filter(|glyph| is_hyphen(glyph.cluster)).count();
    let glyphs = first_glyph..ops.glyphs.len();
    let split = if rtl { glyphs.start + hyphen_glyphs } else { glyphs.end - hyphen_glyphs };
    if rtl {
        ops.push_piece(glyphs.start..split, SOFT_HYPHEN, true, PieceKind::Inserted);
        ops.push_piece(split..glyphs.end, characters, true, PieceKind::Shaped);
    } else {
        ops.push_piece(glyphs.start..split, characters, false, PieceKind::Shaped);
        ops.push_piece(split..glyphs.end, SOFT_HYPHEN, false, PieceKind::Inserted);
    }
    if rise.0 != 0.0 {
        ops.push(TextOperator::SetRise(Px(0.0)));
    }
    if current_font != 0 {
        ops.push(TextOperator::SetFont(font.postscript_name(), font_size));
    }

    SHAPED_SCRATCH.with(|scratch| *scratch.borrow_mut() = shaped);
    advance
}

/// Collects glyphs and the kerns between them into `TJ` operators. The glyphs are appended to
/// `TextOps::glyphs` as they come, and `flush` ends the operator that they make up.
struct GlyphRunWriter {
    /// Where the glyphs of the operator that is being collected start.
    start: usize,
    /// The kern that goes in front of the next glyph, summed up so that a glyph gets one kern.
    pending_kern: FontUnit,
}

impl GlyphRunWriter {
    fn new(ops: &TextOps) -> Self {
        GlyphRunWriter { start: ops.glyphs.len(), pending_kern: FontUnit(0.0) }
    }

    fn kern(&mut self, kern: FontUnit) {
        self.pending_kern.0 += kern.0;
    }

    /// The pending kern in tenths of a font unit, which is reset: 0.1/1000 em is far below
    /// anything visible; the coarser values compress better.
    fn take_kern(&mut self) -> i32 {
        let tenths = (self.pending_kern.0 * 10.0).round();
        self.pending_kern = FontUnit(0.0);
        if tenths.abs() >= 1.0 { tenths as i32 } else { 0 }
    }

    fn glyph(&mut self, ops: &mut TextOps, glyph: GlyphIndex, cluster: u32) {
        let kern = self.take_kern();
        ops.glyphs.push(KernedGlyph { glyph, kern, cluster });
    }

    /// Continues after glyphs that were pushed to `ops` without this writer.
    fn skip_pushed(&mut self, ops: &TextOps) {
        self.start = ops.glyphs.len();
    }

    /// Ends the `TJ` operator of the collected glyphs and kerns and appends it to `ops`.
    fn flush(&mut self, ops: &mut TextOps) {
        let trailing_kern = self.take_kern();
        let end = ops.glyphs.len();
        if end > self.start || trailing_kern != 0 {
            ops.push(TextOperator::KernedGlyphs(GlyphRange {
                start: self.start as u32,
                end: end as u32,
                trailing_kern,
            }));
        }
        self.start = end;
    }
}

thread_local! {
    /// The glyphs of the piece that `shape_piece_in_flow` is shaping.
    static SHAPED_SCRATCH: std::cell::RefCell<Vec<(super::text::GlyphPosition, super::text::GlyphInfo)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::properties::computed::Language as Lang;

    /// The operators of `ops` as the PDF writer gets them: with the `/ActualText` that the text
    /// of the glyphs needs.
    fn marked(ops: &TextOps) -> Vec<TextOperator> {
        let mut usage = crate::font_usage::FontUsage::default();
        usage.record_text(ops);
        crate::pdf::text::with_actual_text(ops, &usage).unwrap_or_else(|| ops.iter().cloned().collect())
    }

    /// Glyph clusters that a map from glyph to text cannot describe get `/ActualText`; the
    /// others go into the map, where a glyph keeps the text of its first use.
    #[test]
    fn ambiguous_clusters_get_actual_text() {
        use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};
        let font = crate::layout::text::font::system_font(
            &[FontFamily::Named("TeX Gyre Termes".into())],
            &FontWeight::NORMAL,
            &FontStyle::Normal,
        )
        .unwrap();
        let glyph = |ch: char| GlyphIndex(font.hb_font.get_nominal_glyph(ch).unwrap());
        // "a" one glyph; "b" drawn with the glyph of "a"; "c" with two glyphs; "d" one glyph
        let mut ops = TextOps::from_ops(vec![TextOperator::SetFont(font.postscript_name(), Px(16.0))]);
        let first = ops.glyphs.len();
        for (ch, cluster) in [('a', 0), ('a', 1), ('c', 2), ('c', 2), ('d', 3)] {
            ops.glyphs.push(KernedGlyph { glyph: glyph(ch), kern: 0, cluster });
        }
        ops.push(TextOperator::KernedGlyphs(GlyphRange { start: first as u32, end: 5, trailing_kern: 0 }));
        ops.push_piece(first..ops.glyphs.len(), "abcd", false, PieceKind::Shaped);

        assert_eq!(actual_texts(&ops), ["b", "c"]);
        assert_balanced(&marked(&ops));

        let mut usage = crate::font_usage::FontUsage::default();
        usage.record_text(&ops);
        let usage = usage.font(&font.postscript_name()).unwrap();
        assert_eq!(usage.text(glyph('a')), Some("a"));
        assert_eq!(usage.text(glyph('d')), Some("d"));
        // a glyph without a text of its own gets the first text it was drawn for in a cluster
        assert_eq!(usage.text(glyph('c')), None);
        assert_eq!(usage.fallback_text(glyph('c')), Some("c"));
        assert_eq!(usage.fallback_text(glyph('a')), Some("b"));
    }

    fn font_named(family: &str) -> Font {
        use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};
        crate::layout::text::font::system_font(
            &[FontFamily::Named(family.into())],
            &FontWeight::NORMAL,
            &FontStyle::Normal,
        )
        .unwrap()
    }

    /// Operators that select `font`, as the layout starts every line.
    fn ops_for(font: &Font) -> TextOps {
        TextOps::from_ops(vec![TextOperator::SetFont(font.postscript_name(), Px(16.0))])
    }

    /// The operators `shape_piece` emits for `text`, with the width it returns.
    fn pieces(family: &str, text: &str, rtl: bool, word_spacing: Px) -> (TextOps, Px, Font) {
        let font = font_named(family);
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut ops = ops_for(&font);
        let width = shape_piece(text, &font, &fonts, None, Px(16.0), word_spacing, &[], rtl, &mut ops);
        (ops, width, font)
    }

    /// The `/ActualText` of the marked content that the PDF writer makes for `ops`.
    fn actual_texts(ops: &TextOps) -> Vec<String> {
        marked(ops)
            .into_iter()
            .filter_map(|op| match op {
                TextOperator::BeginActualText(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Marked content is not nested and ends before the operators of the piece do.
    fn assert_balanced(ops: &[TextOperator]) {
        let mut open = false;
        for op in ops {
            match op {
                TextOperator::BeginActualText(_) | TextOperator::BeginArtifact => {
                    assert!(!open, "nested marked content: {ops:?}");
                    open = true;
                }
                TextOperator::EndMarkedContent => {
                    assert!(open, "unmatched end of marked content: {ops:?}");
                    open = false;
                }
                _ => {}
            }
        }
        assert!(!open, "marked content is not ended: {ops:?}");
    }

    /// The glyphs of the `TJ` operators, in order.
    fn glyph_count(ops: &TextOps) -> usize {
        ops.iter()
            .map(|op| match op {
                TextOperator::KernedGlyphs(range) => (range.end - range.start) as usize,
                _ => 0,
            })
            .sum()
    }

    /// The pen has to end where the returned width says it does: the glyph advances minus
    /// the kerns of the `TJ` operators is the width of the piece.
    fn net_advance(ops: &TextOps, font: &Font, size: f32) -> f32 {
        let mut total = 0.0;
        for op in ops.iter() {
            if let TextOperator::KernedGlyphs(range) = op {
                range.for_each_item(&ops.glyphs, |item| match item {
                    components::KernedItem::Glyphs(glyphs) => {
                        total += glyphs.iter().map(|g| font.advance(g.glyph).0 as f32).sum::<f32>()
                    }
                    components::KernedItem::Kern(kern) => total -= kern.0 as f32,
                });
            }
        }
        total * size / 1000.0
    }

    #[test]
    fn plain_text_has_no_marked_content() {
        let (ops, _, _) = pieces("TeX Gyre Termes", "Hello, world", false, Px(0.0));
        assert!(actual_texts(&ops).is_empty(), "{ops:?}");
        assert!(!marked(&ops).iter().any(|op| matches!(op, TextOperator::EndMarkedContent)));
        assert_eq!(glyph_count(&ops), "Hello, world".len());
    }

    /// The i-matra of "कि" comes first in the glyphs and is drawn before the consonant. Only
    /// that cluster is marked; the space between the clusters is not.
    #[test]
    fn reordered_clusters_are_wrapped_in_actual_text() {
        let (ops, _, _) = pieces("Noto Sans Devanagari", "कि पि", false, Px(0.0));
        assert_eq!(actual_texts(&ops), ["कि", "पि"]);
        let ops = marked(&ops);
        assert_balanced(&ops);
        // each cluster's glyphs are inside of its marked content, so there are at least as many
        // `TJ` operators as clusters and the first operator is the start of the first span
        let begin = ops.iter().position(|op| matches!(op, TextOperator::BeginActualText(_))).unwrap();
        let end = ops.iter().position(|op| matches!(op, TextOperator::EndMarkedContent)).unwrap();
        assert!(
            ops[begin..end].iter().any(|op| matches!(op, TextOperator::KernedGlyphs(_))),
            "{ops:?}"
        );
    }

    /// The marking of a piece does not depend on the text that comes before it in the document:
    /// the same text gets the same marked content whether or not the fonts have seen it.
    #[test]
    fn marking_does_not_depend_on_earlier_text() {
        let font = font_named("Noto Sans Devanagari");
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut first = ops_for(&font);
        let mut second = ops_for(&font);
        for ops in [&mut first, &mut second] {
            shape_piece("कि", &font, &fonts, None, Px(16.0), Px(0.0), &[], false, ops);
        }
        let mut alone = crate::font_usage::FontUsage::default();
        alone.record_text(&first);
        let mut both = crate::font_usage::FontUsage::default();
        both.record_text(&first);
        both.record_text(&second);
        assert_eq!(
            format!("{:?}", crate::pdf::text::with_actual_text(&first, &alone)),
            format!("{:?}", crate::pdf::text::with_actual_text(&second, &both)),
        );
    }

    /// Right-to-left text, whose glyphs are in visual order, is marked with its text in logical
    /// order as a whole, marks and spaces included.
    #[test]
    fn right_to_left_pieces_get_their_logical_text() {
        for text in ["בְּרֵאשִׁית בָּרָא", "אב"] {
            let (ops, _, _) = pieces("Noto Sans Hebrew", text, true, Px(0.0));
            assert_eq!(actual_texts(&ops), [text], "{ops:?}");
            assert_balanced(&marked(&ops));
        }
    }

    /// A glyph cluster of several grapheme clusters (a Kannada conjunct with its vowel sign, a
    /// Sinhala yansaya with its vowel sign) is marked with the text of all of them.
    #[test]
    fn actual_text_covers_the_whole_glyph_cluster() {
        let (ops, _, _) = pieces("Noto Sans Kannada", "ಎಲ್ಲಾ", false, Px(0.0));
        assert_eq!(actual_texts(&ops).concat(), "ಲ್ಲಾ", "{ops:?}");
        assert_balanced(&marked(&ops));
        let (ops, _, _) = pieces("Noto Sans Sinhala", "ක්\u{200D}රියා", false, Px(0.0));
        let texts = actual_texts(&ops);
        assert!(texts.iter().any(|text| text == "ක්\u{200D}රි"), "{texts:?}");
        assert_balanced(&marked(&ops));
    }

    #[test]
    fn two_part_vowel_signs_are_marked() {
        let (ops, _, _) = pieces("Noto Sans Bengali", "কো", false, Px(0.0));
        assert_eq!(actual_texts(&ops), ["কো"]);
        assert_balanced(&marked(&ops));
    }

    /// The glyphs are written left to right whichever direction the text has, and the kerns put
    /// the pen at the end of the piece.
    #[test]
    fn pen_ends_at_the_end_of_the_piece() {
        for (family, text, rtl, spacing) in [
            ("TeX Gyre Termes", "Hello world", false, Px(0.0)),
            ("TeX Gyre Termes", "Hello world", false, Px(3.0)),
            ("Noto Sans Hebrew", "בראשית ברא", true, Px(0.0)),
            ("Noto Sans Hebrew", "בְּרֵאשִׁית ברא", true, Px(2.5)),
        ] {
            let (ops, width, font) = pieces(family, text, rtl, spacing);
            let net = net_advance(&ops, &font, 16.0);
            assert!(
                (net - width.0).abs() < 0.05,
                "{family} {text:?}: net advance {net} but width {width:?}"
            );
        }
    }

    /// Right-to-left text is stored in visual order: the first glyph is the last letter.
    #[test]
    fn right_to_left_glyphs_are_in_visual_order() {
        let font = font_named("Noto Sans Hebrew");
        let (ops, _, _) = pieces("Noto Sans Hebrew", "אב", true, Px(0.0));
        let glyphs: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                TextOperator::KernedGlyphs(range) => Some(&ops.glyphs[range.start as usize..range.end as usize]),
                _ => None,
            })
            .flatten()
            .map(|g| g.glyph)
            .collect();
        let expected = [font.hb_font.get_nominal_glyph('ב').unwrap(), font.hb_font.get_nominal_glyph('א').unwrap()];
        assert_eq!(glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn hyphenation_leaves_two_characters_on_both_sides() {
        // a-ka-de-mi-a: no break after the first or in front of the last letter
        assert_eq!(limit_hyphenation_chars("akademia", vec![1, 3, 5, 7]), vec![3, 5]);
        // short words are not hyphenated at all
        assert_eq!(limit_hyphenation_chars("akme", vec![2]), Vec::<usize>::new());
        // the counts are in characters, not bytes: ά is two bytes
        assert_eq!(limit_hyphenation_chars("άκρας", vec![2, 4]), vec![4]);
    }

    #[test]
    fn languages_get_their_hyphenation_dictionaries() {
        let language = |tag: &str| hyphenation_language(&Lang::from_tag(tag));
        assert_eq!(hyphenation_language(&Lang::default()), Some(Language::EnglishUS));
        assert_eq!(language("en"), Some(Language::EnglishUS));
        assert_eq!(language("en-GB"), Some(Language::EnglishGB));
        assert_eq!(language("de"), Some(Language::German1996));
        assert_eq!(language("de-CH"), Some(Language::GermanSwiss));
        assert_eq!(language("fr-CA"), Some(Language::French));
        assert_eq!(language("sr-Latn"), None);
        assert_eq!(language("ja"), None);
    }

    fn breaks(text: &str) -> Vec<usize> {
        break_opportunities_between_letters(text).collect()
    }

    #[test]
    fn ideographs_can_be_broken_between() {
        // every character is 3 bytes long
        assert_eq!(breaks("道可道"), vec![3, 6]);
    }

    #[test]
    fn closing_punctuation_does_not_start_a_line() {
        // no break in front of the comma or the full stop
        assert_eq!(breaks("道，非。名"), vec![6, 12]);
        // no break after the opening bracket
        assert_eq!(breaks("「道」"), Vec::<usize>::new());
    }

    #[test]
    fn thai_words_are_found_with_a_dictionary() {
        // ภาษา | ไทย | เป็น | ภาษา | ที่ (every character is 3 bytes long, the vowel signs too)
        let text = "ภาษาไทยเป็นภาษาที่";
        let breaks = breaks(text);
        assert!(breaks.contains(&12), "{breaks:?}");
        assert!(breaks.len() >= 3, "{breaks:?}");
        // never inside a cluster: no break in front of a vowel or tone mark
        for index in breaks {
            let next = text[index..].chars().next().unwrap();
            assert!(!matches!(next, '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}'));
        }
    }

    #[test]
    fn breaks_after_spaces_are_not_included() {
        assert!(breaks("a b c").is_empty());
        assert!(breaks("道 道").is_empty());
    }

    #[test]
    fn latin_words_break_after_hyphens_without_stretching() {
        assert_eq!(breaks("well-known"), vec![5]);
        assert!(!is_justified_between("well-known", 5));
    }

    #[test]
    fn gaps_next_to_cjk_characters_are_stretched() {
        assert!(is_justified_between("道可", 3));
        assert!(is_justified_between("道a", 3));
        assert!(is_justified_between("かな", 3));
        assert!(is_justified_between("한국", 3));
    }

    /// The operators `shape_piece_in_flow` emits for `text` with a hyphen after it.
    fn hyphenated(family: &str, text: &str, rtl: bool) -> (TextOps, Px, Font) {
        let font = font_named(family);
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut ops = ops_for(&font);
        let width = shape_piece_in_flow(
            text, &font, &fonts, Some(0), Px(16.0), Px(0.0), &[], rtl, Some('\u{2010}'), None, &mut ops,
        );
        (ops, width, font)
    }

    /// The hyphen is part of the shaped text but not of the text of the word: it is a piece of
    /// its own that stands for a soft hyphen, with its own cluster.
    #[test]
    fn the_hyphen_is_a_piece_that_stands_for_a_soft_hyphen() {
        let (ops, _, font) = hyphenated("TeX Gyre Termes", "Ty", false);
        let pieces: Vec<_> = ops
            .pieces
            .iter()
            .map(|piece| (ops.piece_text(piece), piece.kind, piece.glyphs.clone()))
            .collect();
        assert_eq!(
            pieces,
            [("Ty", PieceKind::Shaped, 0..2), (SOFT_HYPHEN, PieceKind::Inserted, 2..3)]
        );
        let clusters: Vec<u32> = ops.glyphs.iter().map(|glyph| glyph.cluster).collect();
        assert_eq!(clusters, [0, 1, 0]);
        // the glyph is the one of U+2010
        let hyphen = font.hb_font.get_nominal_glyph('\u{2010}').unwrap();
        assert_eq!(ops.glyphs[2].glyph, GlyphIndex(hyphen));
        assert_eq!(actual_texts(&ops), [SOFT_HYPHEN]);
        assert_balanced(&marked(&ops));
    }

    /// The width that is returned and the pen after the text include the hyphen.
    #[test]
    fn the_width_of_a_hyphenated_piece_includes_the_hyphen() {
        let (plain, plain_width, _) = pieces("TeX Gyre Termes", "Ty", false, Px(0.0));
        let (ops, width, font) = hyphenated("TeX Gyre Termes", "Ty", false);
        assert_eq!(glyph_count(&ops), glyph_count(&plain) + 1);
        assert!(width > plain_width);
        assert!((net_advance(&ops, &font, 16.0) - width.0).abs() < 0.01);
    }

    /// In right-to-left text the hyphen is drawn first, on the left; its piece comes first too.
    #[test]
    fn the_hyphen_of_right_to_left_text_is_on_the_left() {
        let (ops, _, _) = hyphenated("Noto Sans Hebrew", "אב", true);
        let pieces: Vec<_> = ops
            .pieces
            .iter()
            .map(|piece| (ops.piece_text(piece), piece.kind, piece.glyphs.clone(), piece.rtl))
            .collect();
        assert_eq!(
            pieces,
            [
                (SOFT_HYPHEN, PieceKind::Inserted, 0..1, true),
                ("אב", PieceKind::Shaped, 1..3, true)
            ]
        );
        // the text of the word is marked as a whole, and the hyphen is a soft hyphen
        assert_eq!(actual_texts(&ops), [SOFT_HYPHEN, "אב"]);
        assert_balanced(&marked(&ops));
    }

    /// A hyphen that was inserted does not say what text its glyph stands for in the font:
    /// hyphens that the author typed keep the text of the glyph, and need no `/ActualText`.
    #[test]
    fn an_inserted_hyphen_does_not_change_the_text_of_the_glyph() {
        let (inserted, _, font) = hyphenated("TeX Gyre Termes", "Ty", false);
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut typed = ops_for(&font);
        shape_piece("a\u{2010}b", &font, &fonts, None, Px(16.0), Px(0.0), &[], false, &mut typed);

        let mut usage = crate::font_usage::FontUsage::default();
        usage.record_text(&inserted);
        usage.record_text(&typed);
        let glyphs = usage.font(&font.postscript_name()).unwrap();
        let hyphen = GlyphIndex(font.hb_font.get_nominal_glyph('\u{2010}').unwrap());
        assert_eq!(glyphs.text(hyphen), Some("\u{2010}"));
        assert!(crate::pdf::text::with_actual_text(&typed, &usage).is_none());
        // the inserted one still says that it is a soft hyphen
        assert!(crate::pdf::text::with_actual_text(&inserted, &usage).is_some());
    }
}
