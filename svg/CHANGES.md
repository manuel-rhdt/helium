# Changes against usvg 0.50.0

- Copied without `main.rs`, the tests and the docs; `pico-args`, `memmap-fonts` and
  `system-fonts` are gone from `Cargo.toml`.
- The parser reads `usvg::xml::Document` (`src/xml.rs`), a tree that owns its strings, instead of
  `roxmltree::Document`. A `DocumentBuilder` makes one from any tree; `Document::from_roxmltree`
  makes one from SVG text (and resolves the CSS with `simplecss`, as before).
  `Tree::from_xml` reads it.
- The CSS declarations that apply to an element are stored with the element in the document, so
  the parser has no CSS engine of its own to run on the tree. The code that builds the document
  decides what matches.
