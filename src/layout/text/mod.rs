use crate::style::properties::computed::PixelLength;

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

/// Returns linebreak locations
pub fn layout_text(text: &str, font_size: PixelLength, container_width: PixelLength) -> Vec<usize> {
    let font = BuiltInFonts::Helvetica.metrics();
    let mut cursor = PixelLength(0.0);
    let mut linebreaks = vec![];

    let mut last_space = (0, PixelLength(0.0));
    for (index, character) in text.chars().enumerate() {
        let char_metric = font
            .char_metrics
            .iter()
            .find(|metric| metric.character_code == character as i32);

        if character == ' ' {
            last_space = (index, cursor);
        }

        if let Some(char_metric) = char_metric {
            cursor += char_metric.advance_width.to_px(font_size);
            if cursor > container_width {
                linebreaks.push(last_space.0);
                cursor = cursor - last_space.1;
            }
        }
    }

    linebreaks
}

pub fn font_height(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFonts::Helvetica.metrics();
    (metrics.ascender - metrics.descender).to_px(font_size)
}

pub fn font_ascender(font_size: PixelLength) -> PixelLength {
    let metrics = BuiltInFonts::Helvetica.metrics();
    metrics.ascender.to_px(font_size)
}
