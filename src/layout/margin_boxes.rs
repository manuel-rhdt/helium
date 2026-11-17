//! Headers and footers: the margin boxes of `@page` rules.
//!
//! The margin boxes are resolved once ([`MarginBoxes`]) and painted onto every page after
//! pagination ([`MarginBoxPaint`]), when the number of pages is known.
//!
//! Only the six top and bottom margin boxes are supported. Their text is laid out on a single
//! line, aligned with `text-align` inside the content area of the page and centered vertically in
//! the margin.

use super::FontMap;
use super::components::{PageGeometry, Paragraph, PlacedColorGlyph, Position, TextFace, TextOperator, ZIndex};
use super::inline::{ShapedLine, shape_line};
use super::pages::{PageContext, PageMargins, Pages, normalize};
use super::counters::format_counter;
use crate::style::parse::{ContentItem, CounterName, MarginBoxKind};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::style::properties::computed::{Language, PixelLength as Px, TextAlign};
use crate::style::properties::specified::TextTransform;
use specs::{Entities, Read, ReadExpect, System, WriteStorage};

/// A margin box whose style and font are resolved.
#[derive(Debug, Clone)]
pub struct ResolvedMarginBox {
    pub kind: MarginBoxKind,
    pub content: Vec<ContentItem>,
    /// What the text is set in. Its fallback fonts are the ones of every cluster: the text is
    /// put together when the number of the page is known.
    pub face: TextFace,
    pub text_transform: TextTransform,
    pub lang: Language,
    pub text_align: TextAlign,
}

/// The margin boxes of the kinds of pages that the `@page` rules tell apart.
#[derive(Debug, Clone, Default)]
pub struct MarginBoxes {
    pub names: HashSet<Arc<str>>,
    pub pages: HashMap<PageContext, Vec<ResolvedMarginBox>>,
}

impl MarginBoxes {
    /// Whether the document has any headers or footers at all.
    pub fn is_empty(&self) -> bool {
        self.pages.values().all(|boxes| boxes.is_empty())
    }

    fn for_page(&self, page: &PageContext) -> &[ResolvedMarginBox] {
        self.pages
            .get(&normalize(page, &self.names))
            .map_or(&[], Vec::as_slice)
    }
}

fn content_text(content: &[ContentItem], page: usize, pages: usize) -> String {
    content
        .iter()
        .map(|item| match item {
            ContentItem::String(string) => string.clone(),
            ContentItem::Counter(CounterName::Page, style) => format_counter(page as i64, *style),
            ContentItem::Counter(CounterName::Pages, style) => format_counter(pages as i64, *style),
        })
        .collect()
}

/// How the number of a page is written in its label in the PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelStyle {
    Decimal,
    LowerRoman,
    UpperRoman,
    LowerLetters,
    UpperLetters,
}

/// A range of pages with the same kind of label: from `first_page` (counted from 0) to the page
/// before the start of the next range. The number of a page is its index plus 1, like
/// `counter(page)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLabelRange {
    pub first_page: usize,
    pub style: LabelStyle,
    pub prefix: String,
}

/// The ranges of the labels of the pages, a resource of the world of the document.
#[derive(Debug, Clone, Default)]
pub struct PageLabels(pub Vec<PageLabelRange>);

/// The margin boxes that can hold the number of the page, in the order in which the first one
/// that does decides the label: the footer before the header, and the middle before the sides.
const FOLIO_ORDER: [MarginBoxKind; 6] = [
    MarginBoxKind::BottomCenter,
    MarginBoxKind::BottomRight,
    MarginBoxKind::BottomLeft,
    MarginBoxKind::TopCenter,
    MarginBoxKind::TopRight,
    MarginBoxKind::TopLeft,
];

