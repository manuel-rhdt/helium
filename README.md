# Helium

Helium turns an HTML document with CSS (and MathML) into a paginated, tagged PDF. It is a print
renderer: it lays out pages, not a scrollable screen, and it runs no scripts.

```
helium [options] input.html output.pdf
```

| Option | Effect |
| --- | --- |
| `--no-compression` | write uncompressed content streams (for inspecting the PDF) |
| `--crop` | size the page to its content instead of A4 |
| `--no-tags` | write an untagged PDF |
| `--no-system-fonts` | use only the built-in fonts, so that the output does not depend on the machine |
| `--dump-layout <file>` | write the computed layout as JSON (`-` for stdout) |
| `--check-layout` | evaluate the `data-expected-*` attributes of the document against the layout |

This list describes what the code supports. Anything not listed is not supported; the open
issues track the known gaps. Unsupported declarations are logged (`RUST_LOG=warn`) and ignored.

## HTML

- **Parsing:** HTML5 (`html5ever`), `lang` and `dir`, `<title>`.
- **Default styles** for the usual elements: sectioning and headings, `p`, `div`, `pre`,
  `blockquote`, `hr`, `ul`/`ol`/`li`/`dl`, `table` with `caption`/`thead`/`tbody`/`tfoot`/`tr`/`td`/`th`/`col`/`colgroup`,
  `a`, inline phrasing elements (`em`, `strong`, `code`, `sub`, `sup`, `q`, `mark`, ...), `figure`,
  `details`, and form controls (`input` shows its `value`; controls are static).
- **Images:** `<img>` with JPEG and PNG files, from a path inside the directory of the document
  or a base64 `data:` URI. The format is recognized by the contents of the file. The pixels
  are not decoded: JPEG files (gray, RGB, CMYK, baseline and progressive) and PNG files (gray,
  RGB and palette, 1 to 16 bits, with a transparent color or a single transparent palette entry)
  are embedded as they are, so large images cost no time. PNG files with an alpha channel or
  with interlacing, and partially transparent palettes, are not supported yet and leave an empty
  box. Sizes follow the `width`/`height` attributes and properties and the proportions of the
  image.
- **Attributes that change the output:** `href` (external links and `#id` links inside the
  document), `style`, `colspan`, `rowspan`, `<ol start>`, `<li value>`, `<col>` widths.
- **Stylesheets:** `<style>` and `<link rel="stylesheet">` with a path relative to the document.

## CSS

- **Box model:** `width`, `height`, `min-`/`max-` width and height, `box-sizing`, `margin`,
  `padding`, `border` (width, color and `solid` style, per side), `border-radius`, `top`/`right`/`bottom`/`left`.
- **Layout:** `display` (`block`, `inline`, `inline-block`, `flex`, `grid`, `table` and the table
  parts, `list-item`, `none`), `position: absolute`, block, inline, flexbox
  (`flex-*`, `gap`, `justify-*`, `align-*`) and grid (`grid-template-*`, `grid-auto-*`,
  `grid-column`, `grid-row`) layout, tables (`border-collapse`, `border-spacing`).
- **Text:** `font` and its longhands (`font-family`, `font-style`, numeric and keyword
  `font-weight`, `font-size`, `line-height`), `color`, `text-align` (including `justify`),
  `text-indent`, `text-decoration`, `text-transform`, `letter-spacing`, `white-space`, `hyphens`,
  `direction`, `unicode-bidi`, `list-style-type`.
- **Backgrounds:** `background` and `background-color`, `-image` (linear, radial and conic
  gradients, also repeating), `-size`, `-position`, `-repeat`, `-blend-mode`.
- **Colors:** named, hex, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`,
  `color()`. The alpha channel is parsed but not painted.
- **Units:** `px`, `pt`, `pc`, `in`, `cm`, `mm`, `em`, `%`.
- **Cascade:** selectors (`selectors` crate), `inherit`, `initial`, `unset`.
- **Pages (`@page`):** `size`, `margin`, the `:first`, `:left` and `:right` selectors, named
  pages (`page` property), the six margin boxes (`@top-left` ... `@bottom-right`) with `content`
  from strings and `counter(page)` / `counter(pages)`, in the styles `decimal`,
  `decimal-leading-zero`, `lower-` and `upper-roman`, `lower-` and `upper-alpha`/`latin`,
  `arabic-indic` and `persian`.
- **Fragmentation:** `break-before`/`-after`/`-inside`, `page-break-*`, `orphans`, `widows`;
  tables and `pre` blocks split across pages.

## MathML

Presentation MathML inside `<math>`, laid out with the font's OpenType `MATH` table (the crate
[`formel`](https://crates.io/crates/formel)): `math`, `mrow`, `mfrac`, `msqrt`, `mroot`, `msub`,
`msup`, `msubsup`, `munder`, `mover`, `munderover`, `mmultiscripts`, `mi`, `mn`, `mo`, `mtext`,
`ms`, `mspace`, `mpadded`, `menclose`, `mphantom`, `mstyle`, `merror`, `semantics`, `mtable`,
`mtr`, `mtd`. Inline and block (`display="block"`) formulas, and the `math-style`, `math-depth`
and `math-shift` properties. The text of the formula is selectable and the formula is tagged as
a `Formula` with alternative text from `alttext` or a TeX annotation.

## Text and fonts

- **Shaping and scripts:** HarfBuzz shaping, the Unicode bidirectional algorithm (right-to-left
  scripts), script itemization, kerning and ligatures, per-character font fallback.
- **Line breaking:** Unicode line breaking, hyphenation with built-in dictionaries (`hyphens:
  auto` and soft hyphens), CJK line breaking, justification.
- **Font formats:** TrueType and OpenType (`.ttf`, `.otf`, `.ttc`, with TrueType or CFF
  outlines). Fonts are subset into the PDF. `@font-face` is not supported: fonts come from the
  system (fontconfig on Linux, and the font directories) or from the built-in set: TeX Gyre
  Heros, Termes and Cursor (the sans, serif and monospace defaults), DejaVu Sans (fallback), STIX
  Two Math and Latin Modern Math. The generic families `serif`, `sans-serif`, `monospace` and
  `system-ui` are mapped to them.

## PDF output

PDF 1.7; Flate-compressed streams; tagged PDF with a structure tree derived from the HTML
(headings, paragraphs, lists, tables, links, figures, formulas) and the PDF/UA identifier;
document title and language; XMP metadata; link annotations; `ToUnicode` maps and
`ActualText`, so that text can be extracted and copied; embedded font subsets; clipped and
rounded boxes; axial, radial and mesh shadings for gradients; blended backgrounds.

## Not supported

Other image formats (GIF, WebP, ...), PNG with alpha, vector images (`<svg>`), `url()` backgrounds, `@font-face`, `@media`,
`@supports`, `@import`, `!important`, `var()`, `calc()`, `overflow`, `float`, `position:
relative`/`fixed`/`sticky`, `z-index`, `transform`, `opacity` and color alpha, `box-shadow`,
`outline`, border styles other than `solid`, `vertical-align`, `word-break`/`overflow-wrap`,
`counter-*` and `::before`/`::after`, `rem`/`vw`/`vh` and other units, scripting, and form
interaction.
