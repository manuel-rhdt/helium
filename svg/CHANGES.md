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
- `foreignObject` is an element (`EId::ForeignObject`). The tree has a node for it
  (`Node::Foreign(ForeignObject)`) with its rectangle and the handle that the builder of the XML
  document gave to the element; the content is not read, the caller draws it.
- Text is not shaped by `harfrust` and read with `skrifa`: the caller gives `Options` a
  `TextShaper` (`text_shaper`), and the outlines and metrics of fonts are read with `ttf-parser`.
  Without a shaper `text` elements are not drawn. Colour glyphs (COLR, SVG and bitmap glyphs),
  variable fonts and `writer` (not a default feature any more) are gone with their code
  (`text/colr.rs`, most of `text/cache.rs`).
- Font fallback replaces the clusters that have a missing glyph (by their bytes in the text)
  instead of assuming that every font makes the same number of glyphs, which dropped the fallback
  of the whole text when a font made a single glyph of a cluster (an emoji with U+FE0F).
