# Umbruch

Umbruch turns an HTML document with CSS (and MathML) into a PDF. 

```
umbruch [options] input.html output.pdf
```

## Install

The releases carry a binary for Linux (`x86_64-unknown-linux-musl`) and one for Windows
(`x86_64-pc-windows-msvc`). It is a single file: the fonts of the default `font-family` (DejaVu) are
built in, the fonts installed on the machine are used for everything else, and nothing has to be
configured. There is no macOS binary yet.

### Windows

The binary is built with the MSVC toolchain and links the C runtime statically, so it needs no
redistributable and no other file. It is not signed, so SmartScreen asks once when it is started
from a download: *More info* → *Run anyway*. There is no installer and no file association: unpack
`umbruch-vX.Y.Z-x86_64-pc-windows-msvc.zip` where you want it and put that directory on `PATH`, in
PowerShell:

```powershell
$dir = "$env:LOCALAPPDATA\Programs\umbruch"
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item .\umbruch.exe $dir
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$dir*") {
  [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
}
```

A new terminal then has `umbruch --version` and `umbruch document.html document.pdf`. The fonts are
read from `%SYSTEMROOT%\Fonts` and the two per-user font directories (those of `%USERPROFILE%`);
`--font-dir DIR` adds another one, and `--no-system-fonts` leaves only the built-in families
(DejaVu, TeX Gyre, STIX Two Math, Latin Modern Math), so that the output does not depend on the
machine. Pass on `LICENSE-DejaVu.txt`, which the zip and the tarball carry, with the binary.

### Linux

Unpack `umbruch-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz` (statically linked against musl, so it
needs no shared library, not even fontconfig) and put `umbruch` on `PATH`:

```
tar xzf umbruch-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz
install -m 755 umbruch-vX.Y.Z-x86_64-unknown-linux-musl/umbruch ~/.local/bin/
```

The installed fonts are found by scanning the font directories.

## Python

`pip install umbruch` installs the executable and a small Python API on top of it:

```python
import umbruch

umbruch.HTML("<h1>Hello</h1>").write_pdf("hello.pdf")      # to a path or a binary file object
pdf = umbruch.HTML(filename="report.html").write_pdf()     # bytes
rendered = umbruch.HTML(html_string).render(pdfa="2b")     # rendered.pdf, .pages, .page_size_pt, .warnings
```

The documents are rendered by one long-lived `umbruch --serve` process that starts with the first
request, is started again if it dies, and is stopped at exit, so there is no process per document.
Make a `umbruch.Server(system_fonts=False, font_dirs=[...], resource_root=...)` for other settings
and pass it as `HTML(..., server=server)`; use one server per worker to render in parallel (a server
answers one request at a time). `HTML.render` takes `crop`, `pdfa`, `tags` and `compress`; the page
size comes from the `@page` rules of the document. `HTML.layout(ids)` returns the boxes of elements,
and `Server.measure(probes)` measures text. Errors are `umbruch.Error`, with the `code` of the protocol.
A relative image path is taken from `base_dir` (the directory of the file, or the current directory
for a string). `$UMBRUCH` overrides the executable.

`Server.set_stylesheet(css)` sends a style sheet once (`@font-face` rules, say) that the server remembers: it
applies to every later `render`, `layout` and `measure`, before the style sheets of the document (which win
ties), until another call replaces it (`""` removes it). Replacing it also drops the cached measurements. A
restarted process is sent the style sheet again. Over `--serve` this is the op `set_stylesheet` (`{css,
base_dir?}`), announced by the capability `stylesheet`.

## Options

