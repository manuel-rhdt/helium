use crate::style::properties::computed::PixelLength;

#[derive(Debug, Copy, Clone)]
pub struct BuiltInFontMetrics {
    pub font_name: &'static str,
    pub ascender: f64,
    pub descender: f64,
    pub char_metrics: &'static [CharMetric],
}

#[derive(Debug, Copy, Clone)]
pub struct CharMetric {
    pub name: &'static str,
    pub bbox: [f64; 4],
    pub character_code: i32,
    pub advance_width: f64,
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
pub fn layout_text(text: &str, container_width: PixelLength) -> Vec<usize> {
    let font = BuiltInFonts::Helvetica.metrics();
    let mut cursor = PixelLength(0.0);
    let mut linebreaks = vec![];
    for (index, character) in text.chars().enumerate() {
        let char_metric = font
            .char_metrics
            .iter()
            .find(|metric| metric.character_code == character as i32);

        if let Some(char_metric) = char_metric {
            cursor += PixelLength(char_metric.advance_width as _);
            if cursor > container_width {
                linebreaks.push(index);
                cursor = PixelLength(0.0);
            }
        }
    }

    linebreaks
}

pub fn line_height() -> PixelLength {
    let metrics = BuiltInFonts::Helvetica.metrics();
    PixelLength((metrics.ascender + metrics.descender) as _)
}
