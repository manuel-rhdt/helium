//! How the glyphs of the display list are written as the strings of `Tj` and `TJ` operators.
//!
//! The text of the display list is made of glyph indices of fonts. The encoding of the embedded
//! font (see [`GlyphEncoding`]) turns them into the codes of the strings: two bytes each, big
//! endian.

use crate::font_usage::FontUsage;
use crate::pdf::encoding::GlyphEncoding;
use crate::layout::components::{GlyphRange, KernedGlyph, KernedItem, NO_CLUSTER, PieceKind, TextOperator, TextOps};
use crate::layout::text::clusters::{PieceClusters, trim_invisible, unit_start};
use crate::layout::text::font::GlyphIndex;

/// Writes glyphs as a literal string of 16-bit glyph indices. A literal string is about half the
/// size of a hex string. Only the bytes that are special inside a literal string need an escape.
/// The string is written in chunks from a buffer on the stack: a document has a string for
/// almost every word.
fn write_glyph_string(
    f: &mut lemon_pdf::object::Formatter,
    encoding: &GlyphEncoding,
    glyphs: impl Iterator<Item = GlyphIndex>,
) -> std::io::Result<()> {
    use std::io::Write;

    let mut buffer = [0u8; 256];
    let mut len = 0;
    let mut open = false;
    for glyph in glyphs {
        if !open {
            buffer[0] = b'(';
            len = 1;
            open = true;
        }
        // a glyph takes at most four bytes, and the closing parenthesis one
        if len + 5 > buffer.len() {
            f.write_all(&buffer[..len])?;
            len = 0;
        }
        for byte in encoding.code(glyph).to_be_bytes() {
            match byte {
                b'(' | b')' | b'\\' => {
                    buffer[len] = b'\\';
                    buffer[len + 1] = byte;
                    len += 2;
                }
                b'\r' => {
                    buffer[len] = b'\\';
                    buffer[len + 1] = b'r';
                    len += 2;
                }
                _ => {
                    buffer[len] = byte;
                    len += 1;
                }
            }
        }
    }
    if open {
        buffer[len] = b')';
        f.write_all(&buffer[..len + 1])?;
    }
    Ok(())
}

/// The operand of a `TJ` operator: the elements of `range` as a PDF array, for a font with the
/// given encoding.
#[derive(Debug)]
pub struct KernedArray<'a> {
    range: GlyphRange,
    glyphs: &'a [KernedGlyph],
    encoding: &'a GlyphEncoding,
}

impl<'a> KernedArray<'a> {
    pub fn new(range: GlyphRange, glyphs: &'a [KernedGlyph], encoding: &'a GlyphEncoding) -> Self {
        KernedArray { range, glyphs, encoding }
    }
}

#[derive(Debug)]
struct GlyphString<'a>(&'a [KernedGlyph], &'a GlyphEncoding);

impl lemon_pdf::PdfFormat for GlyphString<'_> {
    fn write(&self, f: &mut lemon_pdf::object::Formatter) -> std::io::Result<()> {
        write_glyph_string(f, self.1, self.0.iter().map(|glyph| glyph.glyph))
    }
}

impl lemon_pdf::PdfFormat for KernedArray<'_> {
    fn write(&self, f: &mut lemon_pdf::object::Formatter) -> std::io::Result<()> {
        let mut array = Some(f.format_array());
        self.range.for_each_item(self.glyphs, |item| {
            let current = array.take().expect("the array is put back after every element");
            array = Some(match item {
                KernedItem::Glyphs(glyphs) => current.value(&GlyphString(glyphs, self.encoding)),
                KernedItem::Kern(kern) => current.value(&kern),
            });
        });
        array.expect("the array is put back after every element").finish()
    }
}

/// The text that a span of the glyphs of a paragraph is given as `/ActualText`.
struct Span {
    text: String,
    /// The last glyph of the span, as an index of [`TextOps::glyphs`].
    last_glyph: usize,
}

