//! Umbruch as a library: the layout engine and PDF writer behind one entry type, [`Engine`].
//!
//! The `umbruch` binary is a thin consumer of this crate (argument parsing and file I/O); the
//! `--serve` mode of the binary speaks the protocol of [`server`] over stdio.
#![allow(clippy::needless_update)]

#[macro_use]
extern crate cssparser;

#[macro_use]
extern crate html5ever;

#[cfg(test)]
extern crate quickcheck;
#[cfg(test)]
#[macro_use(quickcheck)]
extern crate quickcheck_macros;

pub mod default_cell_ext;
pub mod display_list;
pub mod dot;
pub mod engine;
pub mod font_usage;
pub mod image;
pub mod layout;
pub mod measure;
pub mod metadata;
pub mod node_data_ref;
mod parser;
pub mod pdf;
pub mod resource;
pub mod server;
pub mod simple_dom;
pub mod style;
mod svg;
pub mod svg_dom;
mod svg_input;
mod svg_text;
pub mod tagging;
mod traverse;
mod tree;

pub use engine::{
    Document, DocumentKind, Engine, EngineConfig, Error, LayoutOptions, LayoutResult, RenderOptions, Rendered,
};
pub use style::properties::computed::PixelLength;
