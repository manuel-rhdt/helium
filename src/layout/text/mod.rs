pub mod builtin;
pub mod fallback;
pub mod font;
mod shape;
mod to_pdf_font;
pub mod whitespace;

pub use builtin::{
    InlineFragment, InlineFragmentIterator, InlineFragmentIteratorState, LineIsFullError,
};
pub use font::Font;
pub use shape::{HarfbuzzFont, shape};

pub trait FontLike {}
