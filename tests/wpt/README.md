# web-platform-tests layout checks

A small, vendored subset of [web-platform-tests](https://github.com/web-platform-tests/wpt)
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
`helium --dump-layout`: the solid rectangles (backgrounds and borders) and the positioned glyph
runs must agree within 0.5px. A test must also not paint anything pure red, following the WPT
convention that red means failure. Because only rectangles and text are compared, a test fails
(rather than passes) when the two documents paint the same pixels through different primitives,
for example one filled box versus four border sides. Expected failures are in
`reftest-expectations.txt`; `cargo test --test wpt_reftest` prints why each one differs.

## Inspecting the output

Every test run also writes the PDF helium produced for each input (tests and references) to
`target/tmp/wpt-pdfs/`, named after the path under `tests/wpt` with `/` replaced by `__`. Open a
test and its `-ref` file next to each other to see what a failing reftest looks like.
