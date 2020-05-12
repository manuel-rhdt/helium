pub mod builtin;
pub mod font;
pub mod whitespace;
mod to_pdf_font;
mod shape;

pub use builtin::{
    InlineFragment, InlineFragmentIterator, InlineFragmentIteratorState, LineIsFullError,
};
pub use font::Font;
pub use shape::shape;

pub trait FontLike {}