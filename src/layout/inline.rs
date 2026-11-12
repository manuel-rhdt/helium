use super::components::{self, EntityHrefs, Hyperlink, Hyperlinks, Paragraph, TextOperator};
use crate::tagging::{ContentTag, StructNodeId, StructureTree};
use super::{
    FontMap,
    text::{Font, font::GlyphIndex},
};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px, RGBA, TextAlign};
use crate::style::properties::specified::{Direction, UnicodeBidi};

use crate::display_list::Rect;
use crate::layout::text::{is_invisible_formatting, shape, shape_directed, shape_with_levels};

use components::{Glyphs, GraphicsOp, IntrinsicSize, KernedGlyphs, Position};
use hyphenation::{Hyphenator, Language, Load, Standard};
use lemon_pdf::font::FontUnit;
use specs::storage::GenericReadStorage;
use specs::{Component, DenseVecStorage, Entity};
use specs::{Entities, Join, Read, ReadStorage, System, Write, WriteStorage};

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
    /// Whether the base direction is found from the text (`unicode-bidi: plaintext`) instead of
    /// `direction`.
    pub plaintext: bool,
    /// Whether the (first) paragraph is right-to-left, set by `FindPossibleLineBreaks`.
    pub base_rtl: bool,
    /// The runs of the paragraph with the same bidi embedding level in logical order, set by
    /// `FindPossibleLineBreaks`. Empty if all of the text is left-to-right.
    pub bidi_runs: Vec<BidiRun>,
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
    pub fn new(elements: Vec<Entity>, absolutes: Vec<Entity>, style: &ComputedValues) -> Self {
        InlineFormattingContext {
            text_align: style.text.text_align,
            elements,
            absolutes,
            break_points: vec![],
            lines: vec![],
            atom_offsets: vec![],
            direction: style.text.direction,
            plaintext: style.box_.unicode_bidi == UnicodeBidi::Plaintext,
            base_rtl: style.text.direction == Direction::Rtl,
            bidi_runs: vec![],
        }
    }

    /// The extra space after every space on a justified line (all but the last line of a
    /// paragraph with `text-align: justify`), given the width of the line box.
    pub fn word_spacing(&self, line_num: usize, line_width: Px) -> Px {
        let line = &self.lines[line_num];
        let num_spaces = self.space_count(line.from_break, line.to_break);
        let is_last_line = line_num == self.lines.len() - 1;
        if !is_last_line && self.text_align == TextAlign::Justify && num_spaces > 0 {
            (line_width - self.content_width(line.from_break, line.to_break)) / num_spaces as f32
        } else {
            Px(0.0)
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
        let position = |entity| self.elements.iter().position(|&e| e == entity);
        let (Some(first), Some(last)) = (position(from_break.entity), position(to_break.entity))
        else {
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
                    let start = if element == first {
                        from_break.char_index
                    } else {
                        0
                    };
                    let mut end = if element == last {
                        to_break.char_index
                    } else {
                        text_run.text.len()
                    };
                    if element == last && to_break.space && start < end {
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
    ) -> Result<(), String> {
        let lines = self.compute_lines(line_width, indent, text_runs, intrinsic_sizes)?;
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

    /// Breaks the paragraph into lines without storing them or creating entities for them, so
    /// that it can be used to measure the paragraph for a given line width.
    pub fn compute_lines(
        &self,
        line_width: Px,
        indent: Px,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
    ) -> Result<Vec<MeasuredLine>, String> {
        let breakpoints = &self.break_points;
        if breakpoints.is_empty() {
            return Ok(vec![]);
        }
        if breakpoints.len() < 2 {
            return Err(format!(
                "inline context needs at least 2 break points, found {}",
                breakpoints.len()
            ));
        }

        let opportunities: Vec<_> = breakpoints
            .iter()
            .map(|breakpoint| super::paragraph::BreakOpportunity {
                forced: breakpoint.forced,
                hyphen: breakpoint.hyphen,
                penalty: breakpoint.penalty,
            })
            .collect();
        let mut glue_prefix = Vec::with_capacity(breakpoints.len() + 1);
        glue_prefix.push(super::paragraph::GlueTotals::default());
        for breakpoint in breakpoints {
            let previous = *glue_prefix.last().unwrap();
            glue_prefix.push(super::paragraph::GlueTotals {
                stretch: previous.stretch + f64::from(breakpoint.glue_stretch.0),
                shrink: previous.shrink + f64::from(breakpoint.glue_shrink.0),
            });
        }
        let sequence = super::paragraph::compute_paragraph_breakpoints(
            &opportunities,
            |from, to| {
                let natural_width = breakpoints[to].advance_before
                    - breakpoints[from].advance_after
                    + breakpoints[to].hyphen_width;
                let start_glue = glue_prefix[from + 1];
                let end_glue = glue_prefix[to];
                let glue = super::paragraph::GlueTotals {
                    stretch: end_glue.stretch - start_glue.stretch,
                    shrink: end_glue.shrink - start_glue.shrink,
                };
                Some((f64::from(natural_width.0), glue))
            },
            |from| f64::from((line_width - if from == 0 { indent } else { Px(0.0) }).0),
        );

        let lines = sequence
            .windows(2)
            .map(|window| {
                let (baseline, height) =
                    self.line_height(window[0], window[1], text_runs, intrinsic_sizes);
                if height == Px(0.0) {
                    return Err(format!(
                        "line between break points {} and {} has zero height",
                        window[0], window[1]
                    ));
                }
                Ok(MeasuredLine {
                    from_break: window[0],
                    to_break: window[1],
                    height,
                    baseline,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(lines)
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

    pub fn line_height(
        &self,
        from_index: usize,
        to_index: usize,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
    ) -> (Px, Px) {
        let from = self.break_points[from_index];
        let to = self.break_points[to_index];

        let start = self
            .elements
            .iter()
            .position(|&elem| elem == from.entity)
            .unwrap();
        let end = self
            .elements
            .iter()
            .position(|&elem| elem == to.entity)
            .unwrap();

        let (max_ascent, max_descent) = self.elements[start..=end]
            .iter()
            .map(|&ent| {
                if let Some(text_run) = text_runs.get(ent) {
                    (text_run.baseline, text_run.line_height - text_run.baseline)
                } else if let Some(&IntrinsicSize::SizeWithBaseline {
                    height, baseline, ..
                }) = intrinsic_sizes.get(ent)
                {
                    (baseline, height - baseline)
                } else {
                    log::error!("cannot layout element (entity {}) in line (every inline element must have an intrinsic size)", ent.id());
                    (Px(0.0), Px(0.0))
                }
            })
            .fold(
                (Px(0.0), Px(0.0)),
                |(height, depth), (new_height, new_depth)| {
                    (height.max(new_height), depth.max(new_depth))
                },
            );

        assert!(max_ascent + max_descent != Px(0.0));
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
                atoms
                    .iter()
                    .filter(|&&(atom, _)| atom <= element)
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

            let mut advance = Px(0.0);

            for (index, &entity) in elements.iter().enumerate() {
                let element_start = advance;
                if let Some(text_run) = text_runs.get_mut(entity) {
                    let font = &font_map[&text_run.font_name];
                    let levels = runs_of(index);
                    // the glyphs of the run as `(cluster, width)`, for the advances of the bidi
                    // runs
                    let mut glyph_widths: Vec<(usize, Px)> = Vec::new();
                    if computed_values.get(entity).is_some_and(|style| {
                        style.text.white_space
                            == crate::style::properties::specified::WhiteSpace::Pre
                    }) {
                        let mut segment_start = 0;
                        for (newline_index, _) in text_run.text.match_indices('\n') {
                            for (position, info) in shape_with_levels(
                                &text_run.text[segment_start..newline_index],
                                segment_start,
                                levels,
                                font,
                                &text_run.fallback,
                            ) {
                                let width =
                                    font.scale_to_px(position.x_advance, text_run.font_size);
                                glyph_widths.push((segment_start + info.cluster as usize, width));
                                advance += width;
                            }
                            let break_index = newline_index + 1;
                            break_points.push(BreakPoint {
                                entity,
                                char_index: break_index,
                                advance_before: advance,
                                advance_after: advance,
                                forced: true,
                                space: false,
                                inter_letter: false,
                                hyphen: false,
                                hyphen_width: Px(0.0),
                                glue_stretch: Px(0.0),
                                glue_shrink: Px(0.0),
                                penalty: -10_000,
                            });
                            segment_start = break_index;
                        }
                        for (position, info) in shape_with_levels(
                            &text_run.text[segment_start..],
                            segment_start,
                            levels,
                            font,
                            &text_run.fallback,
                        ) {
                            let width = font.scale_to_px(position.x_advance, text_run.font_size);
                            glyph_widths.push((segment_start + info.cluster as usize, width));
                            advance += width;
                        }
                        push_bidi_runs(bidi_runs, index, levels, &glyph_widths, element_start);
                        continue;
                    }
                    let shaped = shape_with_levels(&text_run.text, 0, levels, font, &text_run.fallback);
                    if !levels.is_empty() {
                        glyph_widths = shaped
                            .iter()
                            .map(|(position, info)| {
                                (
                                    info.cluster as usize,
                                    font.scale_to_px(position.x_advance, text_run.font_size),
                                )
                            })
                            .collect();
                    }
                    let run_start_advance = advance;
                    let mut other_breaks = break_opportunities_between_letters(&text_run.text);
                    for (pos, glyph_info) in &shaped {
                        let cluster = glyph_info.cluster as usize;
                        if other_breaks.peek() == Some(&cluster) {
                            other_breaks.next();
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
                                advance_before: advance,
                                advance_after: advance,
                                forced: false,
                                space: false,
                                inter_letter,
                                hyphen: false,
                                hyphen_width: Px(0.0),
                                glue_stretch: if inter_letter {
                                    text_run.font_size / 8.0
                                } else {
                                    Px(0.0)
                                },
                                glue_shrink: Px(0.0),
                                penalty: 0,
                            });
                        }
                        // a break inside a cluster (e.g. a ligature) is not possible
                        while other_breaks.peek().is_some_and(|&index| index <= cluster) {
                            other_breaks.next();
                        }
                        let cur_char = &text_run.text[glyph_info.cluster as usize..];
                        if cur_char.starts_with(|chr: char| chr.is_whitespace()) {
                            let advance_before = advance;
                            advance += font.scale_to_px(pos.x_advance, text_run.font_size);
                            let advance_after = advance;

                            let ws_len = cur_char.len() - cur_char.trim_start().len();

                            break_points.push(BreakPoint {
                                entity,
                                char_index: glyph_info.cluster as usize + ws_len,
                                advance_before,
                                advance_after,
                                forced: false,
                                space: true,
                                inter_letter: false,
                                hyphen: false,
                                hyphen_width: Px(0.0),
                                glue_stretch: Px(
                                    pos.x_advance.0 as f32 * text_run.font_size.0 / 2000.0
                                ),
                                glue_shrink: Px(
                                    pos.x_advance.0 as f32 * text_run.font_size.0 / 3000.0
                                ),
                                penalty: 0,
                            })
                        } else {
                            advance += font.scale_to_px(pos.x_advance, text_run.font_size);
                        }
                    }
                    let hyphenation = match text_run.hyphens {
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
                            match dictionaries.get(&text_run.lang) {
                                Some(dictionary) => word_ranges(&text_run.text)
                                    .into_iter()
                                    .flat_map(|(word_start, word_end)| {
                                        dictionary
                                            .hyphenate(&text_run.text[word_start..word_end])
                                            .breaks
                                            .into_iter()
                                            .map(move |offset| word_start + offset)
                                    })
                                    .collect(),
                                None => Vec::new(),
                            }
                        }
                    };
                    if !hyphenation.is_empty() {
                        let hyphen_width = shape("-", font, &text_run.fallback, None)
                            .iter()
                            .map(|(position, _)| {
                                font.scale_to_px(position.x_advance, text_run.font_size)
                            })
                            .fold(Px(0.0), |sum, width| sum + width);
                        // The width of the text in front of a glyph, from the shaping of the
                        // whole run above: `(cluster, width of all glyphs before it)`.
                        let mut prefix_widths = Vec::with_capacity(shaped.len());
                        let mut width = Px(0.0);
                        for (position, glyph_info) in &shaped {
                            prefix_widths.push((glyph_info.cluster as usize, width));
                            width += font.scale_to_px(position.x_advance, text_run.font_size);
                        }
                        let run_width = width;
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
                            let prefix_width = width_before(word_start)
                                + shape(&prefix[word_start..], font, &text_run.fallback, Some(word_start))
                                    .iter()
                                    .map(|(position, _)| {
                                        font.scale_to_px(position.x_advance, text_run.font_size)
                                    })
                                    .fold(Px(0.0), |sum, width| sum + width);
                            break_points.push(BreakPoint {
                                entity,
                                char_index,
                                advance_before: run_start_advance + prefix_width,
                                advance_after: run_start_advance + prefix_width,
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
                    push_bidi_runs(bidi_runs, index, levels, &glyph_widths, element_start);
                } else if let Some(intrinsic_size) = intrinsic_sizes.get(entity) {
                    advance += intrinsic_size.width().unwrap_or_default();
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start);
                    if let Some(run) = bidi_runs.last_mut().filter(|run| run.element == index) {
                        run.advance_end = advance;
                    }
                } else if computed_values
                    .get(entity)
                    .map_or(false, |style| style.box_.display.is_atomic_inline())
                {
                    // the width is only known once the box has been laid out
                    atom_offsets.push((entity, advance));
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start);
                } else {
                    push_bidi_runs(bidi_runs, index, runs_of(index), &[], element_start);
                }
                if let Some(style) = computed_values.get(entity) {
                    if style.text.line_break
                        == crate::style::properties::specified::LineBreak::Force
                    {
                        break_points.push(BreakPoint {
                            entity,
                            char_index: 0,
                            advance_before: advance,
                            advance_after: advance,
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
/// `hyphenation` crate, currently US English) are available.
#[derive(Default)]
struct HyphenationDictionaries(HashMap<Language, Option<Standard>>);

impl HyphenationDictionaries {
    /// The dictionary for text in `lang`. Text of an unknown language is hyphenated as English,
    /// text in a language without a dictionary not at all.
    fn get(&mut self, lang: &crate::style::properties::computed::Language) -> Option<&Standard> {
        let language = hyphenation_language(lang)?;
        self.0
            .entry(language)
            .or_insert_with(|| {
                Standard::from_embedded(language).ok().or_else(|| {
                    // British English is hyphenated as American English without its dictionary
                    (language == Language::EnglishGB)
                        .then(|| Standard::from_embedded(Language::EnglishUS).ok())
                        .flatten()
                })
            })
            .as_ref()
    }
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
    unicode_linebreak::linebreaks(text)
        .filter(move |&(index, opportunity)| {
            opportunity == unicode_linebreak::BreakOpportunity::Allowed
                && index < text.len()
                && !text[..index].ends_with(char::is_whitespace)
        })
        .map(|(index, _)| index)
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
    use unicode_linebreak::BreakClass::*;
    matches!(
        unicode_linebreak::break_property(ch as u32),
        Ideographic
            | ConditionalJapaneseStarter
            | HangulLvSyllable
            | HangulLvtSyllable
            | HangulLJamo
            | HangulVJamo
            | HangulTJamo
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
            let size = marker.font_size;
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
                            color: Some(marker.color),
                        },
                        &mut graphics,
                    )
                    .build();
                continue;
            }
            let font = &font_map[&marker.font_name];
            let mut ops = vec![
                TextOperator::SetFont(marker.font_name.clone(), marker.font_size),
                TextOperator::SetColor(marker.color),
            ];
            let width = shape_piece(
                &marker.text,
                font,
                &marker.fallback,
                Some(0),
                marker.font_size,
                Px(0.0),
                &[],
                false,
                &mut ops,
            );
            // the marker is the label (`Lbl`) of the list item
            if let Some(tag) = structure.content_tag(item, 0) {
                ops.insert(0, TextOperator::BeginTag(tag));
                ops.push(TextOperator::EndMarkedContent);
            }
            let gap = marker.font_size * 0.4;
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
                        start_pos: (item_x - gap - width, line_pos.y + baseline),
                        ops,
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
pub struct PlaceInlineBlocks;

impl<'a> System<'a> for PlaceInlineBlocks {
    type SystemData = (
        Read<'a, components::AbsoluteGroups>,
        ReadStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, IntrinsicSize>,
        WriteStorage<'a, Position>,
    );

    fn run(&mut self, (groups, ifcs, intrinsic_sizes, mut positions): Self::SystemData) {
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

                if !ifc.bidi_runs.is_empty() {
                    // the boxes are reordered with the text
                    for (atom, offset) in ifc.visual_atom_offsets(line_num, line_pos.width) {
                        let Some(&IntrinsicSize::SizeWithBaseline { baseline, .. }) =
                            intrinsic_sizes.get(atom)
                        else {
                            continue;
                        };
                        if !ifc.atom_offsets.iter().any(|&(entity, _)| entity == atom) {
                            continue;
                        }
                        let y = line_pos.y + line.baseline - baseline;
                        placements.insert(atom, (line_pos.x + offset, y, line_pos.page));
                    }
                    continue;
                }

                // the same word spacing as the line box creator uses
                let word_spacing = ifc.word_spacing(line_num, line_pos.width);

                for &(atom, offset) in &ifc.atom_offsets {
                    let (width, baseline) = match intrinsic_sizes.get(atom) {
                        Some(&IntrinsicSize::SizeWithBaseline {
                            width, baseline, ..
                        }) => (width, baseline),
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
                    let x =
                        line_pos.x + (offset - from.advance_after) + word_spacing * spaces as f32;
                    let y = line_pos.y + line.baseline - baseline;
                    placements.insert(atom, (x, y, line_pos.page));
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
            for &entity in &groups.groups[group].entities {
                if let Some(pos) = positions.get_mut(entity) {
                    pos.x += dx;
                    pos.y += dy;
                    pos.page = page;
                }
            }
            for (child, other) in groups.groups.iter().enumerate() {
                if other.parent != Some(group) || child == group {
                    continue;
                }
                if let Some(placement) = other.owner.and_then(|owner| placements.get_mut(&owner)) {
                    placement.0 += dx;
                    placement.1 += dy;
                    placement.2 = page;
                }
            }
        }
    }
}

pub struct LineBoxCreator;

impl<'a> System<'a> for LineBoxCreator {
    type SystemData = (
        Entities<'a>,
        ReadStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, components::TextRun>,
        ReadStorage<'a, components::IntrinsicSize>,
        ReadStorage<'a, ComputedValues>,
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
            inline_formatting_contexts,
            text_runs,
            intrinsic_sizes,
            computed_values,
            entity_hrefs,
            mut hyperlinks,
            mut positions,
            mut paragraphs,
            mut graphics,
            font_map,
            structure,
        ): Self::SystemData,
    ) {
        for ifc in inline_formatting_contexts.join() {
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

            let mut ops: Vec<TextOperator> = vec![];
            let mut previous_font: Option<(String, Px)> = None;
            let mut previous_color: Option<RGBA> = None;
            let mut paragraph_pos: Option<components::Position> = None;

            // position of the first baseline of the paragraph
            let mut paragraph_start = (Px(0.0), Px(0.0));

            // The x-intervals on the current line of the hyperlinks that have text on it, as
            // `(href, link node, start, end)`. Adjacent intervals of the same link are merged as
            // the pieces of the line are visited from left to right.
            let mut link_intervals: Vec<(&str, Option<StructNodeId>, Px, Px)> = Vec::new();

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

                // if we want to justify the text we have to add word-spacing
                let word_spacing = ifc.word_spacing(line_num, line_pos.width);

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
                                    ops: ops.clone(),
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

                // the x position on the line and the x position of the text line matrix
                let mut advance = ifc.line_start_offset(line_num, line_pos.width, word_spacing);
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
                        if previous_font != Some((run.font_name.clone(), run.font_size)) {
                            ops.push(TextOperator::SetFont(run.font_name.clone(), run.font_size));
                            previous_font = Some((run.font_name.clone(), run.font_size));
                        }

                        if previous_color != Some(run.color) {
                            ops.push(TextOperator::SetColor(run.color));
                            previous_color = Some(run.color);
                        }

                        let characters = &run.text[piece.start..piece.end];
                        let font = &font_map[&run.font_name];
                        let rtl = piece.is_rtl();

                        // the hyphen of a hyphenated word goes at its end
                        let hyphen = to_break.hyphen && Some(piece_index) == last_piece;
                        let mut piece_width = Px(0.0);
                        if hyphen && rtl {
                            ops.push(TextOperator::BeginActualText(SOFT_HYPHEN.to_owned()));
                            piece_width +=
                                shape_piece(
                                "-",
                                font,
                                &run.fallback,
                                None,
                                run.font_size,
                                Px(0.0),
                                &[],
                                true,
                                &mut ops,
                            );
                            ops.push(TextOperator::EndMarkedContent);
                        }
                        piece_width += shape_piece(
                            characters,
                            font,
                            &run.fallback,
                            Some(piece.start),
                            run.font_size,
                            word_spacing,
                            &ifc.inter_letter_offsets(line, &piece),
                            rtl,
                            &mut ops,
                        );
                        if hyphen && !rtl {
                            ops.push(TextOperator::BeginActualText(SOFT_HYPHEN.to_owned()));
                            piece_width +=
                                shape_piece(
                                "-",
                                font,
                                &run.fallback,
                                None,
                                run.font_size,
                                Px(0.0),
                                &[],
                                false,
                                &mut ops,
                            );
                            ops.push(TextOperator::EndMarkedContent);
                        }
                        if !run.decoration.is_none() && piece_width > Px(0.0) {
                            let page_pos = components::Position {
                                y: line_pos.y,
                                height: Px(0.0),
                                ..line_pos
                            };
                            let x = line_pos.x + advance;
                            let baseline = line_pos.y + line.baseline;
                            for ((offset, thickness), enabled) in
                                decoration_lines(run.font_size, run.decoration)
                            {
                                if !enabled {
                                    continue;
                                }
                                let y = baseline + offset;
                                entities
                                    .build_entity()
                                    .with(page_pos, &mut positions)
                                    .with(
                                        components::VectorGraphics {
                                            ops: vec![GraphicsOp::Line {
                                                from_x: x,
                                                from_y: y,
                                                to_x: x + piece_width,
                                                to_y: y,
                                                thickness,
                                            }],
                                            color: Some(run.color),
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
                                        x: line_pos.x + advance,
                                        y: line_pos.y + line.baseline - baseline,
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
                                        && interval.3 == piece_start =>
                                {
                                    interval.3 = advance;
                                }
                                _ => link_intervals.push((href, node, piece_start, advance)),
                            }
                        }
                    }
                }

                if let Some(next_line) = lines.get(line_num + 1) {
                    let next_line_pos = *positions.get(next_line.entity).unwrap();
                    ops.push(TextOperator::NewLine {
                        x_offset: next_line_pos.x - line_pos.x - text_cursor,
                        leading: (line.height - line.baseline) + next_line.baseline,
                    });
                }

                // Convert the intervals of this line into clickable areas for the hyperlinks.
                for (href, node, start, end) in link_intervals.drain(..) {
                    if end <= start {
                        continue;
                    }
                    hyperlinks.0.push(Hyperlink {
                        page: line_pos.page,
                        rect: Rect {
                            x: line_pos.x + start,
                            y: line_pos.y,
                            width: end - start,
                            height: line.height,
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
                        },
                        &mut paragraphs,
                    )
                    .build();
            }
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
    ops: &mut Vec<TextOperator>,
) -> Px {
    let is_invisible = |info: &super::text::GlyphInfo| {
        characters[info.cluster as usize..]
            .chars()
            .next()
            .is_some_and(is_invisible_formatting)
    };
    let shaped: Vec<_> = shape_directed(characters, font, fonts, at, rtl)
        .into_iter()
        .filter(|(_, info)| !is_invisible(info))
        .collect();

    // Glyphs of characters that `font` has no glyph for come from its fallback fonts. The font
    // is switched for them and switched back to `font` at the end of the piece.
    let fallbacks = font.fallbacks(&fonts.key);
    let font_at = |index: u16| -> &Font {
        match index {
            0 => font,
            n => &fallbacks[n as usize - 1],
        }
    };

    let (mut actual_text, boundaries) = record_to_unicode(characters, &shaped, &font_at);
    // Right-to-left glyphs are written in visual order, but text is logical order: the whole
    // piece gets its text as `/ActualText` (ISO 32000-1 14.9.4), which replaces the text of
    // the glyphs in it, including that of the graphemes that would have their own.
    let run_text = (rtl && !shaped.is_empty()).then(|| trim_invisible(characters));
    if run_text.is_some() {
        actual_text.clear();
    }
    let mut open_cluster: Option<usize> = None;

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
        if characters[cluster..].starts_with(char::is_whitespace) {
            advance += word_spacing;
            pen.0 += word_spacing_units.0;
        }
    }
    let piece_end = pen;

    // The glyphs are written in visual order, and right-to-left text gets its logical order from
    // `/ActualText`. Each kern moves the pen to where its glyph is drawn.
    let mut out = GlyphRunWriter::default();
    let mut current_font = 0u16;
    let mut rise = FontUnit(0.0);
    // where the pen is after the last glyph
    let mut pen = FontUnit(0.0);
    if let Some(text) = run_text {
        ops.push(TextOperator::BeginActualText(text.to_owned()));
    }
    for glyph in &placed {
        let cluster = grapheme_start(&boundaries, glyph.cluster);
        if open_cluster != Some(cluster) {
            if open_cluster.take().is_some() {
                out.flush(ops);
                ops.push(TextOperator::EndMarkedContent);
            }
            if let Some(text) = actual_text.get(&cluster) {
                out.flush(ops);
                ops.push(TextOperator::BeginActualText(text.clone()));
                open_cluster = Some(cluster);
            }
        }

        if glyph.font != current_font {
            out.flush(ops);
            current_font = glyph.font;
            ops.push(TextOperator::SetFont(
                font_at(current_font).postscript_name(),
                font_size,
            ));
        }
        let glyph_font = font_at(current_font);

        // In `TJ`, a positive kern moves the pen to the left.
        out.kern(FontUnit(pen.0 - glyph.x.0));
        // fonts have rounding noise in their anchors; below 1/1000 em a rise is invisible
        if (glyph.rise - rise).0.abs() >= 1.0 {
            out.flush(ops);
            rise = glyph.rise;
            ops.push(TextOperator::SetRise(font_size * (rise.0 as f32 / 1000.0)));
        }
        out.glyph(glyph.glyph);
        pen = FontUnit(glyph.x.0 + glyph_font.advance(glyph.glyph).0);
    }
    if open_cluster.is_some() {
        out.flush(ops);
        ops.push(TextOperator::EndMarkedContent);
    }
    // the text that follows continues from the end of the piece on the baseline
    out.kern(FontUnit(pen.0 - piece_end.0));
    out.flush(ops);
    if run_text.is_some() {
        ops.push(TextOperator::EndMarkedContent);
    }
    if rise.0 != 0.0 {
        ops.push(TextOperator::SetRise(Px(0.0)));
    }
    if current_font != 0 {
        ops.push(TextOperator::SetFont(font.postscript_name(), font_size));
    }

    advance
}

/// Collects glyphs and the kerns between them into `TJ` operators.
#[derive(Default)]
struct GlyphRunWriter {
    items: Vec<KernedGlyphs>,
    glyphs: Vec<GlyphIndex>,
    /// The kern that goes in front of the next glyph, summed up so that a glyph gets one kern.
    pending_kern: FontUnit,
}

impl GlyphRunWriter {
    fn kern(&mut self, kern: FontUnit) {
        self.pending_kern.0 += kern.0;
    }

    fn glyph(&mut self, glyph: GlyphIndex) {
        self.write_pending_kern();
        self.glyphs.push(glyph);
    }

    fn write_pending_kern(&mut self) {
        // 0.1/1000 em is far below anything visible; the coarser values compress better
        let kern = (self.pending_kern.0 * 10.0).round() / 10.0;
        if kern.abs() >= 0.1 {
            if !self.glyphs.is_empty() {
                self.items
                    .push(KernedGlyphs::Glyphs(Glyphs(std::mem::take(&mut self.glyphs))));
            }
            self.items.push(KernedGlyphs::Kern(FontUnit(kern)));
        }
        self.pending_kern = FontUnit(0.0);
    }

    /// Appends the collected glyphs and kerns to `ops` as one `TJ` operator.
    fn flush(&mut self, ops: &mut Vec<TextOperator>) {
        self.write_pending_kern();
        if !self.glyphs.is_empty() {
            self.items
                .push(KernedGlyphs::Glyphs(Glyphs(std::mem::take(&mut self.glyphs))));
        }
        if !self.items.is_empty() {
            ops.push(TextOperator::KernedGlyphs(std::mem::take(&mut self.items)));
        }
    }
}

/// Records which characters the glyphs of `shaped` (in logical order) stand for, so that the text
/// can be extracted from the PDF.
///
/// A cluster of one glyph goes into the `ToUnicode` map of its font, unless the map already says
/// something else for that glyph. Every other cluster (a glyph sequence like a reordered
/// vowel sign and its consonant, or a glyph that is used for different text) cannot be
/// described by a map from glyph to text: it is returned, by its start offset, with the text to
/// give it as `/ActualText`.
fn record_to_unicode<'a>(
    characters: &str,
    shaped: &[(super::text::GlyphPosition, super::text::GlyphInfo)],
    font_at: &impl Fn(u16) -> &'a Font,
) -> (std::collections::HashMap<usize, String>, Vec<usize>) {
    use unicode_segmentation::UnicodeSegmentation;
    let mut clusters: BTreeMap<usize, Vec<(u16, GlyphIndex)>> = BTreeMap::new();
    for (_, info) in shaped {
        clusters
            .entry(info.cluster as usize)
            .or_default()
            .push((info.font, GlyphIndex(info.codepoint)));
    }
    let starts: Vec<usize> = clusters.keys().copied().collect();
    // The unit of `/ActualText` is the grapheme cluster: a base character with its marks, whose
    // glyphs the extractor would otherwise have to assemble from the positions of the marks.
    // A unit has to hold whole glyph clusters too, since a glyph cannot be split between two
    // texts: a conjunct with its vowel sign (Kannada "ಲ್ಲಾ", Sinhala "ක්‍රි") is several
    // grapheme clusters but one glyph cluster. So a unit boundary is a grapheme boundary where a
    // glyph cluster starts.
    let boundaries: Vec<usize> = characters
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .filter(|&i| i == 0 || starts.binary_search(&i).is_ok())
        .collect();
    let mut unclean = std::collections::BTreeSet::new();
    for (i, (&start, glyphs)) in clusters.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(characters.len());
        // without the invisible characters that were dropped in front of the next cluster
        let Some(text) = characters.get(start..end) else { continue };
        let text = trim_invisible(text);
        if text.is_empty() {
            continue;
        }
        let clean = match glyphs[..] {
            [(font, glyph)] => {
                let mut map = font_at(font).to_unicode.lock();
                match map.get(&glyph) {
                    Some(existing) => existing == text,
                    None => {
                        map.insert(glyph, text.to_owned());
                        true
                    }
                }
            }
            _ => false,
        };
        if !clean {
            for &(font, glyph) in glyphs {
                font_at(font)
                    .to_unicode_fallback
                    .lock()
                    .entry(glyph)
                    .or_insert_with(|| text.to_owned());
            }
            unclean.insert(grapheme_start(&boundaries, start));
        }
    }
    let mut actual_text = std::collections::HashMap::new();
    for &start in &unclean {
        let i = boundaries.partition_point(|&b| b <= start);
        let end = boundaries.get(i).copied().unwrap_or(characters.len());
        actual_text.insert(start, trim_invisible(&characters[start..end]).to_owned());
    }
    (actual_text, boundaries)
}

/// `text` without the invisible formatting characters at its end, which have no glyphs. The
/// joiners stay: they are part of the spelling in Indic and Arabic scripts (a Malayalam chillu,
/// a Sinhala yansaya, a Persian half-space).
fn trim_invisible(text: &str) -> &str {
    text.trim_end_matches(|ch| is_invisible_formatting(ch) && !matches!(ch, '\u{200C}' | '\u{200D}'))
}

/// The start of the grapheme cluster that contains the byte `offset`.
fn grapheme_start(boundaries: &[usize], offset: usize) -> usize {
    boundaries[boundaries.partition_point(|&b| b <= offset).saturating_sub(1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::properties::computed::Language as Lang;

    /// Glyph clusters that a map from glyph to text cannot describe get `/ActualText`; the
    /// others go into the `ToUnicode` map, where a glyph keeps the text of its first use.
    #[test]
    fn ambiguous_clusters_get_actual_text() {
        use crate::layout::text::{GlyphInfo, GlyphPosition};
        use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};
        let font = crate::layout::text::font::system_font(
            &[FontFamily::Named("TeX Gyre Termes".into())],
            &FontWeight::Normal,
            &FontStyle::Normal,
        )
        .unwrap();
        let glyph = |ch: char| font.hb_font.get_nominal_glyph(ch).unwrap();
        let info = |codepoint: u32, cluster: u32| {
            (GlyphPosition::default(), GlyphInfo { codepoint, cluster, font: 0 })
        };
        // "a" one glyph; "b" drawn with the glyph of "a"; "c" with two glyphs; "d" one glyph
        let characters = "abcd";
        let shaped = [
            info(glyph('a'), 0),
            info(glyph('a'), 1),
            info(glyph('c'), 2),
            info(glyph('c'), 2),
            info(glyph('d'), 3),
        ];
        let (actual_text, _) = record_to_unicode(characters, &shaped, &|_| &font);
        let mut found: Vec<_> = actual_text.into_iter().collect();
        found.sort();
        assert_eq!(found, [(1, "b".to_owned()), (2, "c".to_owned())]);
        let map = font.to_unicode.lock();
        assert_eq!(map.get(&GlyphIndex(glyph('a'))).map(String::as_str), Some("a"));
        assert_eq!(map.get(&GlyphIndex(glyph('d'))).map(String::as_str), Some("d"));
    }

    fn font_named(family: &str) -> Font {
        use crate::style::properties::computed::{FontFamily, FontStyle, FontWeight};
        crate::layout::text::font::system_font(
            &[FontFamily::Named(family.into())],
            &FontWeight::Normal,
            &FontStyle::Normal,
        )
        .unwrap()
    }

    /// The operators `shape_piece` emits for `text`, with the width it returns.
    fn pieces(family: &str, text: &str, rtl: bool, word_spacing: Px) -> (Vec<TextOperator>, Px, Font) {
        let font = font_named(family);
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut ops = Vec::new();
        let width = shape_piece(text, &font, &fonts, None, Px(16.0), word_spacing, &[], rtl, &mut ops);
        (ops, width, font)
    }

    fn actual_texts(ops: &[TextOperator]) -> Vec<&str> {
        ops.iter()
            .filter_map(|op| match op {
                TextOperator::BeginActualText(text) => Some(text.as_str()),
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
    fn glyph_count(ops: &[TextOperator]) -> usize {
        ops.iter()
            .map(|op| match op {
                TextOperator::KernedGlyphs(items) => items
                    .iter()
                    .map(|item| match item {
                        KernedGlyphs::Glyphs(glyphs) => glyphs.0.len(),
                        KernedGlyphs::Kern(_) => 0,
                    })
                    .sum(),
                _ => 0,
            })
            .sum()
    }

    /// The pen has to end where the returned width says it does: the glyph advances minus
    /// the kerns of the `TJ` operators is the width of the piece.
    fn net_advance(ops: &[TextOperator], font: &Font, size: f32) -> f32 {
        let mut total = 0.0;
        for op in ops {
            if let TextOperator::KernedGlyphs(items) = op {
                for item in items {
                    match item {
                        KernedGlyphs::Glyphs(glyphs) => {
                            total += glyphs.0.iter().map(|g| font.advance(*g).0 as f32).sum::<f32>()
                        }
                        KernedGlyphs::Kern(kern) => total -= kern.0 as f32,
                    }
                }
            }
        }
        total * size / 1000.0
    }

    #[test]
    fn plain_text_has_no_marked_content() {
        let (ops, _, _) = pieces("TeX Gyre Termes", "Hello, world", false, Px(0.0));
        assert!(actual_texts(&ops).is_empty(), "{ops:?}");
        assert!(!ops.iter().any(|op| matches!(op, TextOperator::EndMarkedContent)));
        assert_eq!(glyph_count(&ops), "Hello, world".len());
    }

    /// The i-matra of "कि" comes first in the glyphs and is drawn before the consonant. Only
    /// that cluster is marked; the space between the clusters is not.
    #[test]
    fn reordered_clusters_are_wrapped_in_actual_text() {
        let (ops, _, _) = pieces("Noto Sans Devanagari", "कि पि", false, Px(0.0));
        assert_eq!(actual_texts(&ops), ["कि", "पि"]);
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

    /// The same shaped text gives the same operators the second time, although the `ToUnicode`
    /// map has the glyphs of the first by then.
    #[test]
    fn marking_does_not_depend_on_earlier_text() {
        let font = font_named("Noto Sans Devanagari");
        let fonts = crate::layout::text::fallback::TextFonts::default();
        let mut first = Vec::new();
        let mut second = Vec::new();
        for ops in [&mut first, &mut second] {
            shape_piece("कि", &font, &fonts, None, Px(16.0), Px(0.0), &[], false, ops);
        }
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }

    /// Right-to-left text, whose glyphs are in visual order, is marked with its text in logical
    /// order as a whole, marks and spaces included.
    #[test]
    fn right_to_left_pieces_get_their_logical_text() {
        for text in ["בְּרֵאשִׁית בָּרָא", "אב"] {
            let (ops, _, _) = pieces("Noto Sans Hebrew", text, true, Px(0.0));
            assert_eq!(actual_texts(&ops), [text], "{ops:?}");
            assert_balanced(&ops);
        }
    }

    /// A glyph cluster of several grapheme clusters (a Kannada conjunct with its vowel sign, a
    /// Sinhala yansaya with its vowel sign) is marked with the text of all of them.
    #[test]
    fn actual_text_covers_the_whole_glyph_cluster() {
        let (ops, _, _) = pieces("Noto Sans Kannada", "ಎಲ್ಲಾ", false, Px(0.0));
        assert_eq!(actual_texts(&ops).concat(), "ಲ್ಲಾ", "{ops:?}");
        assert_balanced(&ops);
        let (ops, _, _) = pieces("Noto Sans Sinhala", "ක්\u{200D}රියා", false, Px(0.0));
        let texts = actual_texts(&ops);
        assert!(texts.contains(&"ක්\u{200D}රි"), "{texts:?}");
        assert_balanced(&ops);
    }

    #[test]
    fn two_part_vowel_signs_are_marked() {
        let (ops, _, _) = pieces("Noto Sans Bengali", "কো", false, Px(0.0));
        assert_eq!(actual_texts(&ops), ["কো"]);
        assert_balanced(&ops);
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
                TextOperator::KernedGlyphs(items) => Some(items),
                _ => None,
            })
            .flatten()
            .filter_map(|item| match item {
                KernedGlyphs::Glyphs(g) => Some(g.0.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        let expected = [font.hb_font.get_nominal_glyph('ב').unwrap(), font.hb_font.get_nominal_glyph('א').unwrap()];
        assert_eq!(glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), expected);
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
}
