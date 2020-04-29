pub mod builtin;
pub mod font;
pub mod whitespace;

pub use builtin::{
    InlineFragment, InlineFragmentIterator, InlineFragmentIteratorState, LineIsFullError,
};
pub use font::Font;

pub trait FontLike {}