| Option | Effect |
| --- | --- |
| `--xml` / `--html` | read the input with the XML parser / the HTML parser (by default: XML for the extensions `.svg`, `.xhtml`, `.xht` and `.xml`, HTML for all others) |
| `--no-compression` | write uncompressed content streams (for inspecting the PDF) |
| `--crop` | one page that is as large as the content, instead of A4 pages: no page breaks (also not forced ones), page margins, `@page` rules or margin boxes. The page is as wide as the content needs (what shrink-to-fit comes to with no limit, so text is not broken into lines: give the `body` a `width` to set one) and as high as the content, up to 100 000 pixels, after which it has another page |
| `--no-tags` | write an untagged PDF |
| `--pdfa <level>` | write a PDF/A file: level `1b`, `2a`, `2b`, `2u`, `3a`, `3b` or `3u`. Adds the identification to the XMP packet and an output intent with an sRGB profile (made in code, so there is no file and no licence to care about), which is what makes the device colours that the pages use conformant. The level `a` needs the structure tree (not with `--no-tags`). A CMYK image is an error (the colours are sRGB). `1b` (ISO 19005-1) is written without object streams and with the `CIDSet` of the fonts, and everything that PDF/A-1 does not allow is an error that lists what was found and writes no file: transparency (colours with an alpha channel, `opacity`, gradients with transparent colours, `background-blend-mode`, images with an alpha channel, SVG with opacity, masks or blend modes), colour glyphs, and an image with an ICC profile of version 4. The ICC spaces of `color()` are CIELAB in `1b` (the profiles are version 4), and a gradient with wide colours is clipped to sRGB (with a warning). The levels `1a` and `1u` are not supported. Not checked: the other things that PDF/A forbids, like fonts that may not be embedded |
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
  to 8 bits and the alpha channel written as a soft mask. The ICC profile of a JPEG or PNG file (`iCCP`, or a `cICP` of Display P3) is the
  colour space of the image, written once however many images have it; one that does not fit the image (components, header) is
  ignored with a warning, and an image without a profile is sRGB. PNG gamma and chromaticity chunks are ignored. SVG files are drawn as vector graphics (see below). Sizes follow the
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
  formulas; it can contain `<svg>` again. Gradient stops with `stop-opacity` are painted like those of CSS (a soft mask). Not supported: filters (ignored), GIF and WebP inside of SVG, `!important` in the `style` attribute of SVG
  elements (it does not win over one in a style sheet), a
  `foreignObject` in an SVG file (`<img src="x.svg">`: the content needs the document),
  hyperlinks and tags (accessibility structure) inside of a `foreignObject`.
- **Gouraud triangle meshes in SVG** (an umbruch extension: standard SVG has none, `<meshgradient>`
  was never shipped, and no other renderer draws this): a `<g data-umbruch-mesh="gouraud">` is
  painted as PDF shadings of type 4 (free-form Gouraud triangle mesh) instead of one path per
  triangle, so there are no antialiasing seams between neighbours and the file is small. Two ways
  to write the triangles:
  - *compact*, for programs: the attributes `data-vertices` (six numbers per vertex: x, y, then red,
    green, blue and alpha as sRGB components in 0..1) and `data-triangles` (three indices into the
    vertices per triangle), separated by white space or commas, and no children. Triangles share
    their vertices, so a mesh of a grid is a fraction of the size of the other form and is read
    about ten times faster (20000 triangles: 0.05 s). Anything malformed (a count that is not a
    multiple of 6 or 3, an index out of range, a number that is not finite) is a warning and
    nothing is drawn.
  - *per triangle*, for people (and as a fallback that other renderers show): the children are
    `<path d="M x0 y0 L x1 y1 L x2 y2 Z" data-colors="r0,g0,b0,a0;r1,g1,b1,a1;r2,g2,b2,a2"/>`
    (a `transform` on a path applies, and so does the one of the group); `fill` is what a renderer
    without the extension shows, so the mean colour is a good value. A group with a child that is
    not a valid triangle (not a three-vertex straight path, a missing or malformed `data-colors`,
    a non-finite coordinate) is painted like any other group, with its children.

  The colours are interpolated linearly in sRGB (as Matplotlib does) and written as `DeviceRGB`
  like every other colour, so equal vertex colours give the same pixels as a flat fill. A type 4
  shading has no alpha per vertex: if it is the same everywhere it is the opacity of the mesh,
  painted as a transparency group (poppler ignores the alpha constant of a gradient shading; pdfium,
  mupdf and poppler agree on a group). Otherwise the per-triangle form is drawn with its flat
  triangles and a warning, and the compact form has no flat triangles to fall back to and draws
  nothing. Zero-area triangles are dropped; more than 32768 triangles are split into several
  shadings; the extension is ignored inside of `clipPath`. Over `--serve`, the capability `mesh_gouraud` of `hello` says that
  it is understood; the command line has no such exchange and reads the same markup. PDF/A: an
  opaque mesh is allowed in every level; a translucent one is not in `1b`, like any transparency.
- **SVG and XHTML as the input:** a file with the extension `.svg`, `.xhtml`, `.xht` or `.xml` is
  read as XML (`xml5ever`): namespaces and prefixes are resolved (`<s:svg>`, `<x:div>`, `<m:math>`),
  empty elements (`<div/>`) are empty, and a file that is not well-formed is an error instead of
  being repaired. Entities of an internal DTD are not expanded. The root is an XHTML `html`
  element, laid out as an HTML document, or an `svg` element: `umbruch drawing.svg out.pdf` makes one
  page of the size of the drawing (`width` and `height`, else the `viewBox`, else 300 by 150
  pixels; no margin), with `<foreignObject>` content (HTML, MathML) and `<style>` elements as in
  an inline `<svg>`. Lengths mean the same in the SVG and in the HTML in it (a user unit is a CSS
  pixel, `pt` is 4/3 pixel). Percentages as `width`/`height` are not a size (the `viewBox` is
  used). The structure tree does not reach into a `foreignObject` (no `Formula` tags there).
