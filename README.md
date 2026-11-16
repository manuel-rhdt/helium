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
- **Presentational attributes:** `width` and `height` of `img` and `iframe` (pixels and percentages), `cellspacing` and `cellpadding` of tables.
- **Default styles** for the usual elements: sectioning and headings, `p`, `div`, `pre`,
  `blockquote`, `hr`, `ul`/`ol`/`li`/`dl`, `table` with `caption`/`thead`/`tbody`/`tfoot`/`tr`/`td`/`th`/`col`/`colgroup`,
  `a`, inline phrasing elements (`em`, `strong`, `code`, `sub`, `sup`, `q`, `mark`, ...), `figure`,
  `details`, and form controls (`input` shows its `value`; controls are static).
- **Images:** `<img>` with JPEG, PNG and SVG files, from a path inside the directory of the
  document or a `data:` URI. The format is recognized by the contents of the file. JPEG files
  (gray, RGB, CMYK, baseline and progressive) and PNG files without alpha channel and
  interlacing (gray, RGB and palette, 1 to 16 bits, also with a transparent color or a single
  transparent palette entry) are embedded without being decoded. All other PNG files (alpha
  channel, interlaced, partially transparent palettes) are decoded, with 16 bit samples reduced
  to 8 bits and the alpha channel written as a soft mask. Gamma and color profiles of PNG files
  are ignored. SVG files are drawn as vector graphics (see below). Sizes follow the
  `width`/`height` attributes and properties and the proportions of the image; an image in a box
  of another shape is scaled to fit and centered.
- **SVG** (`usvg` parses it): paths and basic shapes, fill and stroke (caps, joins, dashes), solid
  colors, linear and radial gradients including `spreadMethod`, patterns, clip paths, masks, group
  opacity, `mix-blend-mode`, nested transforms, `use` and `symbol`, embedded JPEG and PNG
  images, text (shaped by HarfBuzz with the fonts of the document, drawn as outlines; no colour glyphs). The same code draws the
  `<svg>` elements of an HTML document: replaced boxes (sized by `width`/`height`, the
  `viewBox` or CSS) that read the DOM directly, so the style sheets of the document apply to
  their elements (all selectors, the cascade, the `style` attribute), `currentColor` and the font
  are the ones of the page, and elements can refer to the ones of another `<svg>` of the page
  (`<use href="#icon">` of a hidden sprite, a shared gradient). `<foreignObject>` holds HTML and
  MathML: its content is laid out like a small page of the size of the element, with the style
  sheets and the fonts of the page (selectors can reach from the page into it), and drawn with
  the transformations, clip paths and masks of the SVG around it, so diagram labels can be
  formulas; it can contain `<svg>` again. Not supported: filters (ignored), the opacity of
  gradient stops, GIF and WebP inside of SVG, `!important` in the `style` attribute of SVG
  elements (it does not win over one in a style sheet), a
  `foreignObject` in an SVG file (`<img src="x.svg">`: the content needs the document),
  hyperlinks and tags (accessibility structure) inside of a `foreignObject`.
- **Attributes that change the output:** `href` (external links and `#id` links inside the
  document), `style`, `colspan`, `rowspan`, `<ol start>`, `<li value>`, `<col>` widths.
- **Stylesheets:** `<style>` and `<link rel="stylesheet">` with a path relative to the document.

## CSS

- **Box model:** `width`, `height`, `min-`/`max-` width and height, `box-sizing`, `margin`,
  `padding`, `border` (width, color and style, per side), `border-radius`, `top`/`right`/`bottom`/`left`.
  All border styles are painted (`dotted`, `dashed`, `double`, `groove`, `ridge`, `inset`,
  `outset`, `hidden`), except that a border with rounded corners draws `dotted`, `dashed`,
  `double`, `groove` and `ridge` as one solid line (`inset` and `outset` keep their two tones);
  in tables with collapsing borders they follow the conflict resolution of CSS 2.2.
- **Cascade:** specificity, `!important`, the origins (user agent, author, `style` attribute),
  and `@media` rules and the `media` attribute of `<style>` and `<link>` for the medium
  `print`. The size features (`width`, `height`, `orientation`, ...) are evaluated against the
  size of the page (the `@page` size, which may itself be set in a `@media` block; the style
  sheets are read again when the size they set differs from the one assumed); `(hover)`, `(pointer)` and
  similar features say that there is no pointer. `rem` is relative to the root element (in
  `@page` and its margin boxes, to the initial font size).
