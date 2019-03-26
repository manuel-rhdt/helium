use crate::style::properties::computed::PixelLength;

mod whitespace;

/// A postscript font unit (1/1000 of an EM)
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct FontUnit(pub f64);

impl std::ops::Add for FontUnit {
    type Output = FontUnit;

    fn add(self, other: FontUnit) -> FontUnit {
        FontUnit(self.0 + other.0)
    }
}

impl std::ops::Sub for FontUnit {
    type Output = FontUnit;

    fn sub(self, other: FontUnit) -> FontUnit {
        FontUnit(self.0 - other.0)
    }
}

impl FontUnit {
    pub fn to_px(self, font_size: PixelLength) -> PixelLength {
        font_size * PixelLength((self.0 / 1000.0) as _)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct BuiltInFontMetrics {
    pub font_name: &'static str,
    pub ascender: FontUnit,
    pub descender: FontUnit,
    pub char_metrics: &'static [CharMetric],
}

#[derive(Debug, Copy, Clone)]
pub struct CharMetric {
    pub name: &'static str,
    pub bbox: [FontUnit; 4],
    pub character_code: i32,
    pub advance_width: FontUnit,
}

// The following `include!` expands to
//
// #[derive(Debug, Copy, Clone, PartialEq, Eq)]
// pub enum BuiltInFonts {
//     CourierBold,
//     CourierBoldOblique,
//     CourierOblique,
//     Courier,
//     HelveticaBold,
//     HelveticaBoldOblique,
//     HelveticaOblique,
//     Helvetica,
//     Symbol,
//     TimesBold,
//     TimesBoldItalic,
//     TimesItalic,
//     TimesRoman,
//     ZapfDingbats,
// }
//
// impl BuiltInFonts {
//     pub fn metrics(self) -> BuiltInFontMetrics { ... }
// }
include!(concat!(env!("OUT_DIR"), "/base_14_fonts.rs"));

/// Returns lines
pub fn layout_text<'a>(
    text: &'a mut String,
    font_size: PixelLength,
    container_width: PixelLength,
) -> impl 'a + Iterator<Item = &'a str> {
    let font = BuiltInFonts::Helvetica.metrics();
    let mut cursor = PixelLength(0.0);

    whitespace::transform_text(text);

    // tracks the string index of the last space and the cursor position for the text after the last space
    let mut last_space = (0, PixelLength(0.0));
    let mut line_start = 0;

    // make an immutable borrow of text.
    let text: &'a str = text;
    text.char_indices().filter_map(move |(index, character)| {
        let char_metric = font
            .char_metrics
            .iter()
            .find(|metric| metric.character_code == character as i32);

        if let Some(char_metric) = char_metric {
            cursor += char_metric.advance_width.to_px(font_size);
            if character == ' ' {
                last_space = (index, cursor);
            }
            if cursor > container_width {
                let result = Some(&text[line_start..last_space.0]);
                line_start = last_space.0 + 1;
                cursor = cursor - last_space.1;
                return result;
            }
        }
        if index + 1 == text.len() {
            return Some(&text[line_start..]);
        }
        return None;
    })
}

pub fn font_height(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFonts::Helvetica.metrics();
    (metrics.ascender - metrics.descender).to_px(font_size)
}

pub fn font_ascender(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFonts::Helvetica.metrics();
    metrics.ascender.to_px(font_size)
}