/// The operators of `ops` with the `/ActualText` that the text of its glyphs needs, or `None` if
/// it needs none, which is the case for most text.
///
/// The `ToUnicode` map of a font says what text a glyph stands for. It cannot describe a cluster
/// of several glyphs (a reordered vowel sign and its consonant), a glyph that is used for
/// different text, or the glyphs of right-to-left text, which are in visual order. That text is
/// given where the glyphs are drawn: the unit of text that they are part of (a grapheme cluster
/// or whole glyph clusters) for the first two and the whole piece for right-to-left text
/// (ISO 32000-1 14.9.4). Where the text was given by an operator already (a soft hyphen), it
/// stays as it is.
pub(crate) fn with_actual_text(ops: &TextOps, usage: &FontUsage) -> Option<Vec<TextOperator>> {
    if ops.pieces.is_empty() {
        return None;
    }
    let fonts = ops.glyph_fonts();
    let mut spans: Vec<Span> = vec![];
    // the span of every glyph, once there is one
    let mut span_of: Vec<Option<u32>> = vec![];
    let mut clusters = PieceClusters::default();
    for piece in &ops.pieces {
        let glyphs = piece.glyphs.start as usize..piece.glyphs.end as usize;
        let text = ops.piece_text(piece);
        let mut found: Vec<(u32, Span)> = vec![];
        if piece.rtl || piece.kind == PieceKind::Inserted {
            // The glyphs of right-to-left text are in visual order, but the text is in logical
            // order. The text of inserted glyphs is not what their font says.
            let text = if piece.kind == PieceKind::Shaped { trim_invisible(text) } else { text };
            found.push((u32::MAX, Span { text: text.to_owned(), last_glyph: glyphs.end - 1 }));
        } else {
            clusters.set(&ops.glyphs, piece);
            let mut boundaries = vec![];
            let mut unclean = std::collections::BTreeSet::new();
            for cluster in clusters.iter(text) {
                if cluster.text.is_empty() {
                    continue;
                }
                let clean = match cluster.glyphs {
                    &[only] => {
                        let glyph = ops.glyphs[only as usize].glyph;
                        let font = fonts[only as usize];
                        usage.font(font).is_none_or(|usage| usage.text(glyph) == Some(cluster.text))
                    }
                    _ => false,
                };
                if !clean {
                    if boundaries.is_empty() {
                        boundaries = clusters.unit_boundaries(text);
                    }
                    unclean.insert(unit_start(&boundaries, cluster.start));
                }
            }
            if unclean.is_empty() {
                continue;
            }
            for &start in &unclean {
                found.push((start as u32, Span { text: clusters.unit_text(text, &boundaries, start).to_owned(), last_glyph: 0 }));
            }
            // the glyphs of a unit are marked together, whether or not they need the text
            let first_span = spans.len() as u32;
            if span_of.is_empty() {
                span_of = vec![None; ops.glyphs.len()];
            }
            for index in glyphs.clone() {
                let cluster = ops.glyphs[index].cluster;
                if cluster == NO_CLUSTER {
                    continue;
                }
                let start = unit_start(&boundaries, cluster as usize) as u32;
                if let Some(position) = found.iter().position(|(unit, _)| *unit == start) {
                    span_of[index] = Some(first_span + position as u32);
                    found[position].1.last_glyph = index;
                }
            }
            spans.extend(found.into_iter().map(|(_, span)| span));
            continue;
        }
        // one span for the whole piece
        if span_of.is_empty() {
            span_of = vec![None; ops.glyphs.len()];
        }
        let id = spans.len() as u32;
        for index in glyphs {
            span_of[index] = Some(id);
        }
        spans.extend(found.into_iter().map(|(_, span)| span));
    }
    if spans.is_empty() {
        return None;
    }

    let mut out: Vec<TextOperator> = Vec::with_capacity(ops.len() + 2 * spans.len());
    // the span whose marked content is open
    let mut open: Option<u32> = None;
    for op in ops.iter() {
        let TextOperator::KernedGlyphs(range) = op else {
            out.push(op.clone());
            continue;
        };
        // The glyphs are shown by one operator for each stretch that is in the same span: the
        // kern in front of the glyph that starts one goes in front of the new operator.
        let mut from = range.start as usize;
        let end = range.end as usize;
        let show = |out: &mut Vec<TextOperator>, from: usize, to: usize, trailing_kern: i32| {
            if to > from || trailing_kern != 0 {
                out.push(TextOperator::KernedGlyphs(GlyphRange { start: from as u32, end: to as u32, trailing_kern }));
            }
        };
        for index in from..end {
            let wanted = span_of.get(index).copied().flatten();
            if wanted != open {
                show(&mut out, from, index, 0);
                from = index;
                if open.take().is_some() {
                    out.push(TextOperator::EndMarkedContent);
                }
                if let Some(id) = wanted {
                    out.push(TextOperator::BeginActualText(spans[id as usize].text.clone()));
                    open = Some(id);
                }
            }
        }
        show(&mut out, from, end, range.trailing_kern);
        if let Some(id) = open
            && end > 0
            && spans[id as usize].last_glyph == end - 1
        {
            out.push(TextOperator::EndMarkedContent);
            open = None;
        }
    }
    debug_assert!(open.is_none(), "the marked content of a span is not ended");
    Some(out)
}
