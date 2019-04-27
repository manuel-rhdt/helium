//! Contains the builtin PDF fonts

pub use lemon_pdf::font::builtin::BuiltInFont;
pub use lemon_pdf::font::FontUnit;

use crate::style::properties::computed::PixelLength;

use std::collections::VecDeque;
use std::str::Chars;

impl From<FontUnit> for PixelLength {
    fn from(fu: FontUnit) -> PixelLength {
        PixelLength((fu.0 / 1000.0) as _)
    }
}

pub fn scale(fu: FontUnit, font_size: PixelLength) -> PixelLength {
    font_size * PixelLength::from(fu)
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InlineFragment {
    pub glyph_indices: Vec<u32>,
    pub advance: PixelLength,
    pub new_line_after: bool,
}

impl InlineFragment {
    pub fn new_line() -> Self {
        InlineFragment {
            new_line_after: true,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.glyph_indices.is_empty()
    }
}

#[derive(Debug, Copy, Clone)]
pub struct LineIsFullError;

#[derive(Debug, Clone)]
pub struct InlineFragmentIteratorState {
    /// an index into the text content of the element describing the current
    /// progress
    progress: usize,
    /// An index into the `glyph_indices` buffer, pointing to the last breakpoint.
    last_breakpoint: usize,
    /// The cursor position just before `last_breakpoint`.
    last_line_advance: PixelLength,
    /// The cursor position just after `last_breakpoint`.
    next_line_start: PixelLength,
    /// the current cursor position
    cursor: PixelLength,
    /// a buffer of glyph indices.
    glyph_indices: VecDeque<u32>,
    /// The font used to determine the glyphs and their advances.
    font: BuiltInFont,
    /// The font size in pixels used to scale the glyph metrics
    font_size: PixelLength,
    pub remaining_line: PixelLength,
    pub line_width: PixelLength,
}

impl InlineFragmentIteratorState {
    pub fn new(
        font: BuiltInFont,
        font_size: PixelLength,
        line_width: PixelLength,
        remaining_line: PixelLength,
    ) -> Self {
        InlineFragmentIteratorState {
            progress: 0,
            last_breakpoint: 0,
            last_line_advance: PixelLength(0.0),
            next_line_start: PixelLength(0.0),
            cursor: PixelLength(0.0),
            glyph_indices: VecDeque::new(),
            font,
            font_size,
            line_width,
            remaining_line,
        }
    }

    fn push_character(&mut self, character: char) -> Result<(), LineIsFullError> {
        self.progress += character.len_utf8();
        if character == ' ' {
            self.last_breakpoint = self.glyph_indices.len();
            self.last_line_advance = self.cursor;
        }

        let char_index = (self.font.metrics().unicode_mapping)(character);
        let char_metric = char_index
            .and_then(|index| self.font.metrics().char_metrics.get(index))
            .unwrap();
        self.glyph_indices.push_back(
            char_index
                .unwrap_or_else(|| panic!("Could not find character {:?} in font.", character))
                as u32,
        );

        self.cursor += scale(char_metric.advance_width, self.font_size);

        if character == ' ' {
            self.next_line_start = self.cursor;
        }

        if self.cursor > self.remaining_line {
            Err(LineIsFullError)
        } else {
            Ok(())
        }
    }

    fn create_line(&mut self, last_line: bool) -> Option<InlineFragment> {
        let finished_line: Vec<_> = if last_line {
            self.glyph_indices.drain(..).collect()
        } else {
            self.glyph_indices.drain(..self.last_breakpoint).collect()
        };
        
        if !finished_line.is_empty() {
            // remove whitespace at beginning of new line.
            self.glyph_indices.pop_front();

            let advance = self.last_line_advance;

            // reset some line-dependent values
            self.remaining_line = self.line_width;
            self.cursor = self.cursor - self.next_line_start;
            self.last_line_advance = PixelLength(0.0);
            self.next_line_start = PixelLength(0.0);
            self.last_breakpoint = 0;

            Some(InlineFragment {
                glyph_indices: finished_line,
                advance,
                new_line_after: !last_line,
            })
        } else {
            self.remaining_line = self.line_width;
            None
        }
    }

    pub fn iter<'a>(
        &self,
        text: &'a str,
    ) -> impl 'a + Iterator<Item = Result<InlineFragment, LineIsFullError>> {
        InlineFragmentIterator {
            state: self.clone(),
            text: text[self.progress..].chars(),
        }
    }
}

pub struct InlineFragmentIterator<'a> {
    state: InlineFragmentIteratorState,
    text: Chars<'a>,
}

impl<'a> Iterator for InlineFragmentIterator<'a> {
    type Item = Result<InlineFragment, LineIsFullError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(character) = self.text.next() {
                if let Err(LineIsFullError) = self.state.push_character(character) {
                    if let Some(line) = self.state.create_line(false) {
                        break Some(Ok(line));
                    } else {
                        break Some(Err(LineIsFullError));
                    }
                }
            } else {
                // Return the last line, if any fragments are still remaining.
                //
                // Since `create_line` returns `None` if no fragments can be created, we also return
                // `None` to indicate that the iterator is depleted.
                break self.state.create_line(true).map(Result::Ok);
            }
        }
    }
}