- **Attributes that change the output:** `href` (external links and `#id` links inside the
  document), `style`, `colspan`, `rowspan`, `<ol start>`, `<ol type>`, `<ul type>`, `<li type>`, `<li value>`, `<progress>` and `<meter>` (a bar), `<col>` widths.
- **Nesting:** elements nested deeper than 512 levels (as in browsers) are flattened at that
  depth instead of overflowing the stack.
- **Tables across pages:** a table is split between its rows, and the header rows (the first
  `thead`) are repeated at the top of every page that goes on inside of the table. Footers are
  not repeated.
- **Grids and flex containers across pages:** a flex container is split like a grid is: between its lines (or items, in a column), and inside items taller than half a page. A grid container whose height is the one of its content is split
  between its rows, where no item spans over the break; the gap between two rows is not at the top
  of the next page. An item that is taller than half a page is cut where its own lines end, and goes
  on at the top of the next page with the border, padding and background of each part, as a table
  row does. `break-before` and `break-after` of the items, and of the boxes in them, force a break;
  `break-inside` and `avoid` values of those boxes are ignored. A grid or flex container with a `height` or `min-height`, an `inline-grid` or `inline-flex`, one inside of a flex or grid
  container, a table cell or a container with columns, and a grid that is the root element are one box.
- **Stylesheets:** `<style>` and `<link rel="stylesheet">` with a path relative to the document.

## CSS

- **Box model:** `width`, `height` (also `min-content`, `max-content`, `fit-content` and `stretch`; not in `min-`/`max-` sizes), `min-`/`max-` width and height, `box-sizing`, `margin`,
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
- **Layout:** `display` (`block`, `inline`, `inline-block`, `flex`, `inline-flex`, `grid`, `inline-grid`, `contents` (no box: the children and the pseudo-elements take the place of the element, which sets no counters),
  `table` and the table
  parts, `list-item`, `none`), `order` of flex and grid items, `inset` and the logical properties (`margin-inline-start`,
  `padding-block`, `inset-inline`, `inline-size`, ... without the logical `border-*`; they follow the
  `writing-mode` and `direction` of the box itself), `aspect-ratio` (also for the images and `<canvas>` boxes, which are blank and 300 by 150 pixels
  unless their attributes say otherwise), `vertical-align` (on inline boxes, and `top`, `middle` and `bottom` in table cells; the baselines of
  the cells in a row are not aligned), `position: absolute`, `position: fixed` (an absolutely positioned box of the first page: it is not repeated on the other pages) and `position: relative` (also `sticky`; `left`, `right`,
  `top` and `bottom` move the box after layout, also inline boxes, `inline-block` boxes and formulas, in
  vertical text too; percentages of `left`/`right` in vertical lines count as `auto`), block, inline, flexbox
  (`flex-*` with `flex-flow`, `gap` with one or two values, `justify-*`, `align-*`, `place-*`) and grid (`grid-template-*` and `grid-auto-*` with `repeat()` (a count, `auto-fill` and `auto-fit`), `minmax()` and `fit-content()`,
  `grid-column`, `grid-row`, `grid-area` (as four lines: no names), also for absolutely positioned children, which are placed in their grid area) layout, tables (`border-collapse`, `border-spacing`).
- **Text:** `font` and its longhands (`font-family`, `font-style`, numeric and keyword
  `font-weight`, `font-size`, `line-height`), `color`, `text-align` (including `justify`),
  `text-indent`, `text-decoration`, `text-transform`, `letter-spacing`, `word-spacing` (a length), `white-space` (`normal`, `nowrap`, `pre`, `pre-wrap`, `pre-line`), `hyphens`,
  `direction`, `unicode-bidi`, `list-style-type`, `list-style-position` and the `list-style` shorthand (`<image>`
  markers are not supported; see generated content below).
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
  (including `cover` and `contain`), `-position`, `-repeat`, `-blend-mode`, `-origin` and `-clip` (`border-box`,
  `padding-box` and `content-box`; boxes with rounded corners are always clipped to the border
  box). A relative URL in a style sheet is relative to the style sheet, and the file has to be
  inside of the directory of the document (or `--resource-root`) like the images of `<img>`. In a
  background with a blend mode, the images are painted over the blended layers, without being
  blended. Text in an SVG image that is a background is set in the system and built-in fonts, not
  in the `@font-face` fonts of the document. The background color of an inline box (`<mark>`, a
  `<span>`) is painted behind its text over the em box of the font plus the vertical padding and border, and the horizontal margin, border and padding at its ends take room on the line
  (a box that wraps has them at its first and last line only). Borders are drawn solid, whatever their style, and the vertical padding and border do not change the height of the line; background images of inline boxes are not painted. Percentages in the padding and margin of an inline box count as 0, and none of this applies in vertical lines.
