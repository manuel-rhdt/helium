use super::entity::{Position, Text, UnscannedText};
use super::text::{font::match_font, Font, InlineFragment, InlineFragmentIteratorState};
use crate::style::properties::computed::{ComputedValues, LineHeight, PixelLength as Px};

use specs::{
    storage::{GenericReadStorage, GenericWriteStorage},
    Component, Entity, VecStorage,
};
use specs::{DenseVecStorage, Entities, Join, ReadStorage, System, WriteStorage};

#[derive(Debug, Clone)]
pub struct InlineFormattingContext {
    pub elements: Vec<Entity>,
}

impl Component for InlineFormattingContext {
    type Storage = VecStorage<Self>;
}

pub struct InlineFormatterData<'refs, 'system> {
    pub entities: &'refs Entities<'system>,
    pub positions: &'refs mut WriteStorage<'system, Position>,
    pub texts: &'refs mut WriteStorage<'system, Text>,
    pub inline_pos: &'refs mut WriteStorage<'system, InlinePos>,
    pub computed_values: &'refs ReadStorage<'system, ComputedValues>,
    pub unscanned_texts: &'refs mut WriteStorage<'system, UnscannedText>,
    pub stacks: &'refs mut WriteStorage<'system, Stack>,
}

#[derive(Debug, Default, Clone)]
struct InlineFormatterState {
    /// An index into the elements array of the `InlineFormattingContext`, pointing to the current
    /// element.
    pub current_element_index: usize,
    /// Additional state associated with the current element.
    inline_fragment_iter_state: Option<InlineFragmentIteratorState>,
}

impl InlineFormatterState {
    pub fn next_element(
        &mut self,
        elements: &[Entity],
        InlineFormatterData {
            computed_values, ..
        }: &InlineFormatterData,
    ) {
        self.current_element_index += 1;
        if let Some(entity) = elements.get(self.current_element_index).cloned() {
            let style = computed_values.get(entity).expect("No style assigned");
            let Font::BuiltIn(font) = match_font(&style).unwrap();
            self.inline_fragment_iter_state =
                Some(InlineFragmentIteratorState::new(font, style.font.font_size));
        }
    }

    pub fn fragment_iter<'a>(&self, text: &'a str) -> impl 'a + Iterator<Item = InlineFragment> {
        self.inline_fragment_iter_state.as_ref().unwrap().iter(text)
    }
}

pub struct InlineFormatter {
    current_line: LineBuilder,
    finished_lines: Stack,
    saved_state: Option<InlineFormatterState>,
    state: InlineFormatterState,
}

impl InlineFormatter {
    pub fn new(line_width: Px, max_height: Px) -> Self {
        InlineFormatter {
            current_line: LineBuilder::new(line_width),
            finished_lines: Stack::new(max_height),
            saved_state: Default::default(),
            state: Default::default(),
        }
    }

    pub fn remaining_inline_space(&self) -> Px {
        self.current_line.remaining_inline_space()
    }

    /// Add the specified fragment to the current line.
    pub fn append_fragment(
        &mut self,
        fragment: InlineFragment,
        line_height: Px,
        font: Font,
        font_size: Px,
        entities: &Entities,
        positions: &mut WriteStorage<Position>,
        texts: &mut WriteStorage<Text>,
    ) {
        let line_gap = line_height - font.font_height(font_size);
        let pos = Position {
            width: fragment.advance,
            height: line_height,
            ..Default::default() //< position gets assigned by `LineBuilder`.
        };

        let entity = entities
            .build_entity()
            .with(pos, positions)
            .with(
                Text {
                    glyphs: fragment.glyph_indices,
                    font: font.clone(),
                    font_size,
                    baseline: font.ascender(font_size) + line_gap / 2.0,
                },
                texts,
            )
            .build();

        self.current_line.append_fragment(entity, positions);
    }

    /// Creates a new line and adds it to the list of `finished_lines`.
    ///
    /// Returns an Error if the new line exceeds the max block height (i.e. a page break is needed).
    fn new_line(
        &mut self,
        entities: &Entities,
        texts: &WriteStorage<Text>,
        inline_pos: &mut WriteStorage<InlinePos>,
        positions: &mut WriteStorage<Position>,
    ) -> Result<(), StackFullError> {
        let line = self
            .current_line
            .build(entities, texts, inline_pos, positions);
        self.current_line.clear();
        if let Some(line) = line {
            let pos = positions.get(line).unwrap();
            self.finished_lines.push(line, pos.height)?;
        }

        // save state at new line
        self.saved_state = Some(self.state.clone());
        Ok(())
    }

    /// Processes an element, splitting it into inline fragments and distributing across lines.
    ///
    /// Returns `Some(...)` if
    pub fn layout_element(
        &mut self,
        entity: Entity,
        data: &mut InlineFormatterData,
    ) -> Option<LineStack> {
        let style = data.computed_values.get(entity).expect("No style assigned");
        let UnscannedText {
            characters: content,
        } = data.unscanned_texts.get(entity).unwrap();
        let font = match_font(&style).unwrap();
        let font_size = style.font.font_size;
        let font_height = font.font_height(style.font.font_size);
        let Font::BuiltIn(font) = font;
        let line_height = match style.font.line_height {
            LineHeight::Normal => font_height * 1.2,
            LineHeight::LengthPercentage(lp) => lp.absolute(style.font.font_size),
        };

        for fragment in self.state.fragment_iter(&content) {
            let new_line_after = fragment.new_line_after;
            self.append_fragment(
                fragment,
                line_height,
                Font::BuiltIn(font),
                font_size,
                data.entities,
                data.positions,
                data.texts,
            );
            if new_line_after
                && self
                    .new_line(data.entities, data.texts, data.inline_pos, data.positions)
                    .is_err()
            {
                // we exceeded the maximum height
                self.undo_current_line();

                let max_height = self.finished_lines.max_height;
                let finished_lines =
                    std::mem::replace(&mut self.finished_lines, Stack::new(max_height));
                let height = finished_lines.height();
                let entity = data
                    .entities
                    .build_entity()
                    .with(finished_lines, &mut data.stacks)
                    .build();
                return Some(LineStack {
                    entity,
                    block_size: height,
                    page_break_after: true
                });
            }
        }

        None
    }