/// The label of a page, from the number that its margin boxes print: the literal text before
/// `counter(page)` is the prefix and the style of the counter is the style of the number. `None`
/// if no margin box of the page prints the number. A box that prints `counter(pages)` before the
/// number of the page, or text after it, is not described by a prefix and a number; the text
/// after the number is left out.
fn folio(boxes: &[ResolvedMarginBox]) -> Option<(LabelStyle, String)> {
    use crate::style::parse::CounterStyle;

    FOLIO_ORDER.iter().find_map(|kind| {
        let margin_box = boxes.iter().find(|margin_box| margin_box.kind == *kind)?;
        let mut prefix = String::new();
        for item in &margin_box.content {
            match item {
                ContentItem::String(string) => prefix.push_str(string),
                ContentItem::Counter(CounterName::Pages, _) => return None,
                ContentItem::Counter(CounterName::Page, style) => {
                    // the other styles (leading zeros, scripts, symbols) are numbers in decimal
                    let style = match style {
                        CounterStyle::LowerRoman => LabelStyle::LowerRoman,
                        CounterStyle::UpperRoman => LabelStyle::UpperRoman,
                        CounterStyle::LowerAlpha => LabelStyle::LowerLetters,
                        CounterStyle::UpperAlpha => LabelStyle::UpperLetters,
                        _ => LabelStyle::Decimal,
                    };
                    return Some((style, prefix));
                }
            }
        }
        None
    })
}

/// The ranges of the labels of the pages: the label of a page is the number that it prints in
/// its margin boxes (see [`folio`]), so that the page box of a viewer says what the page does. A
/// page that prints no number has the number of the page in decimal. Empty if all labels are the
/// numbers of the pages in decimal, which the PDF needs no `/PageLabels` for.
///
/// Must run after the margin boxes are known, like [`MarginBoxPaint`].
pub fn page_label_ranges(world: &specs::World) -> Vec<PageLabelRange> {
    use specs::{Join, WorldExt};

    let margin_boxes = world.read_resource::<MarginBoxes>();
    if margin_boxes.is_empty() {
        return vec![];
    }
    let pages = world.read_resource::<Pages>();
    let page_margins = world.read_resource::<PageMargins>();
    let positions = world.read_storage::<Position>();
    let num_pages = (&positions).join().map(|pos| pos.page + 1).max().unwrap_or(0);

    let mut ranges: Vec<PageLabelRange> = Vec::new();
    for page in 0..num_pages {
        let context = pages.0.get(page).cloned().unwrap_or(PageContext {
            index: page,
            name: None,
            first_in_group: false,
            rtl: page_margins.rtl,
        });
        let (style, prefix) = folio(margin_boxes.for_page(&context)).unwrap_or((LabelStyle::Decimal, String::new()));
        if ranges.last().is_none_or(|last| last.style != style || last.prefix != prefix) {
            ranges.push(PageLabelRange { first_page: page, style, prefix });
        }
    }
    if let [only] = ranges.as_slice() {
        if only.style == LabelStyle::Decimal && only.prefix.is_empty() {
            return vec![];
        }
    }
    ranges
}

/// Creates the paragraphs of the margin boxes of every page.
pub struct MarginBoxPaint;

impl<'a> System<'a> for MarginBoxPaint {
    type SystemData = (
        Entities<'a>,
        Read<'a, MarginBoxes>,
        Read<'a, FontMap>,
        Read<'a, Pages>,
        Read<'a, PageMargins>,
        ReadExpect<'a, PageGeometry>,
        WriteStorage<'a, Position>,
        WriteStorage<'a, Paragraph>,
    );

