use super::components::{self, Paragraph, TextOperator};
use super::{text::font::GlyphIndex, FontMap};
use crate::style::properties::computed::PixelLength as Px;

use crate::layout::text::shape;

use components::{Glyphs, KernedGlyphs};
use lemon_pdf::font::FontUnit;
use specs::{Component, DenseVecStorage, Entity, VecStorage};
use specs::{Entities, Join, Read, ReadStorage, System, WriteStorage};

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
    pub elements: Vec<Entity>,
    pub break_points: Vec<BreakPoint>,
    pub lines: Vec<Line>,
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
    pub line_height: Px,
    pub baseline: Px,
}

// TODO: test this
impl<'a> System<'a> for FindPossibleLineBreaks {
    type SystemData = (
        WriteStorage<'a, InlineFormattingContext>,
        ReadStorage<'a, components::TextRun>,
        Read<'a, FontMap>,
    );

    fn run(&mut self, (mut inline_formatting_contexts, text_runs, font_map): Self::SystemData) {
        for InlineFormattingContext {
            elements,
            break_points,
            ..
        } in (&mut inline_formatting_contexts).join()
        {
            let mut advance = Px(0.0);

            for (entity, text_run) in elements
                .iter()
                .filter_map(|&entity| Some((entity, text_runs.get(entity)?)))
            {
                let font = &font_map[&text_run.font_name];
                let shaped = shape(&text_run.text, font);
                for (pos, glyph_info) in &shaped {
                    // insert first breakpoint at beginning
                    if break_points.is_empty() {
                        break_points.push(BreakPoint {
                            entity,
                            char_index: 0,
                            advance_before: Px(0.0),
                            advance_after: Px(0.0),
                            line_height: text_run.line_height,
                            baseline: text_run.baseline,
                        })
                    }

                    let cur_char = &text_run.text[glyph_info.cluster as usize..];
                    if cur_char.starts_with(|chr: char| chr.is_whitespace()) {
                        let advance_before = advance;
                        advance += font.scale_to_px(pos.x_advance, text_run.font_size);
                        let advance_after = advance;
                        break_points.push(BreakPoint {
                            entity,
                            char_index: glyph_info.cluster as usize,
                            advance_before,
                            advance_after,
                            line_height: text_run.line_height,
                            baseline: text_run.baseline,
                        })
                    } else {
                        advance += font.scale_to_px(pos.x_advance, text_run.font_size);
                    }
                }
            }

            if let Some(&entity) = elements.last() {
                if let Some(text_run) = text_runs.get(entity) {
                    break_points.push(BreakPoint {
                        entity,
                        char_index: text_run.text.len(),
                        advance_before: advance,
                        advance_after: advance,
                        line_height: Px(0.0),
                        baseline: Px(0.0),
                    })
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
        WriteStorage<'a, components::Position>,
        WriteStorage<'a, Paragraph>,
        Read<'a, FontMap>,
    );

    fn run(
        &mut self,
        (entities, inline_formatting_contexts, text_runs, mut positions, mut paragraphs, font_map): Self::SystemData,
    ) {
        for InlineFormattingContext {
            break_points,
            lines,
            ..
        } in inline_formatting_contexts.join()
        {
            let mut ops: Vec<TextOperator> = vec![];
            let mut previous_font: Option<(String, Px)> = None;
            let mut paragraph_pos: Option<components::Position> = None;
            for line in lines {
                let mut from = line.from_break;
                let to = line.to_break;

                let line_pos = *positions
                    .get(line.entity)
                    .expect("every line should have been assigned a position by now");

                if paragraph_pos.map(|pos| pos.page) != Some(line_pos.page) {
                    if let Some(pos) = paragraph_pos {
                        entities
                            .build_entity()
                            .with(pos, &mut positions)
                            .with(Paragraph { ops: ops.clone() }, &mut paragraphs)
                            .build();
                    }
                    ops.clear();
                    if let Some(previous_font) = &previous_font {
                        ops.push(TextOperator::SetFont(
                            previous_font.0.clone(),
                            previous_font.1,
                        ))
                    }
                    paragraph_pos = Some(components::Position {
                        y: line_pos.y + line.baseline,
                        height: Px(0.0),
                        ..line_pos
                    });
                }

                while from < to {
                    let from_break = break_points[from];
                    let to_break = break_points[to];

                    if let Some(run) = text_runs.get(from_break.entity) {
                        if previous_font != Some((run.font_name.clone(), run.font_size)) {
                            ops.push(TextOperator::SetFont(run.font_name.clone(), run.font_size));
                            previous_font = Some((run.font_name.clone(), run.font_size));
                        }

                        let from_char = if run.text.as_bytes()[from_break.char_index] == b' ' {
                            from_break.char_index + 1
                        } else {
                            from_break.char_index
                        };
                        let characters = if from_break.entity == to_break.entity {
                            from = to;
                            &run.text[from_char..to_break.char_index]
                        } else {
                            from += 1;
                            &run.text[from_char..]
                        };

                        let font = &font_map[&run.font_name];
                        let shaped = shape(characters, font);

                        let mut kerned_glyphs = vec![];
                        let mut glyphs = Vec::with_capacity(shaped.len());
                        for (pos, glyph_info) in shaped {
                            let kern = font.advance(GlyphIndex(glyph_info.codepoint))
                                - FontUnit(pos.x_advance as f64 * 1000f64 / font.upem() as f64);
                            if (kern.0 as i64).abs() >= 1 {
                                kerned_glyphs.push(KernedGlyphs::Glyphs(Glyphs(glyphs.clone())));
                                glyphs.clear();
                                kerned_glyphs.push(KernedGlyphs::Kern(-kern.0 as i64));
                            }
                            glyphs.push(GlyphIndex(glyph_info.codepoint))
                        }
                        kerned_glyphs.push(KernedGlyphs::Glyphs(Glyphs(glyphs)));
                        ops.push(TextOperator::KernedGlyphs(kerned_glyphs));
                    } else {
                        from += 1;
                    }
                }

                ops.push(TextOperator::NewLine {
                    leading: line.height,
                });
            }
            if let Some(pos) = paragraph_pos {
                entities
                    .build_entity()
                    .with(pos, &mut positions)
                    .with(Paragraph { ops: ops.clone() }, &mut paragraphs)
                    .build();
            }
        }
    }
}

// impl InlineFormatterState {
//     pub fn next_element(
//         &mut self,
//         elements: &[Entity],
//         line_width: Px,
//         remaining_inline_space: Px,
//         InlineFormatterData {
//             entities,
//             computed_values,
//             unscanned_texts,
//             ..
//         }: &mut InlineFormatterData,
//     ) -> Option<Entity> {
//         let result = if let Some(&entity) = elements.get(self.current_element_index) {
//             let (style, text) = (*computed_values, &mut **unscanned_texts)
//                 .join()
//                 .get(entity, entities)
//                 .expect("No style/text assigned");
//             super::text::whitespace::transform_text(&mut text.characters);
//             let glyph_metrics_provider = match_font(&style)
//                 .expect("Could not find any matching fonts.")
//                 .glyph_metrics_provider(style.font.font_size);
//             self.inline_fragment_iter_state = Some(InlineFragmentIteratorState::new(
//                 glyph_metrics_provider,
//                 line_width,
//                 remaining_inline_space,
//             ));
//             Some(entity)
//         } else {
//             None
//         };
//         self.current_element_index += 1;
//         result
//     }

//     pub fn fragment_iter<'a>(
//         &self,
//         text: &'a str,
//     ) -> impl 'a + Iterator<Item = Result<InlineFragment, LineIsFullError>> {
//         self.inline_fragment_iter_state
//             .as_ref()
//             .expect(
//                 "Before calling fragment_iter, the output of \
//                  `InlineFormatterState.next_element(..)` must be checked.",
//             )
//             .clone()
//             .into_iterator(text)
//     }
// }

/// Add the specified fragment to the current line.
// #[allow(clippy::too_many_arguments)]
// pub fn append_fragment(
//     &mut self,
//     fragment: InlineFragment,
//     line_height: Px,
//     font: Font,
//     font_size: Px,
//     entities: &Entities,
//     positions: &mut WriteStorage<Position>,
//     texts: &mut WriteStorage<Text>,
// ) {
//     let line_gap = line_height - font.font_height(font_size);
//     let pos = Position {
//         width: fragment.advance,
//         height: line_height,
//         ..Default::default() //< position gets assigned by `LineBuilder`.
//     };

//     let entity = entities
//         .build_entity()
//         .with(pos, positions)
//         .with(
//             Text {
//                 glyphs: fragment.glyph_indices,
//                 font: font.clone(),
//                 font_size,
//                 baseline: font.ascender(font_size) + line_gap / 2.0,
//             },
//             texts,
//         )
//         .build();

//     self.current_line.append_fragment(entity, positions);
// }

/// Creates a new line and adds it to the list of `finished_lines`.
///
/// Returns an Error if the new line exceeds the max block height (i.e. a page break is needed).
// fn finish_current_line(
//     &mut self,
//     entities: &Entities,
//     texts: &WriteStorage<Text>,
//     inline_pos: &mut WriteStorage<InlinePos>,
//     positions: &mut WriteStorage<Position>,
//     line_boxes: &mut WriteStorage<LineBox>,
// ) -> Result<(), StackFullError> {
//     let line = self
//         .current_line
//         .build(entities, texts, inline_pos, positions, line_boxes);
//     self.current_line.clear();
//     self.current_line.inline_size = self.line_width;
//     if let Some(line) = line {
//         let pos = positions.get(line).unwrap();
//         self.finished_lines.push(line, pos.height)?;
//     }

//     // save state at new line
//     self.saved_state = Some(self.state.clone());
//     Ok(())
// }

/// Processes an element, splitting it into inline fragments and distributing them across lines.
///
/// Returns `Some(...)` if
// pub fn layout_element(
//     &mut self,
//     entity: Entity,
//     data: &mut InlineFormatterData,
// ) -> Option<LineStack> {
//     let (style, unscanned_text) = (data.computed_values, &*data.unscanned_texts)
//         .join()
//         .get(entity, data.entities)?;
//     let content = &unscanned_text.characters;
//     let font = match_font(&style)?;

//     let font_size = style.font.font_size;
//     let font_height = font.font_height(style.font.font_size);
//     let Font::BuiltIn(font) = font;
//     let line_height = match style.font.line_height {
//         LineHeight::Normal => font_height * 1.2,
//         LineHeight::LengthPercentage(lp) => lp.absolute(style.font.font_size),
//     };

//     for fragment in self.state.fragment_iter(&content) {
//         let requires_new_line = match fragment {
//             Ok(fragment) => {
//                 let new_line_after = fragment.new_line_after;
//                 self.append_fragment(
//                     fragment,
//                     line_height,
//                     Font::BuiltIn(font),
//                     font_size,
//                     data.entities,
//                     data.positions,
//                     data.texts,
//                 );
//                 new_line_after
//             }
//             Err(LineIsFullError) => true,
//         };

//         if requires_new_line
//             && self
//                 .finish_current_line(
//                     data.entities,
//                     data.texts,
//                     data.inline_pos,
//                     data.positions,
//                     data.line_boxes,
//                 )
//                 .is_err()
//         {
//             // we exceeded the maximum height
//             self.undo_current_line();

//             let max_height = self.finished_lines.max_height;
//             let finished_lines =
//                 std::mem::replace(&mut self.finished_lines, Stack::new(max_height));
//             return Some(LineStack {
//                 stack: finished_lines,
//                 page_break_after: true,
//             });
//         }
//     }

//     None
// }

// fn layout_lines(
//     &mut self,
//     entities: &[Entity],
//     data: &mut InlineFormatterData,
// ) -> Option<LineStack> {
//     loop {
//         if let Some(elem) = self.state.next_element(
//             entities,
//             self.line_width,
//             self.remaining_inline_space(),
//             data,
//         ) {
//             if let Some(stack) = self.layout_element(elem, data) {
//                 return Some(stack);
//             }
//         } else {
//             // TODO: Test this
//             let page_break_after = match self.finish_current_line(
//                 data.entities,
//                 data.texts,
//                 data.inline_pos,
//                 data.positions,
//                 data.line_boxes,
//             ) {
//                 Ok(()) => false,
//                 Err(StackFullError) => true,
//             };
//             let max_height = self.finished_lines.max_height;
//             let stack = std::mem::replace(&mut self.finished_lines, Stack::new(max_height));
//             if stack.is_empty() {
//                 return None;
//             }
//             return Some(LineStack {
//                 stack,
//                 page_break_after,
//             });
//         }
//     }
// }

// /// Returns an iterator which produces stacks of lines.
// pub fn iter<'refs, 'data>(
//     &'refs mut self,
//     entities: &'refs [Entity],
//     data: &'refs mut InlineFormatterData<'refs, 'data>,
// ) -> InlineFormatterIter<'refs, 'data> {
//     InlineFormatterIter {
//         inline_formatter: self,
//         entities,
//         data,
//     }
// }

// pub struct InlineFormatterIter<'refs> {
//     inline_formatter: &'refs mut InlineFormatter,
//     entities: &'refs [Entity],
// }

// impl<'refs, 'data> Iterator for InlineFormatterIter<'refs> {
//     type Item = LineStack;

//     fn next(&mut self) -> Option<Self::Item> {
//         self.inline_formatter.layout_lines(self.entities, self.data)
//     }
// }

/// Contains entities and keeps track of their total height.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    entities: Vec<Entity>,
    total_height: Px,
    pub max_height: Px,
}

impl Stack {
    pub fn new(max_height: Px) -> Self {
        Stack {
            entities: vec![],
            total_height: Px(0.0),
            max_height,
        }
    }

    pub fn push(&mut self, entity: Entity, height: Px) -> Result<(), StackFullError> {
        if self.total_height + height > self.max_height {
            Err(StackFullError)
        } else {
            self.total_height += height;
            self.entities.push(entity);
            Ok(())
        }
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn height(&self) -> Px {
        self.total_height
    }

    /// Returns the number of entities stored in the stack.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns `true` if the stack contains no entities.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    // pub fn set_positions(
    //     &self,
    //     offset: (Px, Px, ZIndex),
    //     positions: &mut WriteStorage<'_, Position>,
    // ) -> Result<(), specs::error::WrongGeneration> {
    //     let mut pos_y = offset.1;
    //     for &stack_element in &self.entities {
    //         let pos = positions
    //             .entry(stack_element)?
    //             .or_insert(Default::default());
    //         pos.x = offset.0;
    //         pos.y = pos_y;
    //         pos_y += pos.height;
    //         pos.z = offset.2;
    //     }
    //     Ok(())
    // }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct StackFullError;

/// Constructs a line and assigns the position of the fragments therein.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct LineBuilder {
    pub inline_size: Px,
    cursor: Px,
    inline_fragments: Vec<Entity>,
}

impl LineBuilder {
    pub fn new(inline_size: Px) -> Self {
        LineBuilder {
            inline_size,
            ..Default::default()
        }
    }

    pub fn remaining_inline_space(&self) -> Px {
        self.inline_size - self.cursor
    }

    /// Add a fragment to the line. The given position's size will be respected.
    ///
    /// # Panics
    ///
    /// Panics if `positions` contains no entry for `fragment`.
    ///
    /// N.B. reads `position.width` and sets `position.x`.
    // pub fn append_fragment(
    //     &mut self,
    //     fragment: Entity,
    //     positions: &mut WriteStorage<'_, Position>,
    // ) {
    //     let position = positions
    //         .get_mut(fragment)
    //         .expect("Inline fragment needs to have an assigned Position");
    //     self.inline_fragments.push(fragment);
    //     position.x = self.cursor;
    //     self.cursor += position.width;
    // }

    /// Finish the line construction and set the final positions of the line fragments.
    // pub fn build(
    //     &self,
    //     entities: &Entities<'_>,
    //     texts: impl GenericReadStorage<Component = Text>,
    //     mut inline_pos: impl GenericWriteStorage<Component = InlinePos>,
    //     positions: &mut WriteStorage<'_, Position>,
    //     line_boxes: &mut WriteStorage<'_, LineBox>,
    // ) -> Option<Entity> {
    //     let line_entity = if self.inline_fragments.is_empty() {
    //         return None;
    //     } else {
    //         entities.create()
    //     };

    //     // Align the fragments along the baseline.
    //     // First find max ascender and max descender...
    //     let get_ascent = |entity, positions: &WriteStorage<'_, _>| {
    //         let &Position { height, .. } = positions.get(entity).unwrap();
    //         texts.get(entity).map(|t| t.baseline).unwrap_or(height)
    //     };
    //     let get_descent = |entity, positions: &WriteStorage<'_, _>| {
    //         let &Position { height, .. } = positions.get(entity).unwrap();
    //         texts
    //             .get(entity)
    //             .map(|t| height - t.baseline)
    //             .unwrap_or(Px(0.0))
    //     };
    //     let mut max_ascent = Px(0.0);
    //     let mut max_descent = Px(0.0);
    //     for &fragment in &self.inline_fragments {
    //         inline_pos
    //             .insert(fragment, InlinePos { line: line_entity })
    //             .unwrap();

    //         if get_ascent(fragment, &positions) > max_ascent {
    //             max_ascent = get_ascent(fragment, &positions)
    //         }
    //         if get_descent(fragment, &positions) > max_descent {
    //             max_descent = get_descent(fragment, &positions)
    //         }
    //     }

    //     // ...then shift the individual fragments to align at the baseline.
    //     for &fragment in &self.inline_fragments {
    //         let ascent = get_ascent(fragment, &positions);
    //         let pos = positions.get_mut(fragment).unwrap();
    //         pos.y = max_ascent - ascent;
    //     }

    //     let pos = Position {
    //         height: max_ascent + max_descent,
    //         width: self.cursor,
    //         ..Default::default() //< Position gets assigned by block formatter.
    //     };
    //     positions.insert(line_entity, pos).unwrap();
    //     line_boxes
    //         .insert(
    //             line_entity,
    //             LineBox {
    //                 fragments: self.inline_fragments.clone(),
    //             },
    //         )
    //         .unwrap();

    //     Some(line_entity)
    // }

    pub fn clear(&mut self) {
        self.inline_fragments.clear();
        self.cursor = Px(0.0);
    }
}

#[derive(Debug)]
pub struct InlinePos {
    pub line: Entity,
}

impl Component for InlinePos {
    type Storage = VecStorage<Self>;
}

#[derive(Debug, Default)]
pub struct LineBox {
    pub fragments: Vec<Entity>,
}

impl Component for LineBox {
    type Storage = VecStorage<Self>;
}

pub struct InlineLayoutPrepare;

impl<'a> System<'a> for InlineLayoutPrepare {
    type SystemData = ();

    fn run(&mut self, _: Self::SystemData) {}
}

pub struct InlinePositionAssign;

// impl<'a> System<'a> for InlinePositionAssign {
//     type SystemData = (
//         Entities<'a>,
//         WriteStorage<'a, Position>,
//         ReadStorage<'a, InlinePos>,
//     );

//     fn run(&mut self, (entities, mut positions, inline_pos): Self::SystemData) {
//         for (entity, &InlinePos { line }) in (&entities, &inline_pos).join() {
//             // This closure is for convenience (we can use the `?` operator).
//             let assign_pos = |positions: &mut WriteStorage<Position>| {
//                 let parent_pos = *positions.get(line)?;
//                 let child_pos = positions.get_mut(entity)?;
//                 child_pos.x += parent_pos.x;
//                 child_pos.y += parent_pos.y;
//                 child_pos.z = parent_pos.z;
//                 Some(())
//             };
//             assign_pos(&mut positions)
//                 .expect("Internal Error: cannot assign position to inline fragment.")
//         }
//     }
// }

/// A `LineStack` contains a list of lines that should be laid out vertically (block-direction) one
/// after the other.
#[derive(Debug, Clone)]
pub struct LineStack {
    pub stack: Stack,
    pub page_break_after: bool,
}