- **Shadows:** `box-shadow` with offsets, blur radius, spread, colour and several shadows. The
  blur is a Gaussian, drawn as a stack of translucent shapes (vector, no raster image). The
  shadow is clipped to the outside of the border box, follows `border-radius`, and is painted
  for boxes that are not split over pages (a fragment of a split box has none). `inset` shadows
  are not drawn, and `text-shadow` is not supported.
- **Colors:** named, hex, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`,
  `color()`. A colour is written in the space that CSS wrote it in: `rgb()`, `hsl()`, `hwb()`, hex codes,
  names, `color(srgb …)` and an `oklab()` or `oklch()` that sRGB holds are device RGB, which every page and form defines as sRGB
  (`DefaultRGB`); a `color()` in Display P3, Adobe RGB (`a98-rgb`), ProPhoto RGB, Rec. 2020 (gamma 2.4, as CSS says) or a
  linear space is in an ICC space of that profile (written once); `lab()`, `lch()`, `color(xyz …)`, an `oklab()` or `oklch()` that sRGB cannot
  hold, and a colour outside of the space it was written in are CIELAB (D50; the PDF has no Oklab, so those are converted), or, with a\* or b\*
  beyond ±128 (an iPhone does not show Lab right there), in the ICC space of Rec. 2020 if that holds the colour, else with its chroma reduced. Gradients (axial and radial; a conic one is a mesh in sRGB) and `color-mix()` keep a wide colour too: a gradient with
  one is a shading in an ICC space with the primaries of Rec. 2020 and a gamma of 2.4 and is cut into steps, interpolated in the space of the gradient. SVG (inline, as a file or as an image) reads the same colours in `fill`, `stroke`, `stop-color`, `color` and the `style` attribute and style sheets, with the code of the style sheets, and writes them the same way (the stops of a gradient with a wide colour are in the ICC space of the wide colours, in `1b` they are clipped to sRGB with a warning); the layers of
  colour glyphs are still sRGB, and a wide colour is clipped there; so is a wide colour in a filter of SVG, which is ignored anyway), with an alpha channel (`rgb(0 0 0 / 50%)`, `#rrggbbaa`, `transparent`) that is
  painted for backgrounds, borders, text, text decorations and gradients. `color-mix()` mixes
  two colours (also nested, and with `var()`) in `srgb`, `srgb-linear`, `hsl`, `hwb`, `lab`,
  `lch`, `oklab`, `oklch`, `xyz`, `xyz-d50` and `xyz-d65`, with percentages, alpha premultiplied
  and the hue methods; gradients take `in <space>` (also with a hue method, before or after the
  direction): the colours between the stops are interpolated in that space, by cutting every
  transition into up to 24 stops of sRGB. `color-mix()` is computed when the style sheet is read, so `currentcolor` cannot be
  mixed (the declaration is dropped), nor the RGB spaces with a wide gamut, nor `none`.
- **Opacity:** `opacity` (a number or a percentage; not inherited). A box with an opacity below 1
  and everything inside of it are painted as a transparency group, which is made translucent as a
  whole. The content of such a group is not tagged in the structure tree, and a box that is
  split over pages is translucent on its first page only (like `transform`). The opacity of a
  non-replaced inline box is multiplied into the alpha of the colors of its text and its
  decorations instead, so overlapping parts of it show through each other.
