# Gallery

Documents written in plain HTML and CSS, rendered to PDF by Helium. The `Gallery` workflow
renders them on every push and, on `master`, publishes them with thumbnails to GitHub Pages.

| Directory | Shows |
| --- | --- |
| `invoice/` | gradient header card, flexbox and grid columns, a line-item table, badges |
| `letter/` | letterhead, a different first page, running headers, hyphenated justified text |
| `report/` | a cover card, linked contents, a JPEG figure, data table, a flexbox bar chart, code |
| `math-notes/` | MathML: integrals, sums, matrices, theorem boxes, numbered equations |
| `book/` | a trade paperback: facing pages, running heads, chapter openings |
| `poetry/` | verse on small pages |
| `scripts/` | one sentence in 30 languages and more than 20 scripts |
| `design/` | gradients, blend modes, tiled patterns, rounded boxes, a dashboard |

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