    fn layout_lines(
        &mut self,
        entities: &[Entity],
        data: &mut InlineFormatterData,
    ) -> Option<LineStack> {
        loop {
            let mut iter = entities.get(self.state.current_element_index..)?.iter();
            for &elem in &mut iter {
                if let Some(stack) = self.layout_element(elem, data) {
                    return Some(stack);
                }
                // TODO: last stack
            }
            // iter is exhausted
            self.state.next_element(entities, &data);
        }
    }

    /// Returns an iterator, returning stacks of lines.
    pub fn iter<'refs, 'data>(
        &'refs mut self,
        entities: &'refs [Entity],
        data: &'refs mut InlineFormatterData<'refs, 'data>,
    ) -> InlineFormatterIter<'refs, 'data> {
        InlineFormatterIter {
            inline_formatter: self,
            entities,
            data
        }
    }

    fn undo_current_line(&mut self) {
        if let Some(saved_state) = self.saved_state.take() {
            self.state = saved_state;
        }
        self.current_line.clear();
    }
}

pub struct InlineFormatterIter<'refs, 'data> {
    inline_formatter: &'refs mut InlineFormatter,
    entities: &'refs [Entity],
    data: &'refs mut InlineFormatterData<'refs, 'data>,
}

impl<'refs, 'data> Iterator for InlineFormatterIter<'refs, 'data> {
    type Item = LineStack;

    fn next(&mut self) -> Option<Self::Item> {
        self.inline_formatter.layout_lines(self.entities, self.data)
    }
}

/// Contains entities and keeps track of their total height.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    entities: Vec<Entity>,
    total_height: Px,
    pub max_height: Px,
}

impl Component for Stack {
    type Storage = DenseVecStorage<Self>;
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

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
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
    pub fn append_fragment(
        &mut self,
        fragment: Entity,
        positions: &mut WriteStorage<'_, Position>,
    ) {
        let position = positions
            .get_mut(fragment)
            .expect("Inline fragment needs to have an assigned Position");
        self.inline_fragments.push(fragment);
        position.x = self.cursor;
        self.cursor += position.width;
    }

    /// Finish the line construction and set the final positions of the line fragments.
    pub fn build(
        &self,
        entities: &Entities<'_>,
        texts: impl GenericReadStorage<Component = Text>,
        mut inline_pos: impl GenericWriteStorage<Component = InlinePos>,
        positions: &mut WriteStorage<'_, Position>,
    ) -> Option<Entity> {
        let line_entity = if self.inline_fragments.is_empty() {
            return None;
        } else {
            entities.create()
        };

        // Align the fragments along the baseline.
        // First find max ascender and max descender...
        let get_ascent = |entity, positions: &WriteStorage<'_, _>| {
            let &Position { height, .. } = positions.get(entity).unwrap();
            texts.get(entity).map(|t| t.baseline).unwrap_or(height)
        };
        let get_descent = |entity, positions: &WriteStorage<'_, _>| {
            let &Position { height, .. } = positions.get(entity).unwrap();
            texts
                .get(entity)
                .map(|t| height - t.baseline)
                .unwrap_or(Px(0.0))
        };
        let mut max_ascent = Px(0.0);
        let mut max_descent = Px(0.0);
        for &fragment in &self.inline_fragments {
            inline_pos
                .insert(fragment, InlinePos { line: line_entity })
                .unwrap();

            if get_ascent(fragment, &positions) > max_ascent {
                max_ascent = get_ascent(fragment, &positions)
            }
            if get_descent(fragment, &positions) > max_descent {
                max_descent = get_descent(fragment, &positions)
            }
        }

        // ...then shift the individual fragments to align at the baseline.
        for &fragment in &self.inline_fragments {
            let ascent = get_ascent(fragment, &positions);
            let pos = positions.get_mut(fragment).unwrap();
            pos.y = max_ascent - ascent;
        }

        let pos = Position {
            height: max_ascent + max_descent,
            width: self.cursor,
            ..Default::default() //< Position gets assigned by block formatter.
        };
        positions.insert(line_entity, pos).unwrap();

        Some(line_entity)
    }

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

pub struct InlinePositionAssign;

use super::entity::StackingContextRelativePosition;

impl<'a> System<'a> for InlinePositionAssign {
    type SystemData = (
        Entities<'a>,
        WriteStorage<'a, Position>,
        ReadStorage<'a, StackingContextRelativePosition>,
        ReadStorage<'a, InlinePos>,
    );

    fn run(&mut self, (entities, mut positions, global_pos, inline_pos): Self::SystemData) {
        for (entity, &InlinePos { line }) in (&entities, &inline_pos).join() {
            let line_pos = global_pos.get(line).cloned().unwrap();
            let self_pos = positions.get_mut(entity).unwrap();

            self_pos.x += line_pos.x;
            self_pos.y += line_pos.y;
        }
    }
}

#[derive(Debug, Clone)]
pub struct LineStack {
    entity: Entity,
    pub block_size: Px,
    pub page_break_after: bool,
}