- **Generated content:** the `::before` and `::after` pseudo-elements (also written with one
  colon) with `content` (strings, `counter()`, `counters()`, `attr()` with an optional string
  fallback, `url()` images, and `open-quote`, `close-quote`, `no-open-quote`, `no-close-quote`;
  `none` and `normal` generate nothing), `counter-reset`, `counter-increment`, `counter-set`
  (scoped as in CSS Lists 3; elements without a box do not count) and `quotes` (`auto` picks the
  marks of the language of the text for the common languages). A pseudo-element is a box of its
  own, so `display`, `position`, borders, backgrounds and the like apply to it. A `url()` in
  `content` is an inline image of its natural size (SVG, PNG and JPEG, and in the same places
  as the images of `<img>`; a relative URL is relative to the style sheet) that the rules of the
  document for `img` do not reach. No pseudo-elements are generated for replaced elements and
  other elements without content (`img`, `input`, `br`, `hr`, `video`, `canvas`, `iframe`, ...):
  CSS 2.1 leaves their interaction with `:before` and `:after` undefined, and browsers do not
  make them.
  `::marker` styles the marker of a list item (`color` and the font properties, and `content`,
  which replaces the marker text: `none` removes the marker and a text is the whole marker, with
  no space added after it; the `counter-*` properties work on the counters of the item, so
  `content` can use them); `::before::marker` and `::after::marker` do the same for a `::before`
  or `::after` that is a list item. An outside marker is drawn next to the first line and is not
  a box of its own: it takes no `display`, `position` or the like. `list-style-position:
  inside` puts the marker in front of the content of the item as an inline box. The
  `list-style-type` is a keyword (`disc`, `circle`, `square`, `decimal`, `lower-alpha`,
  `upper-alpha`, `lower-roman`, `upper-roman`, `none`) or a string, which is the whole marker.
  The counter styles of `counter()` are the ones of the `@page` margin boxes (which share the
  code) plus `lower-greek`, `georgian`, `disc`, `circle`, `square` and `none`. `display:
  list-item` (the default of `<li>`) counts in the `list-item` counter, which `<ol>`, `<ul>` and
  `<menu>` start and `<ol start>` and `<li value>` set; the number of the marker is the value of
  that counter, so `counter-increment: list-item` and the like change it. Not supported: `content`
  on elements (e.g. `content: url()` on an `<a>`), `::first-line` and `::first-letter`,
  and `list-style-image`. Counters in `@page` margin boxes are only `page`
  and `pages`. A counter value stays within a 32 bit integer (an increment that would leave the
  range is not made), and a name that is repeated in one `counter-*` property counts every time.
  The behaviour is the one of the published CSS Lists 3 (a counter of the previous sibling is only
  inherited when the parent has none of that name); `counter-reset: reversed()` and `<ol
  reversed>`, which are only in the editor's draft, are not supported.
  Cross-references (CSS GCPM 3): `target-counter(target, name, style)`, `target-counters(target,
  name, separator, style)` and `target-text(target)` read the counter and the text content of the
  element that a link points at, which may come after the reference (a table of contents). The
  `target` is `attr(href)` (or another attribute, with or without the `url` type) or a `url()` or
  string; only the fragment (`#id`) is followed. The counter `page` is the page number (the value
  of the `page` counter on the page where the element starts, in the given style). `target-text()`
  has no second argument but `content`; its text leaves out generated content and elements that are
  not displayed, and white space is collapsed. A target that does not exist (or has no page)
  gives nothing. The page of a target is known after pagination, so a document that reads any of
  these is laid out again with what the previous layout found, until the page numbers stop
  changing, but at most five times (the last layout is the result, with a warning, if they do
  not settle). A document without them is laid out once. Not supported: `target-text()` with
  `before`, `after` and `first-letter`.
  `leader(type)` (CSS Generated Content 3, 2.5, a Working Draft that calls itself a very rough
  draft): `dotted` (the string `.`), `solid` (`_`), `space` (a space) or a string, which is repeated
  over what is left of the line, so that `a::after { content: leader('.') target-counter(attr(href),
  page) }` makes the entry of a table of contents. The line that has a leader is as wide as the
  paragraph whatever its `text-align` (a justified line gives the room to the leader, not to its
  spaces), the text after the leader is at the end of the line, and the string is drawn at the places
  of a grid that starts at the start of the line, so that the leaders of lines that follow each other
  line up (with a `text-indent`, the first line has its grid moved by it). Only whole copies of the
  string are drawn. The leader has no width when the lines are broken: it takes what is left of the
  line that the text before and after it ends up on, and if there are several on a line, they share
  it. It is an artifact, not text of the document, and has the font and the colour of the element
  that it is in. Not supported: a leader in vertical text (it takes the room, but draws nothing),
  and moving the text after a leader to the next line when no whole copy of the string fits.
