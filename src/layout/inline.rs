use super::components::{self, Paragraph, TextOperator};
use super::{
    text::{font::GlyphIndex, Font},
    FontMap,
};
use crate::style::properties::computed::{PixelLength as Px, TextAlign};

use crate::layout::text::shape;

use components::{Glyphs, IntrinsicSize, KernedGlyphs, Position};
use lemon_pdf::font::FontUnit;
use specs::{Component, DenseVecStorage, Entity};
use specs::{Entities, Join, Read, ReadStorage, System, WriteStorage};

use std::collections::BTreeMap;

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
    pub break_points: Vec<BreakPoint>,
    pub lines: Vec<Line>,
}

impl InlineFormattingContext {
    pub fn line_height(
        &self,
        from_index: usize,
        to_index: usize,
        text_runs: &ReadStorage<components::TextRun>,
        intrinsic_sizes: &ReadStorage<components::IntrinsicSize>,
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
}

// TODO: test this
impl<'a> System<'a> for FindPossibleLineBreaks {
    type SystemData = (
        WriteStorage<'a, InlineFormattingContext>,
        WriteStorage<'a, components::TextRun>,
        ReadStorage<'a, components::IntrinsicSize>,
        Read<'a, FontMap>,
    );

    fn run(
        &mut self,
        (mut inline_formatting_contexts, mut text_runs, intrinsic_sizes, font_map): Self::SystemData,
    ) {
        for InlineFormattingContext {
            elements,
            break_points,
            ..
        } in (&mut inline_formatting_contexts).join()
        {
            // insert first breakpoint at beginning
            break_points.push(BreakPoint {
                entity: elements[0],
                char_index: 0,
                advance_before: Px(0.0),
                advance_after: Px(0.0),
            });

            let mut advance = Px(0.0);

            let mut last_whitespace = true;

            for (index, &entity) in elements.iter().enumerate() {
                if let Some(text_run) = text_runs.get_mut(entity) {
                    super::text::whitespace::transform_text(&mut text_run.text, last_whitespace);
                    if text_run.text.ends_with(char::is_whitespace) {
                        last_whitespace = true;
                    } else {
                        last_whitespace = false;
                    }
                    if index == elements.len() - 1 && last_whitespace {
                        let new_len = text_run.text.trim_end().len();
                        text_run.text.truncate(new_len);
                    }

                    let font = &font_map[&text_run.font_name];
                    let shaped = shape(&text_run.text, font);
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
                            })
                        } else {
                            advance += font.scale_to_px(pos.x_advance, text_run.font_size);
                        }
                    }
                } else if let Some(intrinsiz_size) = intrinsic_sizes.get(entity) {
                    last_whitespace = false;
                    advance += intrinsiz_size.width().unwrap_or_default();
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
            })
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
        WriteStorage<'a, components::Position>,
        WriteStorage<'a, Paragraph>,
        Read<'a, FontMap>,
    );

    fn run(
        &mut self,
        (
            entities,
            inline_formatting_contexts,
            text_runs,
            intrinsic_sizes,
            mut positions,
            mut paragraphs,
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
                break;
            };

            let mut ops: Vec<TextOperator> = vec![];
            let mut previous_font: Option<(String, Px)> = None;
            let mut paragraph_pos: Option<components::Position> = None;

            // position of the first baseline of the paragraph
            let mut paragraph_start = (Px(0.0), Px(0.0));

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
                let word_spacing: Px = if (!is_last_line) && *text_align == TextAlign::Justify {
                    // the ideal and maximal width of the line
                    let wanted_width = line_pos.width;
                    // the number of stretchable spaces in the current line (needed for justification)
                    let num_spaces = to - from - 1;
                    // the actual width of the unstretched characters in the line
                    let line_width =
                        break_points[to].advance_before - break_points[from].advance_after;
                    (wanted_width - line_width) / num_spaces as f32
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
                    advance += if let Some(run) = text_runs.get(entity) {
                        if !run.text.is_empty() {
                            if previous_font != Some((run.font_name.clone(), run.font_size)) {
                                ops.push(TextOperator::SetFont(
                                    run.font_name.clone(),
                                    run.font_size,
                                ));
                                previous_font = Some((run.font_name.clone(), run.font_size));
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

                            shape_piece(characters, font, run.font_size, word_spacing, &mut ops)
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
                        width
                    } else {
                        log::error!("unsopported inline element type");
                        Px(0.0)
                    };

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

fn shape_piece(
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
