//! Contains the builtin PDF fonts

pub use lemon_pdf::font::builtin::BuiltInFont;
pub use lemon_pdf::font::FontUnit;

use super::font::{GlyphIndex, GlyphMetricsProvide};
use crate::style::properties::computed::PixelLength;
use std::collections::VecDeque;

use std::ops::Deref;
use std::str::Chars;
impl From<FontUnit> for PixelLength {
    fn from(fu: FontUnit) -> PixelLength {
        PixelLength((fu.0 / 1000.0) as _)
    }
}

#[derive(Debug, Clone)]
pub(super) struct FontInstance {
    font: BuiltInFont,
    size: PixelLength,
}

impl FontInstance {
    pub(super) fn new(font: BuiltInFont, size: PixelLength) -> FontInstance {
        FontInstance { font, size }
    }
}

impl GlyphMetricsProvide for FontInstance {
    fn char_to_glyph_index(&self, ch: char) -> GlyphIndex {
        GlyphIndex(
            (self.font.metrics().unicode_mapping)(ch)
                .unwrap_or_else(|| panic!("Could not find character {:?} in font.", ch))
                as u32,
        )
    }

    fn glyph_advance(&self, glyph: GlyphIndex) -> PixelLength {
        scale(
            self.font
                .metrics()
                .char_metrics
                .get(glyph.0 as usize)
                .unwrap()
                .advance_width,
            self.size,
        )
    }
}

pub fn scale(fu: FontUnit, font_size: PixelLength) -> PixelLength {
    font_size * PixelLength::from(fu)
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct InlineFragment {
    pub glyph_indices: Vec<GlyphIndex>,
    pub advance: PixelLength,
    pub new_line_after: bool,
}

impl InlineFragment {
    pub fn is_empty(&self) -> bool {
        self.glyph_indices.is_empty()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LineIsFullError;

#[derive(Debug, Clone)]
pub struct InlineFragmentIteratorState<Font> {
    /// An index into the text content of the element describing the current progress.
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
    glyph_indices: VecDeque<GlyphIndex>,
    /// The font used to determine the glyphs and their advances.
    font: Font,
    /// The line width of the current line.
    pub remaining_line: PixelLength,
    /// The line width of the lines after the current line.
    pub line_width: PixelLength,
}

impl<Font, T> InlineFragmentIteratorState<Font>
where
    Font: std::ops::Deref<Target = T>,
    T: ?Sized + GlyphMetricsProvide,
{
    pub fn new(font: Font, line_width: PixelLength, remaining_line: PixelLength) -> Self {
        InlineFragmentIteratorState {
            progress: 0,
            last_breakpoint: 0,
            last_line_advance: PixelLength(0.0),
            next_line_start: PixelLength(0.0),
            cursor: PixelLength(0.0),
            glyph_indices: VecDeque::new(),
            font,
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

        let glyph_index = self.font.char_to_glyph_index(character);
        self.glyph_indices.push_back(glyph_index);

        self.cursor += self.font.glyph_advance(glyph_index);

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

    pub fn into_iterator(self, text: &str) -> InlineFragmentIterator<'_, Font> {
        let progress = self.progress;
        InlineFragmentIterator {
            state: self,
            text: text[progress..].chars(),
        }
    }
}

pub struct InlineFragmentIterator<'a, Font> {
    state: InlineFragmentIteratorState<Font>,
    text: Chars<'a>,
}

impl<'a, Font: Deref<Target = T>, T: ?Sized + GlyphMetricsProvide> Iterator
    for InlineFragmentIterator<'a, Font>
{
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

#[cfg(test)]
mod test {
    use super::*;
    use PixelLength as Px;

    struct MockFont;

    impl GlyphMetricsProvide for MockFont {
        fn char_to_glyph_index(&self, _ch: char) -> GlyphIndex {
            GlyphIndex(1)
        }

        fn glyph_advance(&self, _glyph: GlyphIndex) -> PixelLength {
            Px(100.0)
        }
    }

    #[test]
    fn test_line_breaking() {
        let inline_fragment_iterator_state =
            InlineFragmentIteratorState::new(&MockFont, Px(1000.0), Px(1000.0));

        let text = "0123 4567 89 0123 456 789";

        let mut iter = inline_fragment_iterator_state.into_iterator(&text);

        assert_eq!(9, iter.next().unwrap().unwrap().glyph_indices.len());
        assert_eq!(7, iter.next().unwrap().unwrap().glyph_indices.len());
        assert_eq!(7, iter.next().unwrap().unwrap().glyph_indices.len());
        assert_eq!(None, iter.next());
    }

    #[test]
    fn test_skip_inline_fragment_if_empty() {
        let inline_fragment_iterator_state =
            InlineFragmentIteratorState::new(&MockFont, Px(1000.0), Px(1000.0));
        let text = "";
        let mut iter = inline_fragment_iterator_state.into_iterator(&text);
        assert_eq!(None, iter.next());
    }
}