- **Units:** `px`, `pt`, `pc`, `in`, `cm`, `mm`, `em`, `rem`, `ex`, `ch`, `vw`, `vh`, `vi`, `vb`, `vmin`, `vmax` (relative to the page area), `%`.
- **Math functions** (CSS Values 4): `calc()`, `min()`, `max()`, `clamp()` (with `none`),
  `round()` (`nearest`, `up`, `down`, `to-zero`), `mod()`, `rem()`, `sin()`, `cos()`, `tan()`,
  `asin()`, `acos()`, `atan()`, `atan2()`, `pow()`, `sqrt()`, `hypot()`, `log()`, `exp()`,
  `abs()`, `sign()` and the constants `e`, `pi`, `infinity`, `-infinity` and `NaN`, nested and
  with `var()`, wherever a length, a length-percentage, a number, an integer or an angle is
  accepted (sizes, margins, padding, insets, borders, radii, gaps, grid tracks, flex, fonts,
  `line-height`, `vertical-align`, `text-indent`, spacing, shadows, transforms, gradients,
  backgrounds, counters, `opacity` and media queries). They are type-checked as the
  specification says, with typed arithmetic (`calc(1em * 1em / 1px)` is a length); a value that
  is out of the range of its property is clamped (`padding: calc(50% - 10em)` is never
  negative), `NaN` is 0 and infinity is the largest length. A percentage that is not just added
  to a length (`min(100%, 40em)`) is kept until layout knows what it refers to. Not supported:
  `<time>`, `<frequency>` and `<resolution>` values, and a number that divides relative lengths
  (`calc(1em / 1px)`) except in media queries.
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
- **Pages (`@page`):** `size`, `margin`, the `:first`, `:left` and `:right` selectors (`:first` is
  the first page of the document, also in `name:first`, which matches it only if it is a page
  with that name: CSS Paged Media 3, 4.2.2), `:nth(An+B)` and `:nth(An+B of name)` (CSS GCPM 3,
  3.1 and 3.2, below), named pages (`page` property), the six margin boxes (`@top-left` ... `@bottom-right`) with `content`
  from strings and `counter(page)` / `counter(pages)`, in the styles `decimal`,
  `decimal-leading-zero`, `lower-` and `upper-roman`, `lower-` and `upper-alpha`/`latin`,
  `arabic-indic` and `persian`. The `page` counter is a real counter (CSS Paged Media 3, § 6.1):
  `counter-reset`, `counter-increment` and `counter-set` of `page` in an `@page` rule apply to the
  pages that it selects, in the order of CSS Lists: the page counter is reset, then incremented
  (by 1 unless `counter-increment` says otherwise), then set. So `@page :first {
  counter-reset: page 0 }` makes the first page of the document page 1, `@page :nth(1 of chapter) {
  counter-reset: page 0 }` the first page of every chapter, and `@page {
  counter-increment: page 2 }` numbers the pages 2, 4, 6. Not supported: `counter-reset` of
  `page` on an element (the specification does not say what it does), and changing `pages`
  (which, as in the specification, has no effect).
  **Page groups and `:nth()`** (CSS GCPM 3, a Working Draft): an element with a `page` name and a
  forced break (`break-before` or `break-after` of `page`, `left`, `right`, `recto` or `verso`) makes
  a *page group* of the pages that it has (`div.chapter { page: chapter; break-before: page }`: every
  chapter is a group). `:nth(An+B)` selects pages of the document by their number from 1, and
  `:nth(An+B of name)` the pages in the groups of that name, counted in the group: `@page :nth(1 of
  chapter)` is the first page of every chapter, `:nth(odd of chapter)` its odd pages. A page can be
  in several groups (a chapter in a part), and is counted in the innermost one of a name. An element
  makes a group only if it has the name itself: one that inherits it from its parent does not (so a
  chapter inside another element of the name `chapter` is not a group of its own). `:nth()` has the
  specificity of `:first`. It works in every `@page` rule except for `size` and `margin`: the
  pages are chosen before the number of a page in its group is known, so these follow `:nth(1 of
  name)` (the first page of a group, which is known) and not other `:nth()` selectors, which have
  no effect on them (the margin boxes, the `page` counter and `-umbruch-page-label` of the rule
  apply). There is no `name:first` for a group: `:first` is the first page of the document.
- **Fragmentation:** `break-before`/`-after`/`-inside` (`auto`, `avoid`, `avoid-page`,
  `avoid-column`, `page`, `column`, `left`, `right`, `recto`, `verso`), `page-break-*`, `orphans`,
  `widows`, and `looseness` (not in CSS: how many lines more or fewer than the best breaking a
  paragraph may have, so that a page that fits it better can take it); tables and `pre` blocks split across pages.
