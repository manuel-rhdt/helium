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
use crate::style::parse::{ContentItem, CounterName, MarginBoxKind, PageLabelSpec};
use crate::style::properties::generated::CounterList;
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

/// What the page context of a page does to the `page` counter (CSS Paged Media 3, § 6.1): the
/// `counter-reset`, `counter-increment` and `counter-set` of the `@page` rules that apply to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCounterOps {
    /// The value that the counter is reset to when the page box is generated (before the
    /// increment).
    pub reset: Option<i32>,
    /// What the counter is incremented by: 1 unless `counter-increment` names `page`.
    pub increment: i32,
    /// The value that the counter is set to after the increment.
    pub set: Option<i32>,
}

impl Default for PageCounterOps {
    fn default() -> Self {
        PageCounterOps { reset: None, increment: 1, set: None }
    }
}

impl PageCounterOps {
    /// The operations of the lists of `counter-reset`, `counter-increment` and `counter-set` that
    /// apply to a page. An entry for a counter other than `page` is not for us.
    pub fn from_lists(
        reset: Option<&CounterList>,
        increment: Option<&CounterList>,
        set: Option<&CounterList>,
    ) -> Self {
        // the values written for `page` in a list (`None` for no value)
        fn entries(list: Option<&CounterList>) -> Vec<Option<i32>> {
            list.map_or_else(Vec::new, |list| {
                list.0.iter().filter(|(name, _)| name == "page").map(|(_, value)| *value).collect()
            })
        }
        let mut ops = PageCounterOps::default();
        // (a counter that is named twice in a `counter-reset` takes the last value; increments add)
        ops.reset = entries(reset).last().map(|value| value.unwrap_or(0));
        let increments = entries(increment);
        if !increments.is_empty() {
            ops.increment = increments.iter().map(|value| value.unwrap_or(1)).sum();
        }
        ops.set = entries(set).last().map(|value| value.unwrap_or(0));
        ops
    }

    /// The value of the counter on a page box, given its value on the page before (0 before the
    /// first page): in the order of CSS Lists 3, reset, then increment, then set.
    pub fn next(&self, before: i64) -> i64 {
        let mut value = self.reset.map_or(before, i64::from);
        value += i64::from(self.increment);
        if let Some(set) = self.set {
            value = i64::from(set);
        }
        value
    }
}

/// The margin boxes of the kinds of pages that the `@page` rules tell apart.
#[derive(Debug, Clone, Default)]
pub struct MarginBoxes {
    pub names: HashSet<Arc<str>>,
    pub pages: HashMap<PageContext, Vec<ResolvedMarginBox>>,
    /// What the page context does to the `page` counter, by kind of page.
    pub counters: HashMap<PageContext, PageCounterOps>,
    /// The `-helium-page-label` of the kinds of pages.
    pub labels: HashMap<PageContext, PageLabelSpec>,
}

impl MarginBoxes {
    /// Whether the document has any headers or footers at all.
    pub fn is_empty(&self) -> bool {
        self.pages.values().all(|boxes| boxes.is_empty())
    }

    /// The operations on the `page` counter of a page.
    fn counter_ops(&self, page: &PageContext) -> PageCounterOps {
        self.counters.get(&normalize(page, &self.names)).copied().unwrap_or_default()
    }

    /// The `-helium-page-label` of a page.
    fn label_spec(&self, page: &PageContext) -> &PageLabelSpec {
        self.labels.get(&normalize(page, &self.names)).unwrap_or(&PageLabelSpec::Auto)
    }

    /// The value of the `page` counter on every one of the first `num_pages` pages, which are
    /// the pages of `pages`.
    pub fn page_numbers(&self, pages: &Pages, num_pages: usize, rtl: bool) -> Vec<i64> {
        let mut number = 0;
        (0..num_pages)
            .map(|index| {
                number = self.counter_ops(&page_context(pages, index, rtl)).next(number);
                number
            })
            .collect()
    }

    fn for_page(&self, page: &PageContext) -> &[ResolvedMarginBox] {
        self.pages
            .get(&normalize(page, &self.names))
            .map_or(&[], Vec::as_slice)
    }
}

/// The page context of the page with this index.
fn page_context(pages: &Pages, index: usize, rtl: bool) -> PageContext {
    pages.0.get(index).cloned().unwrap_or(PageContext { index, name: None, first_in_group: false, rtl })
}

