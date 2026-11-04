use super::components::{self, EntityHrefs, Hyperlink, Hyperlinks, Paragraph, TextOperator};
use super::{
    FontMap,
    text::{Font, font::GlyphIndex},
};
use crate::style::properties::computed::{ComputedValues, PixelLength as Px, RGBA, TextAlign};

use crate::display_list::Rect;
use crate::layout::text::shape;

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
}
impl InlineFormattingContext {
    /// Breaks the paragraph into lines using adjustment-ratio badness and breakpoint demerits.
    pub fn break_lines(
        &mut self,
        line_width: Px,
        indent: Px,
        entities: &Entities,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &impl GenericReadStorage<Component = components::IntrinsicSize>,
    ) -> Result<(), String> {
        let breakpoints = &self.break_points;
        if breakpoints.is_empty() {
            self.lines.clear();
            return Ok(());
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
                Ok(Line {
                    from_break: window[0],
                    to_break: window[1],
                    height,
                    baseline,
                    entity: entities.create(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.lines = lines;
        Ok(())
    }

    /// The width of the content between two break points.
    pub fn content_width(&self, from_break: usize, to_break: usize) -> Px {
        self.break_points[to_break].advance_before - self.break_points[from_break].advance_after
            + self.break_points[to_break].hyphen_width
    }

    fn space_count(&self, from_break: usize, to_break: usize) -> usize {
        self.break_points[from_break + 1..to_break]
            .iter()
            .filter(|break_point| break_point.space)
            .count()
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
                (Px(0.0), Px(10.0)),
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
        let dictionary = Standard::from_embedded(Language::EnglishUS)
            .expect("embedded English hyphenation dictionary should load");

        for InlineFormattingContext {
            elements,
            break_points,
            atom_offsets,
            ..
        } in (&mut inline_formatting_contexts).join()
        {
            // an empty block (e.g. `<div></div>`) has an inline formatting context without content
            if elements.is_empty() {
                continue;
            }

            // insert first breakpoint at beginning
            break_points.push(BreakPoint {
                entity: elements[0],
                char_index: 0,
                advance_before: Px(0.0),
                advance_after: Px(0.0),
                forced: false,
                space: false,
                hyphen: false,
                hyphen_width: Px(0.0),
                glue_stretch: Px(0.0),
                glue_shrink: Px(0.0),
                penalty: 0,
            });

            let mut advance = Px(0.0);

            for (_index, &entity) in elements.iter().enumerate() {
                if let Some(text_run) = text_runs.get_mut(entity) {
                    let font = &font_map[&text_run.font_name];
                    let shaped = shape(&text_run.text, font);
                    let run_start_advance = advance;
                    for (pos, glyph_info) in &shaped {
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
                            word_ranges(&text_run.text)
                                .into_iter()
                                .flat_map(|(word_start, word_end)| {
                                    dictionary
                                        .hyphenate(&text_run.text[word_start..word_end])
                                        .breaks
                                        .into_iter()
                                        .map(move |offset| word_start + offset)
                                })
                                .collect()
                        }
                    };
                    if !hyphenation.is_empty() {
                        let hyphen_width = shape("-", font)
                            .iter()
                            .map(|(position, _)| {
                                font.scale_to_px(position.x_advance, text_run.font_size)
                            })
                            .fold(Px(0.0), |sum, width| sum + width);
                        for char_index in hyphenation {
                            if !text_run.text.is_char_boundary(char_index) || char_index == 0 {
                                continue;
                            }
                            let prefix = &text_run.text[..char_index];
                            let prefix_width = shape(prefix, font)
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
                                hyphen: true,
                                hyphen_width,
                                glue_stretch: Px(0.0),
                                glue_shrink: Px(0.0),
                                penalty: 50,
                            });
                        }
                    }
                } else if let Some(intrinsic_size) = intrinsic_sizes.get(entity) {
                    advance += intrinsic_size.width().unwrap_or_default();
                } else if computed_values.get(entity).map_or(false, |style| {
                    style.box_.display == crate::style::properties::specified::Display::InlineBlock
                }) {
                    // the width is only known once the box has been laid out
                    atom_offsets.push((entity, advance));
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
    );

    fn run(
        &mut self,
        (entities, text_list, ifcs, markers, mut positions, mut paragraphs, mut graphics, font_map): Self::SystemData,
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
                        created.push((item_x, line_pos, baseline, marker.clone()));
                    }
                }
                _ => {}
            }
        }

        for (item_x, line_pos, baseline, marker) in created {
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
            let width = shape_piece(&marker.text, font, marker.font_size, Px(0.0), &mut ops);
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

                // the same word spacing as the line box creator uses
                let is_last_line = line_num == ifc.lines.len() - 1;
                let num_spaces = ifc.space_count(line.from_break, line.to_break);
                let word_spacing = if !is_last_line
                    && ifc.text_align == crate::style::properties::specified::TextAlign::Justify
                    && num_spaces > 0
                {
                    (line_pos.width - ifc.content_width(line.from_break, line.to_break))
                        / num_spaces as f32
                } else {
                    Px(0.0)
                };

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
                    let spaces = (line.from_break + 1..line.to_break)
                        .filter(|&b| ifc.break_points[b].advance_before.0 < offset.0 - EPSILON)
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
        ): Self::SystemData,
    ) {
        for InlineFormattingContext {
            break_points,
            lines,
            elements,
            text_align,
            ..
        } in inline_formatting_contexts.join()
        {
            let mut elements_iter = elements.iter();

            let mut entity = if let Some(&e) = elements_iter.next() {
                e
            } else {
                // an empty block has nothing to draw, but other contexts still do
                continue;
            };

            let mut ops: Vec<TextOperator> = vec![];
            let mut previous_font: Option<(String, Px)> = None;
            let mut previous_color: Option<RGBA> = None;
            let mut paragraph_pos: Option<components::Position> = None;

            // position of the first baseline of the paragraph
            let mut paragraph_start = (Px(0.0), Px(0.0));

            // The x-intervals on the current line of every hyperlink that has text on it.
            // `link_intervals: href -> (start, end)`; adjacent intervals of the same link are
            // merged as the text runs of a link are visited in order.
            let mut link_intervals: HashMap<&str, (Px, Px)> = HashMap::new();

            for (line_num, line) in lines.iter().enumerate() {
                let from = line.from_break;
                let to = line.to_break;

                let line_pos = *positions
                    .get(line.entity)
                    .expect("every line should have been assigned a position by now");

                if line_num == 0 {
                    paragraph_start = (line_pos.x, line_pos.y + line.baseline);
                }

                // if we want to justify the text we have to add word-spacing
                let is_last_line = line_num == lines.len() - 1;
                let num_spaces = break_points[from + 1..to]
                    .iter()
                    .filter(|break_point| break_point.space)
                    .count();
                let word_spacing: Px =
                    if !is_last_line && *text_align == TextAlign::Justify && num_spaces > 0 {
                        // the ideal and maximal width of the line
                        let desired_width = line_pos.width;
                        // the actual width of the unstretched characters in the line
                        let line_width = break_points[to].advance_before
                            - break_points[from].advance_after
                            + break_points[to].hyphen_width;
                        (desired_width - line_width) / num_spaces as f32
                    } else {
                        // no adjustment needed since this line should not be justified
                        Px(0.0)
                    };

                // Check if a page break happened between this line and the previous. If a page
                // break happend we have to add the existing (partial) paragraph to the previous
                // page and create a fresh paragraph for the remaining lines.
                if paragraph_pos.map(|pos| pos.page) != Some(line_pos.page) {
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

                let from_break = break_points[from];
                let to_break = break_points[to];

                let mut advance = Px(0.0);
                let mut text_cursor = Px(0.0);
                loop {
                    // The x position of the current piece on the line, before its width is
                    // added to `advance`.
                    let piece_start = advance;
                    let piece_width = if let Some(run) = text_runs.get(entity) {
                        if !run.text.is_empty() {
                            if previous_font != Some((run.font_name.clone(), run.font_size)) {
                                ops.push(TextOperator::SetFont(
                                    run.font_name.clone(),
                                    run.font_size,
                                ));
                                previous_font = Some((run.font_name.clone(), run.font_size));
                            }

                            if previous_color != Some(run.color) {
                                ops.push(TextOperator::SetColor(run.color));
                                previous_color = Some(run.color);
                            }

                            // get the string slice of the current text fragment
                            let start = if entity == from_break.entity {
                                from_break.char_index
                            } else {
                                0
                            };
                            let end = if entity == to_break.entity {
                                to_break.char_index
                            } else {
                                run.text.len()
                            };

                            let characters = &run.text[start..end];
                            let font = &font_map[&run.font_name];

                            let mut piece_width = shape_piece(
                                characters,
                                font,
                                run.font_size,
                                word_spacing,
                                &mut ops,
                            );
                            if to_break.hyphen && entity == to_break.entity {
                                piece_width +=
                                    shape_piece("-", font, run.font_size, Px(0.0), &mut ops);
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
                        } else {
                            Px(0.0)
                        }
                    } else if let Some(&IntrinsicSize::SizeWithBaseline {
                        width,
                        height,
                        baseline,
                    }) = intrinsic_sizes.get(entity)
                    {
                        ops.push(TextOperator::NewLine {
                            x_offset: advance + width - text_cursor,
                            leading: Px(0.0),
                        });
                        text_cursor = advance + width;

                        // The contents of an `inline-block` box are placed by `PlaceInlineBlocks`.
                        let is_inline_block = computed_values.get(entity).map_or(false, |style| {
                            style.box_.display
                                == crate::style::properties::specified::Display::InlineBlock
                        });
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
                            let interval = link_intervals
                                .entry(href)
                                .or_insert_with(|| (piece_start, piece_start));
                            interval.1 = advance;
                        }
                    }

                    if entity == to_break.entity {
                        break;
                    } else {
                        entity = match elements_iter.next() {
                            Some(&ent) => ent,
                            None => break,
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
                for (href, (start, end)) in link_intervals.drain() {
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
                    });
                }
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

pub(super) fn shape_piece(
    characters: &str,
    font: &Font,
    font_size: Px,
    word_spacing: Px,
    ops: &mut Vec<TextOperator>,
) -> Px {
    let shaped = shape(characters, font);

    let mut kerned_glyphs = vec![];
    let mut glyphs = Vec::with_capacity(shaped.len());

    // TODO: We currently rely on the first glyph having cluster value 0
    let mut previous_cluster = 0xFFFF;
    let mut cluster_starting_glyph = 0xFFFF;

    let mut to_unicode_map = BTreeMap::new();

    let mut advance = Px(0.0);
    for (pos, glyph_info) in shaped {
        advance += font.scale_to_px(pos.x_advance, font_size);

        if glyph_info.cluster != previous_cluster {
            // a new cluster began
            if previous_cluster < glyph_info.cluster {
                let text = &characters[previous_cluster as usize..glyph_info.cluster as usize];
                to_unicode_map.insert(GlyphIndex(cluster_starting_glyph), text.to_owned());
            }
            cluster_starting_glyph = glyph_info.codepoint;
            previous_cluster = glyph_info.cluster;
        }

        glyphs.push(GlyphIndex(glyph_info.codepoint));
        let mut kern = font.advance(GlyphIndex(glyph_info.codepoint)) - pos.x_advance;

        // adjust kerning of space glyphs to allow text justification
        if characters[glyph_info.cluster as usize..].starts_with(char::is_whitespace) {
            advance += word_spacing;
            let word_spacing = FontUnit((word_spacing / font_size.0).0 as f64 * 1000.0);
            kern.0 -= word_spacing.0;
        }

        // 0.1/1000 em is far below anything visible; the coarser values compress better
        kern.0 = (kern.0 * 10.0).round() / 10.0;
        if kern.0.abs() >= 0.1 {
            kerned_glyphs.push(KernedGlyphs::Glyphs(Glyphs(glyphs.clone())));
            kerned_glyphs.push(KernedGlyphs::Kern(kern));
            glyphs.clear();
        }
    }
    if !glyphs.is_empty() {
        kerned_glyphs.push(KernedGlyphs::Glyphs(Glyphs(glyphs)));
    }
    if !kerned_glyphs.is_empty() {
        ops.push(TextOperator::KernedGlyphs(kerned_glyphs));
    }

    if let Some(text) = characters.get(previous_cluster as usize..) {
        to_unicode_map.insert(GlyphIndex(cluster_starting_glyph), text.to_owned());
    }
    font.to_unicode.lock().append(&mut to_unicode_map);

    advance
}
