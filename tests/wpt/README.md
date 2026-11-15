# web-platform-tests layout checks

A vendored subset of [web-platform-tests](https://github.com/web-platform-tests/wpt)
(BSD 3-Clause, see https://github.com/web-platform-tests/wpt/blob/master/LICENSE.md), used to compare
helium's layout against the results browsers agree on.

The tests use `checkLayout(...)` from `resources/check-layout-th.js`: elements carry
`data-expected-width`, `data-expected-margin-left`, ... attributes. `helium --check-layout` evaluates
those against the computed layout (see `src/layout/check.rs`); `tests/wpt.rs` runs every file.

Only tests that are fully static can be used, because helium does not run scripts. Tests whose
script modifies the document before calling `checkLayout` are not vendored.

* Add a test: copy it from WPT into the same relative path under this directory.
* Known failures live in `expectations.txt`. They must keep failing, so a fix to helium shows up
  as "unexpectedly passes" and the entry gets deleted.

## Reftests

`tests/wpt_reftest.rs` runs every test that has a `<link rel="match" href="...">`. Helium cannot
rasterize, so the test and its reference are compared through the `paint` section of
`helium --dump-layout`: the regions that the solid rectangles (backgrounds, borders and images of one
colour) paint, with later rectangles covering earlier ones, and the positioned glyph runs must
agree within 0.5px (a box with borders is one region, like a box of the same size). A test must also not paint anything pure red, following the WPT
convention that red means failure. Expected failures are in
`reftest-expectations.txt`; `cargo test --test wpt_reftest` prints why each one differs.

## Fonts

`fonts/Ahem.ttf` (public domain) is the font that tests ask for with `font-family: Ahem`, whose glyphs
are squares of the font size. The runners pass `--font-dir tests/wpt/fonts` to helium, which
stands in for `/fonts/ahem.css` (`@font-face` is not supported).

## Inspecting the output

Every test run also writes the PDF helium produced for each input (tests and references) to
`target/tmp/wpt-pdfs/`, named after the path under `tests/wpt` with `/` replaced by `__`. Open a
test and its `-ref` file next to each other to see what a failing reftest looks like.

## Which tests, and how to add more

The tests are static ones (no script apart from `checkLayout`) of the directories `css/CSS2`
(borders, box-display, margin-padding-clear, normal-flow, visudet), `css-values`, `css-cascade`,
`mediaqueries`, `css-backgrounds`, `css-box`, `css-lists` and `css-color` (reftests), and of
`css-flexbox`, `css-grid`, `css-align`, `css-sizing`, `css-tables`, `css-position` and `css-text`
(only the `checkLayout` tests). The license is in `LICENSE.md`. Most of them fail: the lists of
known failures are the work to do, and `WPT_UPDATE_EXPECTATIONS=1 cargo test --test wpt --test
wpt_reftest` writes them again from what the tests do now (the reasons that were written by hand
stay). The tests are run on all cores. `WPT_ROOT=<dir>` runs the tests of another directory, such
as a checkout of the whole of web-platform-tests, without vendoring it.

A test is vendored if it has no script, no `reftest-wait`, and a reference (or `data-expected-*`
attributes) and all the files it uses are in the checkout.