    fn run(
        &mut self,
        (entities, margin_boxes, font_map, pages, page_margins, geometry, mut positions, mut paragraphs): Self::SystemData,
    ) {
        use specs::Join;

        if margin_boxes.is_empty() {
            return;
        }
        let num_pages = (&positions)
            .join()
            .map(|pos| pos.page + 1)
            .max()
            .unwrap_or(0);

        for page in 0..num_pages {
            let context = pages.0.get(page).cloned().unwrap_or(PageContext {
                index: page,
                name: None,
                first_in_group: false,
                rtl: page_margins.rtl,
            });
            // the margins of the page: the inner and outer margins of a book differ, and the
            // first page of a document or of a page group can have other margins
            let edges = page_margins.edges(&context);
            let (margin_left, margin_right) = (edges.left, edges.right);
            let (page_width, page_height) =
                page_margins.size(&context, (geometry.width, geometry.height));
            let area_left = margin_left;
            let area_width = page_width - margin_left - margin_right;
            for margin_box in margin_boxes.for_page(&context) {
                let mut text = content_text(&margin_box.content, page + 1, num_pages);
                if let Some(transformed) = crate::layout::text::transform::apply(
                    &text,
                    margin_box.text_transform,
                    &margin_box.lang,
                    &mut false,
                ) {
                    text = transformed;
                }
                if text.is_empty() {
                    continue;
                }
                let face = &margin_box.face;
                let font = &font_map[&face.font_name];
                // relative to the start of the text on the baseline until the box is placed
                let ShapedLine { mut ops, width, color_glyphs } =
                    shape_line(&text, font, face, page_margins.rtl);
                ops.insert(0, TextOperator::BeginArtifact);
                ops.push(TextOperator::EndMarkedContent);

                let x = match margin_box.text_align.resolve(page_margins.rtl) {
                    TextAlign::Center => area_left + (area_width - width) / 2.0,
                    TextAlign::Right => area_left + area_width - width,
                    TextAlign::Left | TextAlign::Justify | TextAlign::Start | TextAlign::End => {
                        area_left
                    }
                };

                let ascent = font.scale_to_px(font.ascent(), face.font_size);
                let descent = font.scale_to_px(font.descent(), face.font_size).abs();
                let height = ascent + descent;
                let (margin_top, margin_height) = match margin_box.kind {
                    MarginBoxKind::TopLeft | MarginBoxKind::TopCenter | MarginBoxKind::TopRight => {
                        (Px(0.0), edges.top)
                    }
                    MarginBoxKind::BottomLeft
                    | MarginBoxKind::BottomCenter
                    | MarginBoxKind::BottomRight => {
                        (page_height - edges.bottom, edges.bottom)
                    }
                };
                let y = margin_top + (margin_height - height) / 2.0;

                entities
                    .build_entity()
                    .with(
                        Position {
                            x,
                            y,
                            width,
                            height,
                            // last in the content stream, so that the page numbers come after the
                            // text of the page for extractors that read it in stream order
                            z: ZIndex(i64::MAX),
                            page,
                        },
                        &mut positions,
                    )
                    .with(
                        Paragraph {
                            start_pos: (x, y + ascent),
                            ops,
                            color_glyphs: color_glyphs
                                .into_iter()
                                .map(|glyph| PlacedColorGlyph {
                                    x: glyph.x + x,
                                    baseline: glyph.baseline + y + ascent,
                                    ..glyph
                                })
                                .collect(),
                        },
                        &mut paragraphs,
                    )
                    .build();
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::style::parse::{CounterStyle, PagePseudo, PageSelector, Styles};

    fn margin_box(kind: MarginBoxKind, content: Vec<ContentItem>) -> ResolvedMarginBox {
        ResolvedMarginBox {
            kind,
            content,
            face: Default::default(),
            text_transform: Default::default(),
            lang: Default::default(),
            text_align: TextAlign::Center,
        }
    }

    #[test]
    fn the_label_of_a_page_is_the_number_that_it_prints() {
        use crate::style::parse::CounterStyle;

        let number = |style| ContentItem::Counter(CounterName::Page, style);
        let text = |text: &str| ContentItem::String(text.into());
        // no number
        assert_eq!(folio(&[]), None);
        assert_eq!(folio(&[margin_box(MarginBoxKind::TopLeft, vec![text("Title")])]), None);
        // the style and the text before the number
        assert_eq!(
            folio(&[margin_box(MarginBoxKind::BottomCenter, vec![text("A-"), number(CounterStyle::UpperRoman), text(" of")])]),
            Some((LabelStyle::UpperRoman, "A-".to_string()))
        );
        // styles that the PDF has no label for are decimal
        assert_eq!(
            folio(&[margin_box(MarginBoxKind::BottomCenter, vec![number(CounterStyle::DecimalLeadingZero)])]),
            Some((LabelStyle::Decimal, String::new()))
        );
        // the footer before the header, and the middle before the sides
        let boxes = [
            margin_box(MarginBoxKind::TopCenter, vec![number(CounterStyle::LowerAlpha)]),
            margin_box(MarginBoxKind::BottomRight, vec![number(CounterStyle::LowerRoman)]),
            margin_box(MarginBoxKind::BottomLeft, vec![number(CounterStyle::UpperRoman)]),
        ];
        assert_eq!(folio(&boxes), Some((LabelStyle::LowerRoman, String::new())));
        // a total of pages before the number is not a prefix
        let boxes = [margin_box(
            MarginBoxKind::BottomCenter,
            vec![ContentItem::Counter(CounterName::Pages, CounterStyle::Decimal), number(CounterStyle::Decimal)],
        )];
        assert_eq!(folio(&boxes), None);
    }

    #[test]
    fn counters_are_formatted() {
        let content = [
            ContentItem::String("Page ".into()),
            ContentItem::Counter(CounterName::Page, CounterStyle::Decimal),
            ContentItem::String(" of ".into()),
            ContentItem::Counter(CounterName::Pages, CounterStyle::LowerRoman),
        ];
        assert_eq!(content_text(&content, 2, 14), "Page 2 of xiv");
    }

    #[test]
    fn counters_are_formatted_in_every_style() {
        let format = |style, value| {
            content_text(&[ContentItem::Counter(CounterName::Page, style)], value, 1)
        };

        assert_eq!(format(CounterStyle::Decimal, 1), "1");
        assert_eq!(format(CounterStyle::DecimalLeadingZero, 1), "01");
        assert_eq!(format(CounterStyle::DecimalLeadingZero, 27), "27");
        assert_eq!(format(CounterStyle::LowerRoman, 4), "iv");
        assert_eq!(format(CounterStyle::UpperRoman, 4), "IV");
        assert_eq!(format(CounterStyle::LowerAlpha, 1), "a");
        assert_eq!(format(CounterStyle::UpperAlpha, 1), "A");
        // past z the alphabetic styles continue with aa, ab, …
        assert_eq!(format(CounterStyle::LowerAlpha, 26), "z");
        assert_eq!(format(CounterStyle::LowerAlpha, 27), "aa");
        // roman numerals and letters start at 1, so 0 falls back to decimal
        assert_eq!(format(CounterStyle::LowerRoman, 0), "0");
        assert_eq!(format(CounterStyle::LowerAlpha, 0), "0");
        assert_eq!(format(CounterStyle::ArabicIndic, 1405), "١٤٠٥");
        assert_eq!(format(CounterStyle::Persian, 90), "۹۰");
    }

    #[test]
    fn unsupported_counter_styles_fall_back_to_decimal() {
        let mut styles = Styles::default();
        styles.parse_css(
            "@page { @bottom-center { content: counter(page, cjk-ideographic) \" / \" counter(pages) } }",
        );
        let rule = styles.rules_iter().last().unwrap();
        assert_eq!(
            rule.margin_boxes[0].content,
            Some(vec![
                ContentItem::Counter(CounterName::Page, CounterStyle::Decimal),
                ContentItem::String(" / ".into()),
                ContentItem::Counter(CounterName::Pages, CounterStyle::Decimal),
            ])
        );
    }

    #[test]
    fn unsupported_content_functions_keep_the_rest_of_the_footer() {
        let mut styles = Styles::default();
        styles.parse_css(
            "@page { @bottom-center { content: \"p. \" counter(page) string(chapter) attr(class) } }",
        );
        let rule = styles.rules_iter().last().unwrap();
        assert_eq!(
            rule.margin_boxes[0].content,
            Some(vec![
                ContentItem::String("p. ".into()),
                ContentItem::Counter(CounterName::Page, CounterStyle::Decimal),
            ])
        );
    }

    #[test]
    fn margin_boxes_are_parsed() {
        let mut styles = Styles::default();
        styles.parse_css(
            "@page :first { @top-left { content: \"Hi\" counter(page, upper-roman); color: red } @bogus { } }",
        );
        let rule = styles.rules_iter().last().unwrap();
        assert_eq!(
            rule.page_selector(),
            Some(&PageSelector::pseudo(PagePseudo::First))
        );
        assert_eq!(rule.margin_boxes.len(), 1);
        let margin_box = &rule.margin_boxes[0];
        assert_eq!(margin_box.kind, MarginBoxKind::TopLeft);
        assert_eq!(
            margin_box.content,
            Some(vec![
                ContentItem::String("Hi".into()),
                ContentItem::Counter(CounterName::Page, CounterStyle::UpperRoman),
            ])
        );
    }
}
