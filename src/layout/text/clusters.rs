//! The glyph clusters of the pieces of a [`TextOps`](crate::layout::components::TextOps).
//!
//! A shaped piece says for every glyph which characters it was shaped from (its cluster). This
//! module groups the glyphs by cluster and finds the units of text that can be given as a whole:
//! what a backend does with them (the `ToUnicode` map and `/ActualText` of PDF, the characters
//! of a word in HTML) is its own business.

use crate::layout::components::{KernedGlyph, NO_CLUSTER, PieceKind, TextPiece};
use crate::layout::text::is_invisible_formatting;
use unicode_segmentation::UnicodeSegmentation;

/// A glyph cluster of a piece: glyphs that were shaped from the same characters, like the
/// glyphs of a conjunct with its vowel sign or a reordered vowel sign and its consonant.
#[derive(Debug, Clone, Copy)]
pub struct Cluster<'a> {
    /// The byte offset of its first character in the text of the piece.
    pub start: usize,
    /// Its characters: up to the start of the next cluster in the text of the piece, without
    /// the invisible formatting characters that were dropped in front of that. Empty if the
    /// piece has no text for the cluster.
    pub text: &'a str,
    /// Its glyphs, as indices of [`TextOps::glyphs`](crate::layout::components::TextOps::glyphs),
    /// in the order in which they are drawn.
    pub glyphs: &'a [u32],
}

/// The clusters of one piece, in the order of their text. The buffers are kept between pieces:
/// this runs for every piece of text of a document.
#[derive(Debug, Default)]
pub struct PieceClusters {
    /// The indices of the glyphs of the piece that have a cluster, ordered by cluster. The glyphs
    /// of a cluster stay in the order in which they are drawn.
    order: Vec<u32>,
    /// The start of every cluster and the range of `order` that holds its glyphs.
    groups: Vec<(usize, std::ops::Range<usize>)>,
    /// Whether the text of the piece is kept as it is: only the text of a shaped piece has
    /// invisible formatting characters to leave out.
    verbatim: bool,
}

impl PieceClusters {
    /// Groups the glyphs of `piece` by cluster.
    pub fn set(&mut self, glyphs: &[KernedGlyph], piece: &TextPiece) {
        self.verbatim = piece.kind != PieceKind::Shaped;
        self.order.clear();
        self.order.extend(
            (piece.glyphs.start..piece.glyphs.end).filter(|&index| glyphs[index as usize].cluster != NO_CLUSTER),
        );
        // The glyphs of left-to-right text are in the order of their clusters already, those of
        // right-to-left text are not. The stable sort keeps the order of the glyphs of a cluster.
        let cluster = |&index: &u32| glyphs[index as usize].cluster;
        if !self.order.is_sorted_by_key(cluster) {
            self.order.sort_by_key(cluster);
        }
        self.groups.clear();
        let mut from = 0;
        for chunk in self.order.chunk_by(|a, b| cluster(a) == cluster(b)) {
            let start = cluster(&chunk[0]) as usize;
            self.groups.push((start, from..from + chunk.len()));
            from += chunk.len();
        }
    }

    /// The clusters of the piece, whose text is `text`.
    pub fn iter<'a>(&'a self, text: &'a str) -> impl Iterator<Item = Cluster<'a>> {
        self.groups.iter().enumerate().map(move |(i, (start, range))| {
            let end = self.groups.get(i + 1).map_or(text.len(), |(next, _)| *next);
            Cluster {
                start: *start,
                text: text.get(*start..end).map_or("", |text| self.trimmed(text)),
                glyphs: &self.order[range.clone()],
            }
        })
    }

    /// `text` as the text of a cluster or unit of the piece.
    fn trimmed<'a>(&self, text: &'a str) -> &'a str {
        if self.verbatim { text } else { trim_invisible(text) }
    }

    /// The text of the unit that starts at `start` (see [`unit_start`]).
    pub fn unit_text<'a>(&self, text: &'a str, boundaries: &[usize], start: usize) -> &'a str {
        let next = boundaries.partition_point(|&b| b <= start);
        let end = boundaries.get(next).copied().unwrap_or(text.len());
        self.trimmed(&text[start..end])
    }

    /// The units of text of the piece, which are made of whole clusters.
    ///
    /// A unit is a grapheme cluster: a base character with its marks, whose glyphs an extractor
    /// would otherwise have to assemble from the positions of the marks. A unit has to hold
    /// whole glyph clusters too, since a glyph cannot be split between two texts: a conjunct
    /// with its vowel sign (Kannada "ಲ್ಲಾ", Sinhala "ක්‍රි") is several grapheme clusters but one
    /// glyph cluster. So a boundary between units is a grapheme boundary where a cluster starts.
    pub fn unit_boundaries(&self, text: &str) -> Vec<usize> {
        text.grapheme_indices(true)
            .map(|(offset, _)| offset)
            .filter(|&offset| offset == 0 || self.groups.binary_search_by_key(&offset, |(start, _)| *start).is_ok())
            .collect()
    }
}

/// The start of the unit that contains the byte `offset`, for the `boundaries` of
/// [`PieceClusters::unit_boundaries`].
pub fn unit_start(boundaries: &[usize], offset: usize) -> usize {
    boundaries[boundaries.partition_point(|&b| b <= offset).saturating_sub(1)]
}

/// `text` without the invisible formatting characters at its end, which have no glyphs. The
/// joiners stay: they are part of the spelling in Indic and Arabic scripts (a Malayalam chillu,
/// a Sinhala yansaya, a Persian half-space).
pub fn trim_invisible(text: &str) -> &str {
    text.trim_end_matches(|ch| is_invisible_formatting(ch) && !matches!(ch, '\u{200C}' | '\u{200D}'))
}