fn content_text(content: &[ContentItem], page: i64, pages: usize) -> String {
    content
        .iter()
        .map(|item| match item {
            ContentItem::String(string) => string.clone(),
            ContentItem::Counter(CounterName::Page, style) => format_counter(page, *style),
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

impl LabelStyle {
    fn counter_style(self) -> crate::style::parse::CounterStyle {
        use crate::style::parse::CounterStyle;
        match self {
            LabelStyle::Decimal => CounterStyle::Decimal,
            LabelStyle::LowerRoman => CounterStyle::LowerRoman,
            LabelStyle::UpperRoman => CounterStyle::UpperRoman,
            LabelStyle::LowerLetters => CounterStyle::LowerAlpha,
            LabelStyle::UpperLetters => CounterStyle::UpperAlpha,
        }
    }
}

/// A range of pages with the same kind of label: from `first_page` (counted from 0) to the page
/// before the start of the next range. The pages of the range are numbered from `start` in the
/// style, after the prefix; without a style the label is the prefix alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLabelRange {
    pub first_page: usize,
    pub style: Option<LabelStyle>,
    pub prefix: String,
    pub start: u32,
}

/// The ranges of the labels of the pages, a resource of the world of the document.
#[derive(Debug, Clone, Default)]
pub struct PageLabels(pub Vec<PageLabelRange>);

/// The margin boxes that can hold the number of the page, in the order in which the first one
/// that does decides the label of a page with `-helium-page-label: auto`: the footer before the
/// header, and the middle before the sides.
const FOLIO_ORDER: [MarginBoxKind; 6] = [
    MarginBoxKind::BottomCenter,
    MarginBoxKind::BottomRight,
    MarginBoxKind::BottomLeft,
    MarginBoxKind::TopCenter,
    MarginBoxKind::TopRight,
    MarginBoxKind::TopLeft,
];

/// The style of the number and the prefix of a label written like `content`: the literal text
/// before `counter(page, <style>)` is the prefix. `None` if there is no `counter(page)` that only
/// strings come before; what comes after the number is not part of the label.
fn label_of(content: &[ContentItem]) -> Option<(LabelStyle, String)> {
    use crate::style::parse::CounterStyle;

    let mut prefix = String::new();
    for item in content {
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
}

/// The label of a page from the number that its margin boxes print, for
/// `-helium-page-label: auto`, so that the page box of a viewer says what the page does (PDF/UA-2
/// asks for this). `None` if no margin box of the page prints the number.
fn folio(boxes: &[ResolvedMarginBox]) -> Option<(LabelStyle, String)> {
    FOLIO_ORDER.iter().find_map(|kind| {
        let margin_box = boxes.iter().find(|margin_box| margin_box.kind == *kind)?;
        label_of(&margin_box.content)
    })
}

/// The ranges of the labels of the pages. The label of a page is decided by its
/// `-helium-page-label` and the value of its `page` counter. A page that has no label from that
/// has its number in decimal. Empty if all labels are the numbers of the pages in decimal, which
/// the PDF needs no `/PageLabels` for.
///
/// Must run after the margin boxes are known, like [`MarginBoxPaint`].
pub fn page_label_ranges(world: &specs::World) -> Vec<PageLabelRange> {
    use specs::{Join, WorldExt};

    let margin_boxes = world.read_resource::<MarginBoxes>();
    if margin_boxes.is_empty() && margin_boxes.counters.is_empty() && margin_boxes.labels.is_empty() {
        return vec![];
    }
    let pages = world.read_resource::<Pages>();
    let page_margins = world.read_resource::<PageMargins>();
    let positions = world.read_storage::<Position>();
    let num_pages = (&positions).join().map(|pos| pos.page + 1).max().unwrap_or(0);
    let numbers = margin_boxes.page_numbers(&pages, num_pages, page_margins.rtl);

    let mut ranges: Vec<PageLabelRange> = Vec::new();
    for (page, &number) in numbers.iter().enumerate() {
        let context = page_context(&pages, page, page_margins.rtl);
        let (style, prefix) = match margin_boxes.label_spec(&context) {
            PageLabelSpec::None => None,
            PageLabelSpec::Content(content) => label_of(content),
            PageLabelSpec::Auto => folio(margin_boxes.for_page(&context)),
        }
        .unwrap_or((LabelStyle::Decimal, String::new()));
        // a label of the PDF has a positive number: the others are written out
        let (style, prefix, start) = if number >= 1 {
            (Some(style), prefix, number.min(i64::from(u32::MAX)) as u32)
        } else {
            (None, prefix + &format_counter(number, style.counter_style()), 1)
        };
        // the labels of the pages go on counting, in the same style, as long as the counter does
        let continues = ranges.last().is_some_and(|last| {
            last.style == style
                && style.is_some()
                && last.prefix == prefix
                && numbers[page - 1] + 1 == number
        });
        if !continues {
            ranges.push(PageLabelRange { first_page: page, style, prefix, start });
        }
    }
    if let [only] = ranges.as_slice() {
        if only.style == Some(LabelStyle::Decimal) && only.prefix.is_empty() && only.start == 1 {
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

        let numbers = margin_boxes.page_numbers(&pages, num_pages, page_margins.rtl);
        for page in 0..num_pages {
            let context = page_context(&pages, page, page_margins.rtl);
            // the margins of the page: the inner and outer margins of a book differ, and the
            // first page of a document or of a page group can have other margins
            let edges = page_margins.edges(&context);
            let (margin_left, margin_right) = (edges.left, edges.right);
            let (page_width, page_height) =
                page_margins.size(&context, (geometry.width, geometry.height));
            let area_left = margin_left;
            let area_width = page_width - margin_left - margin_right;
            for margin_box in margin_boxes.for_page(&context) {
                let mut text = content_text(&margin_box.content, numbers[page], num_pages);
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

    fn list(entries: &[(&str, Option<i32>)]) -> CounterList {
        CounterList(entries.iter().map(|(name, value)| (name.to_string(), *value)).collect())
    }

    #[test]
    fn the_page_counter_follows_the_page_context() {
        let numbers = |ops: &[PageCounterOps]| {
            let mut value = 0;
            ops.iter().map(|ops| { value = ops.next(value); value }).collect::<Vec<_>>()
        };
        let plain = PageCounterOps::default();
        assert_eq!(numbers(&[plain; 4]), [1, 2, 3, 4]);
        // CSS Paged Media 3, 6.1: `counter-increment: page 2` makes all pages even-numbered
        let twice = PageCounterOps::from_lists(None, Some(&list(&[("page", Some(2))])), None);
        assert_eq!(numbers(&[twice; 3]), [2, 4, 6]);
        // the reset comes before the increment (CSS Lists 3): `page 0` makes this page the first
        let restart = PageCounterOps::from_lists(Some(&list(&[("page", Some(0))])), None, None);
        assert_eq!(numbers(&[plain, plain, restart, plain]), [1, 2, 1, 2]);
        // no number: 0
        let restart = PageCounterOps::from_lists(Some(&list(&[("page", None)])), None, None);
        assert_eq!(numbers(&[plain, plain, restart]), [1, 2, 1]);
        // `counter-set` comes after the increment
        let set = PageCounterOps::from_lists(None, None, Some(&list(&[("page", Some(10))])));
        assert_eq!(numbers(&[plain, set, plain]), [1, 10, 11]);
        // other counters are not for us, and an increment without a value is 1
        let other = PageCounterOps::from_lists(
            Some(&list(&[("chapter", Some(5))])),
            Some(&list(&[("chapter", Some(7)), ("page", None), ("page", Some(2))])),
            None,
        );
        assert_eq!(other, PageCounterOps { reset: None, increment: 3, set: None });
    }

    #[test]
    fn page_labels_are_parsed() {
        use crate::style::parse::{CounterStyle, PageLabelSpec, Styles};

        let label = |css: &str| {
            let mut styles = Styles::default();
            styles.parse_css(&format!("@page {{ -helium-page-label: {css} }}"));
            styles.rules_iter().last().unwrap().page_label.clone()
        };
        assert_eq!(label("auto"), Some(PageLabelSpec::Auto));
        assert_eq!(label("none"), Some(PageLabelSpec::None));
        assert_eq!(
            label("\"A-\" counter(page, upper-roman)"),
            Some(PageLabelSpec::Content(vec![
                ContentItem::String("A-".into()),
                ContentItem::Counter(CounterName::Page, CounterStyle::UpperRoman),
            ]))
        );
        // no number, the total of pages, or something else before the number
        for invalid in ["\"x\"", "counter(pages)", "counter(pages) counter(page)", "counter(page) counter(page)", "red"] {
            let valid = label(invalid).is_some();
            assert_eq!(valid, invalid == "counter(page) counter(page)", "{invalid}");
        }
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
