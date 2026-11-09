//! Headers and footers: the margin boxes of `@page` rules.
//!
//! The margin boxes are resolved once ([`MarginBoxes`]) and painted onto every page after
//! pagination ([`MarginBoxPaint`]), when the number of pages is known.
//!
//! Only the six top and bottom margin boxes are supported. Their text is laid out on a single
//! line, aligned with `text-align` inside the content area of the page and centered vertically in
//! the margin.

use super::FontMap;
use super::components::{PageGeometry, Paragraph, Position, TextOperator, ZIndex};
use super::inline::shape_piece;
use crate::style::parse::{ContentItem, CounterName, CounterStyle, MarginBoxKind, PageSelector};
use crate::style::properties::computed::{PixelLength as Px, RGBA, TextAlign};
use specs::{Entities, Read, ReadExpect, System, WriteStorage};

/// A margin box whose style and font are resolved.
#[derive(Debug, Clone)]
pub struct ResolvedMarginBox {
    pub kind: MarginBoxKind,
    pub content: Vec<ContentItem>,
    pub font_name: String,
    pub font_size: Px,
    pub color: RGBA,
    pub fallback: crate::layout::text::fallback::TextFonts,
    pub text_align: TextAlign,
}

/// The margin boxes for the three kinds of pages: the first page, other left pages and other
/// right pages.
#[derive(Debug, Clone, Default)]
pub struct MarginBoxes {
    pub first: Vec<ResolvedMarginBox>,
    pub left: Vec<ResolvedMarginBox>,
    pub right: Vec<ResolvedMarginBox>,
}

impl MarginBoxes {
    /// Whether the document has any headers or footers at all.
    pub fn is_empty(&self) -> bool {
        self.first.is_empty() && self.left.is_empty() && self.right.is_empty()
    }

    /// The margin boxes of the page with the given index. The first page is a right page, so the
    /// pages alternate between right and left (for left-to-right text).
    fn for_page(&self, index: usize) -> &[ResolvedMarginBox] {
        if index == 0 {
            &self.first
        } else if index % 2 == 1 {
            &self.left
        } else {
            &self.right
        }
    }
}

/// Whether an `@page` rule with the selector applies to the first/left/right kind of page.
pub fn selector_matches(selector: PageSelector, first: bool, left: bool) -> bool {
    match selector {
        PageSelector::Any => true,
        PageSelector::First => first,
        PageSelector::Left => left,
        PageSelector::Right => !left,
    }
}

fn roman(mut n: usize) -> String {
    const NUMERALS: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    for &(value, numeral) in &NUMERALS {
        while n >= value {
            result.push_str(numeral);
            n -= value;
        }
    }
    result
}

/// `a`, `b`, … `z`, `aa`, `ab`, … — the `lower-alpha` counter style.
fn alphabetic(mut n: usize) -> String {
    const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
    let mut result = Vec::new();
    while n > 0 {
        n -= 1;
        result.push(LETTERS[n % 26]);
        n /= 26;
    }
    result.reverse();
    String::from_utf8(result).expect("the alphabet is valid UTF-8")
}

fn format_counter(value: usize, style: CounterStyle) -> String {
    match style {
        CounterStyle::Decimal => value.to_string(),
        // `decimal-leading-zero` pads to at least two digits: 01, 02, … 10
        CounterStyle::DecimalLeadingZero => format!("{value:02}"),
        // roman numerals cannot represent 0, and the alphabetic styles start at 1; both fall
        // back to decimal for smaller values
        CounterStyle::LowerRoman if value > 0 => roman(value),
        CounterStyle::UpperRoman if value > 0 => roman(value).to_uppercase(),
        CounterStyle::LowerAlpha if value > 0 => alphabetic(value),
        CounterStyle::UpperAlpha if value > 0 => alphabetic(value).to_uppercase(),
        CounterStyle::LowerRoman
        | CounterStyle::UpperRoman
        | CounterStyle::LowerAlpha
        | CounterStyle::UpperAlpha => value.to_string(),
    }
}

fn content_text(content: &[ContentItem], page: usize, pages: usize) -> String {
    content
        .iter()
        .map(|item| match item {
            ContentItem::String(string) => string.clone(),
            ContentItem::Counter(CounterName::Page, style) => format_counter(page, *style),
            ContentItem::Counter(CounterName::Pages, style) => format_counter(pages, *style),
        })
        .collect()
}

/// Creates the paragraphs of the margin boxes of every page.
pub struct MarginBoxPaint;

impl<'a> System<'a> for MarginBoxPaint {
    type SystemData = (
        Entities<'a>,
        Read<'a, MarginBoxes>,
        Read<'a, FontMap>,
        ReadExpect<'a, PageGeometry>,
        WriteStorage<'a, Position>,
        WriteStorage<'a, Paragraph>,
    );

    fn run(
        &mut self,
        (entities, margin_boxes, font_map, geometry, mut positions, mut paragraphs): Self::SystemData,
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

        let area_left = geometry.margin.left;
        let area_width = geometry.width - geometry.margin.left - geometry.margin.right;
        for page in 0..num_pages {
            for margin_box in margin_boxes.for_page(page) {
                let text = content_text(&margin_box.content, page + 1, num_pages);
                if text.is_empty() {
                    continue;
                }
                let font = &font_map[&margin_box.font_name];
                let mut ops = vec![
                    TextOperator::SetFont(margin_box.font_name.clone(), margin_box.font_size),
                    TextOperator::SetColor(margin_box.color),
                ];
                let width =
                    shape_piece(
                    &text,
                    font,
                    &margin_box.fallback,
                    None,
                    margin_box.font_size,
                    Px(0.0),
                    &[],
                    false,
                    &mut ops,
                );

                let x = match margin_box.text_align.resolve(false) {
                    TextAlign::Center => area_left + (area_width - width) / 2.0,
                    TextAlign::Right => area_left + area_width - width,
                    TextAlign::Left | TextAlign::Justify | TextAlign::Start | TextAlign::End => {
                        area_left
                    }
                };

                let ascent = font.scale_to_px(font.ascent(), margin_box.font_size);
                let descent = font.scale_to_px(font.descent(), margin_box.font_size).abs();
                let height = ascent + descent;
                let (margin_top, margin_height) = match margin_box.kind {
                    MarginBoxKind::TopLeft | MarginBoxKind::TopCenter | MarginBoxKind::TopRight => {
                        (Px(0.0), geometry.margin.top)
                    }
                    MarginBoxKind::BottomLeft
                    | MarginBoxKind::BottomCenter
                    | MarginBoxKind::BottomRight => (
                        geometry.height - geometry.margin.bottom,
                        geometry.margin.bottom,
                    ),
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
                            z: ZIndex::default(),
                            page,
                        },
                        &mut positions,
                    )
                    .with(
                        Paragraph {
                            start_pos: (x, y + ascent),
                            ops,
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
    use crate::style::parse::Styles;

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
    }

    #[test]
    fn unsupported_counter_styles_fall_back_to_decimal() {
        let mut styles = Styles::default();
        styles.parse_css(
            "@page { @bottom-center { content: counter(page, lower-greek) \" / \" counter(pages) } }",
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
        assert_eq!(rule.page_selector(), Some(PageSelector::First));
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