- **Floats in the manner of TeX:** `float: top` and `float: bottom` (CSS GCPM) for a box between
  the blocks of a block container. The box is taken out of the text and shown at the top or the
  bottom of the page that the place where it is called for is on, if it fits next to the text there
  (floats take at most 70% of such a page), else at the top of a later page; floats keep their
  order, and a float that cannot share a page with text, or that waits at the end of the document,
  gets a page of floats. In a container with columns `float: top` and `bottom` place a float in a
  column, as wide as the column (floats that wait go on in the next column and over the page
  break), and `float-reference: page` makes it a float of the page, as wide as the page, which is
  placed at the top or bottom of the next page and takes room from the container that continues
  there. Not supported: floats in paragraphs, in tables and in flex and grid containers, and
  `float: left` and `right`.
- **Multi-column layout:** `column-count`, `column-width`, `columns`, `column-gap` (`normal` is
  `1em`; also `gap` and `row-gap` take `normal`, which is no gap in a grid and a flex container),
  `column-rule` with `-width`, `-style` and `-color`, `column-fill` (`balance`, `balance-all` and
  `auto`) and `column-span`. The columns of a block container are balanced (the least height that
  holds the content, with the first columns the fullest), keep `avoid` breaks, orphans and widows,
  and end at `break-before` and `break-after: column`. A container that is taller than the rest
  of its page is split over pages: the columns of every page are filled in turn, a new row of
  columns starts on the next page, and only the last fragment is balanced (with `column-fill:
  auto` none is, and with `balance-all` the ones that end at a forced break are). A forced page
  break in a container ends its fragment, the columns are found for the width of every page, and
  the rules are drawn in every fragment. A box and a paragraph that are cut between columns or
  pages have the border, padding and background of each part, and links work. A container with a
  `height` keeps it, and the columns are balanced within it. Not supported: a container with a
  `height`, `min-height` or `max-height` that is split over pages (the columns that do not fit
  overflow below the last one), a container that is inside of another container with columns, of a
  flex or grid container or of a table cell that is split (it is one box), `column-span: all`
  (parsed and ignored), rules other
  than solid ones (a visible style is painted solid), tables split between columns (a table is one
  box), absolutely positioned boxes that are placed by the container, and vertical writing modes
  (the container is a block).

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
  auto` and soft hyphens), CJK line breaking, justification. Paragraphs are broken as a whole, as
  TeX does (Knuth and Plass): the badness of a line is 100 times the cube of how far its spaces
  stretch or shrink, a line has a penalty, a line that is much looser or tighter than the one before
  it has demerits, so does a hyphen after a hyphen and a hyphen at the end of the line before the
  last of a paragraph, and a hyphen has its own penalty.
  `hyphenate-limit-chars` (CSS Text 4: `[ auto | <integer> ]{1,3}`, the fewest characters of a word
  that is hyphenated, before the hyphen and after it; a missing second number is `auto`, a missing
  third the second; `auto` is 5, 2 and 2, which the specification suggests; TeX's English patterns
  use 2 and 3) and `hyphenate-limit-last` (`none`, `always`, `column`, `page`, `spread`; inherited)
  set the limits. The last line before a break should not end in a hyphen, says the specification,
  which asks for it: `always` is about the last full line of the paragraph (the line before the last,
  for which TeX has 5000 demerits, which `none` keeps, and `always` raises to 100 000, so that it is
  avoided unless every other breaking is much worse) and about the last line of every page and
  column; `column` about every page and column; `page` about every page (the last line of a column
  is not); `spread` about the pages at the end of a spread (the right pages, and the first page; in a
  document from right to left, the left pages). A page that ends in a hyphen where it should not costs
  as much as 2.5 lines of room, so it ends a line early if there is no other way; the pages choose
  among the ways to break a paragraph if it has a `looseness`, and then a way that does not end in a
  hyphen costs less than a short page. This is TeX's `\brokenpenalty`, with a size that was chosen
  on a novel: it avoids most of the hyphens at the end of a page for pages that end a line early
  (see `CssObjective::HYPHEN_END_LINES`). Not supported: `hyphenate-limit-lines` (a limit on the
  number of hyphens in a row, which TeX only counts demerits for, as here), `hyphenate-limit-zone`
  and `hyphenate-character`.
  `hanging-punctuation` (CSS Text 3, which marks it at risk; inherited): `first` lets an opening
  bracket or quote (the categories Ps, Pf and Pi, the apostrophe, the quotation mark and the
  ideographic space) at the start of the first line of a paragraph hang outside of the line,
  `last` a closing bracket or quote (Pe, Pf and Pi, the apostrophe and the quotation mark) at the
  end of the last line, `force-end` a stop or comma (the 13 of the specification) at the end of a
  line, and `allow-end` such a stop or comma only on a line that does not fit without it. As the
  specification lets a user agent include other characters, the hyphen of a hyphenated word and
  U+002D and U+2010 at the end of a line hang with the end values too, as the hyphens of a
  book do in TeX with microtype. What hangs is not part of the line for fit, alignment and
  justification: the breaker (which is the same for all of them) measures the lines without it,
  and the justified lines are stretched to the width of the line without it, so that the edge
  of the text is straight and the punctuation stands in the margin. One character hangs at each
  edge. It is the same measure as the breaker uses, so a line that has more room because of what
  hangs can take another word. Not supported: hanging in right-to-left text and in vertical text
  (it is ignored there), a margin that is not wide enough (what hangs overflows the box, as the
  specification says), the intrinsic sizes (the width of a paragraph in shrink-to-fit still
  counts what hangs), an opening quote at the start of a line that is not the first of its
  paragraph (the specification's `first` is about the first line only), and the characters that
  TeX's microtype lets protrude by part of their width (letters like T and V, and the quotes and
  the hyphen by less than their width).
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
  The Noto fonts for the other scripts, symbols, colour emoji and CJK (about 18 MB of the
  binary) are built in by the cargo feature `embedded-fonts`, which is on by default;
  `--no-default-features` (add `--features fontconfig` on Linux if you want it) leaves them out and
  characters of those scripts are then only drawn from installed fonts or `@font-face`.

## PDF output

PDF 1.7; Flate-compressed streams; tagged PDF with a structure tree derived from the HTML
(headings, paragraphs, lists, tables, links, figures, formulas, and tables of contents) and the PDF/UA identifier;
document title and language; metadata from `<meta name>` (`author`, `description`, `keywords`, `generator`, `date`) in the document information dictionary and the XMP packet (there is a date only from `<meta name="date">` or `SOURCE_DATE_EPOCH`, so that the output does not depend on the time); the outline panel opens when there are bookmarks; page labels (the `-umbruch-page-label` of the `@page` rules, which is not in CSS: `auto` takes the style of the one `counter(page)` that the headers and footers of the page have between them, and the decimal number if there are none or several, `none` the decimal number, or `"A-" counter(page, upper-roman)` a prefix and the style of the number; the text around a counter in a header or footer is never a prefix); named destinations for the `id` attributes, so that `document.pdf#id` opens the PDF at that element; link annotations; a document outline (bookmarks)
from the elements with a `bookmark-level` (UA default: `h1` to `h6` and `role=heading` with `aria-level`; entries nest by level, elements that are not displayed are left out), with `bookmark-label` (strings, `content()`, `attr()`) and `bookmark-state`; `ToUnicode` maps and
`ActualText`, so that text can be extracted and copied; embedded font subsets; clipped and
rounded boxes; axial, radial and mesh shadings for gradients; blended backgrounds; transparency
groups for `opacity`.