- **Layout:** `display` (`block`, `inline`, `inline-block`, `flex`, `inline-flex`, `grid`, `inline-grid`,
  `table` and the table
  parts, `list-item`, `none`), `order` of flex and grid items, `inset` and the logical properties (`margin-inline-start`,
  `padding-block`, `inset-inline`, `inline-size`, ... without the logical `border-*`; they follow the
  `writing-mode` and `direction` of the box itself), `aspect-ratio` (also for the images and `<canvas>` boxes, which are blank and 300 by 150 pixels
  unless their attributes say otherwise), `vertical-align` (on inline boxes, and `top`, `middle` and `bottom` in table cells; the baselines of
  the cells in a row are not aligned), `position: absolute` and `position: relative` (`left`, `right`,
  `top` and `bottom` move the box after layout, also inline boxes, `inline-block` boxes and formulas, in
  vertical text too; percentages of `left`/`right` in vertical lines count as `auto`), block, inline, flexbox
  (`flex-*` with `flex-flow`, `gap` with one or two values, `justify-*`, `align-*`, `place-*`) and grid (`grid-template-*`, `grid-auto-*`,
  `grid-column`, `grid-row`, also for absolutely positioned children, which are placed in their grid area) layout, tables (`border-collapse`, `border-spacing`).
- **Text:** `font` and its longhands (`font-family`, `font-style`, numeric and keyword
  `font-weight`, `font-size`, `line-height`), `color`, `text-align` (including `justify`),
  `text-indent`, `text-decoration`, `text-transform`, `letter-spacing`, `white-space` (`normal`, `nowrap`, `pre`, `pre-wrap`, `pre-line`), `hyphens`,
  `direction`, `unicode-bidi`, `list-style-type`.
- **OpenType features:** `font-feature-settings`, `font-kerning`, and `font-variant` with
  `font-variant-ligatures`, `-caps`, `-numeric`, `-east-asian` and `-position`; the `font`
  shorthand resets them and takes `small-caps`. They are handed to the shaper in the order of the
  CSS Fonts 4 precedence (the `font-feature-settings` last). Not supported:
  `font-variant-alternates` and `-emoji`, `font-variation-settings`, and the synthesis of small
  caps, subscripts and superscripts for fonts that lack the features (the text stays as it is).
