# Gallery

Documents written in plain HTML and CSS, rendered to PDF by Umbruch. The `Gallery` workflow
renders them on every push and, on `master`, publishes them with thumbnails to GitHub Pages.

| Directory | Shows |
| --- | --- |
| `invoice/` | gradient header card with an SVG logo, flexbox and grid columns, a line-item table, badges |
| `letter/` | letterhead, a different first page, running headers, hyphenated justified text |
| `report/` | a cover card, linked contents, a JPEG figure, data table, a flexbox bar chart, code |
| `math-notes/` | MathML: integrals, sums, matrices, theorem boxes, numbered equations, an SVG plot |
| `book/` | a whole novel as a trade paperback in three volumes: all 61 chapters of Pride and Prejudice (372 pages), facing pages and running heads, a tagged table of contents with dot leaders (`leader()`) and page numbers that `target-counter()` looks up, page groups (`:nth(1 of volume)`, `:nth(1 of chapter)`) for the volume title pages and the chapter openings, roman folios for the front matter and a new count for the body, a list of scenes with chapter numbers in roman numerals, an outline of volumes and chapters, hyphenation limits as TeX has them for English (`hyphenate-limit-chars`) and no hyphen at the end of a right-hand page (`hyphenate-limit-last: spread`), hanging punctuation (`hanging-punctuation`: quotes, full stops, commas and hyphens stand in the margin), small capitals and old-style folios |
| `poetry/` | verse on small pages |
| `scripts/` | one sentence in 30 languages and more than 20 scripts |
| `figures/` | a one-page figure sheet: inline SVG with the CSS of the page, MathML labels and equation cards inside of the figures (`foreignObject`), an icon sprite, patterns, markers, gradients |
| `poster/` | a two-page A2 print of 123 writing systems, each in a real text; flexbox masonry, vertical `writing-mode` for Han, Japanese and Mongolian (needs extra fonts, which CI downloads) |
| `emoji/` | colour emoji painted as vectors from the `COLR` table of the built-in Noto Color Emoji: ZWJ sequences, skin tones, flags and keycaps, text and emoji presentation, any size, tables, lists, page margins |
| `typography/` | a specimen of OpenType features in EB Garamond, set in two columns that flow (`column-count`) and chosen with `font-feature-settings` and the `font-variant-*` properties: ligatures, real small capitals, old-style, lining and tabular figures, fractions, superiors, historical forms, case-sensitive forms, stylistic sets (needs EB Garamond, which CI downloads) |
| `design/` | gradients, blend modes, tiled patterns, rounded boxes, SVG images (clip path, mask, pattern, text), a dashboard |
| `article/` | a two-column physics letter in the layout of a journal such as Physical Review Letters: a full-width title block and abstract, then 10 pt text in two columns of 24 em that run over the pages, run-in headings, numbered MathML equations, SVG figures with real data from `generate.py` (an exact first-passage calculation and a Langevin simulation), tables with rules, numbered references, running heads on facing pages |
| `slides/` | a ten-slide conference talk on 16:9 pages: a full-bleed photo title, big-number cards, a split slide with a picture, a code slide with highlighted source, a gradient quote, a chart drawn in SVG, a before-and-after comparison, a picture mosaic, a roadmap and a closing slide; the pictures come from `generate.py` (numpy and Pillow) |
| `cv/` | a one-page CV: gradient sidebar, an illustrated portrait and icons in inline SVG, skill rings from conic gradients, bars, a timeline built from borders and generated content, stat and project cards, tags, a pull quote |

## Build locally

```
cargo build --release
python3 gallery/build.py            # writes _site/
python3 gallery/build.py letter     # only some documents
```

`build.py` needs `pdftoppm` and `pdfinfo` (poppler-utils) for the thumbnails and page counts.

## Add a document

Create `gallery/<name>/index.html` (with its images next to it) and add the name to
`order.txt`. The page takes its card from the document:

- `<title>`, `<meta name="description">` and `<meta name="keywords">` (comma-separated tags);
- `<meta name="gallery-thumbnail" content="3">` to use another page than the first;
- `<meta name="gallery-fonts" content="system">` to render with the fonts installed on the
  machine. By default only the fonts built into Umbruch are used (they include Noto fonts for
  most scripts), so the output is the same everywhere.

References in the document (stylesheets, images) are resolved relative to its directory, and
only files inside that directory can be read.