A table of contents is tagged as one (ISO 32000-1, 14.8.4.3) if it has the role `doc-toc` (DPUB-ARIA,
`<nav role="doc-toc">`): it is a `TOC`, its `<li>` are `TOCI`, a list in an item is a `TOC` of its
own after the item (its sub-entries), and a link in an item is a `Reference` that holds the `Link`.
The number of an item is its `Lbl`. A list directly in the table is not a structure element, and
block-level `<p>`, `<div>` and `<a>` in it are items, so a table of contents that is not a list
works too. The `leader()` of an entry is an artifact. The page numbers (`target-counter()`) are
text of the item.

### Conformance

The gallery is validated with [veraPDF](https://verapdf.org) for PDF/UA-1 and for every level of
`--pdfa` (the documents without transparency for `1b`; `tests/conformance.rs`; the test is skipped unless `VERAPDF_LIB` is the directory with the
jars of veraPDF, which `tools/fetch_verapdf.sh DIR` downloads to `DIR/lib`). All documents conform
but one: the sidebar of the `cv` has headings before the `h1`, which PDF/UA-1 does not allow. This
says that veraPDF finds nothing wrong in the gallery, which is not a proof for every document: it
checks what a program can check, and not for example whether the reading order makes sense.

## Not supported

Other image formats (GIF, WebP, ...), `url()` anywhere but in `background`,
`@supports`, `@import`, `overflow`, `float: left`/`right`, `position:
fixed`/`sticky`, `z-index`, `text-shadow`, inset shadows,
`outline`, `word-break`/`overflow-wrap`,
`counter-*` and `::before`/`::after`, `lh` and other units, scripting, and form
interaction.
