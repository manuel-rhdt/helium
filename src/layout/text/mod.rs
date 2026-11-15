mod embedded;
pub mod fallback;
mod fallback_data;
pub mod font;
pub mod segment;
mod shape;
mod to_pdf_font;
pub mod transform;
pub mod whitespace;

pub use font::Font;
pub use shape::{
    GlyphInfo, GlyphPosition, HarfbuzzFont, is_invisible_formatting, shape, shape_directed, shape_directed_into,
    shape_with_levels,
};
