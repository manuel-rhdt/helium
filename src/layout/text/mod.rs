pub mod builtin;
pub mod font;
pub mod whitespace;
mod to_pdf_font;

pub use builtin::{
    InlineFragment, InlineFragmentIterator, InlineFragmentIteratorState, LineIsFullError,
};
pub use font::Font;

pub trait FontLike {}