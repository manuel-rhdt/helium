pub mod builtin;
pub mod font;
mod whitespace;

pub use font::Font;
pub use builtin::{InlineFragment, InlineFragmentIteratorState, InlineFragmentIterator};
