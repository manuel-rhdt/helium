# Gallery

Documents written in plain HTML and CSS, rendered to PDF by Helium. The `Gallery` workflow
renders them on every push and, on `master`, publishes them with thumbnails to GitHub Pages.

| Directory | Shows |
| --- | --- |
| `invoice/` | gradient header card with an SVG logo, flexbox and grid columns, a line-item table, badges |
| `letter/` | letterhead, a different first page, running headers, hyphenated justified text |
| `report/` | a cover card, linked contents, a JPEG figure, data table, a flexbox bar chart, code |
| `math-notes/` | MathML: integrals, sums, matrices, theorem boxes, numbered equations, an SVG plot |
| `book/` | a trade paperback: facing pages, running heads, chapter openings |
| `poetry/` | verse on small pages |
| `scripts/` | one sentence in 30 languages and more than 20 scripts |
| `figures/` | a one-page figure sheet: inline SVG with the CSS of the page, MathML labels and equation cards inside of the figures (`foreignObject`), an icon sprite, patterns, markers, gradients |
| `poster/` | a two-page A2 print of 123 writing systems, each in a real text; flexbox masonry, vertical `writing-mode` for Han, Japanese and Mongolian (needs extra fonts, which CI downloads) |
| `emoji/` | colour emoji painted as vectors from the `COLR` table of the built-in Noto Color Emoji: ZWJ sequences, skin tones, flags and keycaps, text and emoji presentation, any size, tables, lists, page margins |
| `design/` | gradients, blend modes, tiled patterns, rounded boxes, SVG images (clip path, mask, pattern, text), a dashboard |

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
  machine. By default only the fonts built into Helium are used (they include Noto fonts for
  most scripts), so the output is the same everywhere.

References in the document (stylesheets, images) are resolved relative to its directory, and
only files inside that directory can be read.