- **Vertical text:** `writing-mode` (`vertical-rl`, `vertical-lr`, and `sideways-rl` and
  `sideways-lr` as the vertical modes) and `text-orientation` (`mixed`, `upright`, `sideways`).
  A box with another writing mode than its parent is an orthogonal flow, which is laid out on its
  own in logical axes (inline and block) and then placed: its physical sizes, margins, padding
  and borders turn with the axes, its lines run down and stack right to left (`vertical-rl`) or
  left to right (`vertical-lr`), upright characters (UAX #50: CJK, Hangul, ...) use the vertical
  glyphs of the font and the others are turned a quarter. An orthogonal flow is not split
  between pages and needs a `height` (the length of its lines) or a containing block with one.
  Not supported yet: vertical text for the whole document or for the content that is split over
  pages, `direction: rtl` and `text-align: left`/`right` in vertical lines, absolutely
  positioned boxes and tables in a vertical flow, `text-combine-upright`, `text-orientation`
  per character class beyond UAX #50, and `sideways-lr` lines that run upwards.
- **Transforms:** `transform` (2D: `translate`, `scale`, `rotate`, `skew` and their axis forms,
  `matrix`; applies to the box and everything inside of it) and `transform-origin`. A transform
  does not change the layout, does not apply to non-replaced inline boxes, and a box that is
  split over pages is transformed on its first page only.
- **Backgrounds:** `background` and `background-color`, `-image` (linear, radial and conic
  gradients, also repeating, and `url()` images: PNG, JPEG and SVG files and `data:` URIs), `-size`
  (including `cover` and `contain`), `-position`, `-repeat`, `-blend-mode`, `-clip` (`border-box`,
  `padding-box` and `content-box`; boxes with rounded corners are always clipped to the border
  box). A relative URL in a style sheet is relative to the style sheet, and the file has to be
  inside of the directory of the document (or `--resource-root`) like the images of `<img>`. In a
  background with a blend mode, the images are painted over the blended layers, without being
  blended. Text in an SVG image that is a background is set in the system and built-in fonts, not
  in the `@font-face` fonts of the document.
- **Colors:** named, hex, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`,
  `color()`. The alpha channel is parsed but not painted.
- **Units:** `px`, `pt`, `pc`, `in`, `cm`, `mm`, `em`, `rem`, `ex`, `ch`, `vw`, `vh`, `vi`, `vb`, `vmin`, `vmax` (relative to the page area), `%`.
- **Cascade:** selectors (`selectors` crate, with `:is()` and `:where()`), `inherit`, `initial`,
  `unset`.
- **Nesting:** style rules and `@media` blocks inside style rules, with `&` or with relative
  selectors (`.a { .b { } > p { } &:hover { } }`). A declaration that follows a nested rule is
  cascaded after it. Not in the `style` attribute.
- **Custom properties:** `--name: value` (inherited, case sensitive, with `initial`, `inherit` and
  `unset`) and `var(--name, fallback)` in the value of any supported property, also in shorthands
  and in other custom properties. Cycles make the properties invalid, and so does a `var()` with
  neither value nor fallback; the property that uses it is then `unset`. Custom properties are
  not available in `@page` rules and their margin boxes, nor in the style of inline SVG.
- **Pages (`@page`):** `size`, `margin`, the `:first`, `:left` and `:right` selectors, named
  pages (`page` property), the six margin boxes (`@top-left` ... `@bottom-right`) with `content`
  from strings and `counter(page)` / `counter(pages)`, in the styles `decimal`,
  `decimal-leading-zero`, `lower-` and `upper-roman`, `lower-` and `upper-alpha`/`latin`,
  `arabic-indic` and `persian`.
- **Fragmentation:** `break-before`/`-after`/`-inside` (`auto`, `avoid`, `avoid-page`,
  `avoid-column`, `page`, `column`, `left`, `right`, `recto`, `verso`), `page-break-*`, `orphans`,
  `widows`; tables and `pre` blocks split across pages.
- **Multi-column layout:** `column-count`, `column-width`, `columns`, `column-gap` (`normal` is
  `1em`; also `gap` and `row-gap` take `normal`, which is no gap in a grid and a flex container),
  `column-rule` with `-width`, `-style` and `-color`, `column-fill` and `column-span`. The columns
  of a block container are balanced (the least height that holds the content, with the first
  columns the fullest), keep `avoid` breaks, orphans and widows, and end at `break-before` and
  `break-after: column`. A container with a `height` keeps it, and the columns are balanced
  within it. A box and a paragraph that are cut between columns have the border, padding and
  background of each part, and links work. Not supported: a container that is split over pages
  (it moves to the next page as a whole; one that is taller than a page overflows it),
  `column-span: all` (parsed and ignored), `column-fill: auto` (parsed, and the columns are
  balanced), rules other than solid ones (a visible style is painted solid), tables split
  between columns (a table is one box), absolutely positioned boxes that are placed by the
  container, and vertical writing modes (the container is a block).

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
- **`@font-face`:** the descriptors `font-family`, `src`, `font-weight` (also a range) and
  `font-style`. A source is `url()` of a file inside of the directory of the document (relative
  to its style sheet; or `--resource-root`) or of a `data:` URI, or `local()` (an installed font
  by its PostScript or family name); `format()` and `tech()` are read, and sources of formats
  that cannot be read (WOFF, WOFF2) are skipped like the ones that fail to load, so the next
  one is used. Faces are chosen like in CSS Fonts 4 § 5.2 (style, then weight; of equal faces
  the later rule wins), also for the text of SVG images. The faces of a family hide the
  installed fonts of the same name, even if none of its sources loads; then the next family of
  `font-family` is used. Nothing is fetched from the network. Not supported: `unicode-range`,
  `font-stretch`, `font-display`, `font-variation-settings` (a variable font is drawn in its
  default instance), the metric overrides (`size-adjust`, `ascent-override`, ...), a face other
  than the first of a collection, `@font-face` in `@import`ed sheets (`@import` is not
  supported), and `local()` by the full name of a font.
- **Font formats:** TrueType and OpenType (`.ttf`, `.otf`, `.ttc`, with TrueType or CFF
  outlines). Fonts are subset into the PDF. Besides the faces of `@font-face`, fonts come from the
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

Other image formats (GIF, WebP, ...), `url()` anywhere but in `background`,
`@supports`, `@import`, `calc()`, `overflow`, `float`, `position:
fixed`/`sticky`, `z-index`, `opacity` and color alpha, `box-shadow`,
`outline`, `word-break`/`overflow-wrap`,
`counter-*` and `::before`/`::after`, `lh` and other units, scripting, and form
interaction.
