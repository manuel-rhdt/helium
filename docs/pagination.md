# Pagination: design

Status: proposal. Nothing described here is implemented yet. The section
"Where the code stands" describes the code as of this writing; everything after
it is the plan.

## Goals

1. Text flows across pages of differing geometry: `@page :first` without a
   forced break, left and right pages of different widths, named pages in the
   middle of the flow, and a `size` per page.
2. Line breaks and page breaks are chosen together: a paragraph may run a line
   longer or shorter if that gives better pages.
3. Solvers are cheap to write and to compare. A greedy solver comes first, an
   exact one second, and later ones may be found by search (OpenEvolve or
   similar). A solver must not be able to produce an invalid layout.
4. Grid, flex and table containers can eventually be split across pages,
   including rows whose cells each continue on the next page.

## Non-goals

- Floats and footnotes. `float` is not supported by helium today. The design
  leaves room for them (see "Open questions") but does not cover them.
- Multi-column layout. A column is a fragmentainer like a page, and the kernel
  could serve it, but nothing here plans for it.
- Forking Taffy to make it fragmentation-aware.

## Where the code stands

The pipeline in `src/layout/mod.rs` (`layout`) is:

1. `FindPossibleLineBreaks` finds the break opportunities of every paragraph.
2. `TaffyBlockLayout` lays out the whole document once, as an endless galley
   at the content width of the plain `@page` rule. Paragraphs are measured
   inside Taffy's measure function (`HeliumTree::compute_inline_layout`, which
   calls `InlineFormattingContext::compute_lines`), and broken for real when
   the positions are written (`Writer::write_lines`).
3. `Paginator` (`src/layout/paginate.rs`) collects break opportunities from the
   `TextList`, with the y position of each, and chooses breaks with the same
   least-weight-subsequence routine that is used for lines
   (`paragraph::compute_breakpoints`). The cost of a page is the cube of its
   unused height, plus penalties for `avoid` and for orphans and widows.
4. The paginator then moves every box to its page, and splits the decoration
   of boxes that contain a break.

Limits of this, in order of importance:

- **One width.** The galley has one width. Two workarounds exist. Page groups
  (`taffy_layout/page_groups.rs`) wrap content that is delimited by forced
  breaks in a box of another width. Left and right pages are shifted sideways
  (`PageMargins::shift`), which is only right if their margins have the same
  sum; `PageMargins::left_right_conflicts` warns otherwise. `@page :first`
  with other margins has no effect on the width unless a forced break follows
  the first page.
- **The page class is guessed.** The paginator needs the height of the page
  that starts at a break before it knows the number of that page.
  `PageMargins::edges_of_page_start` therefore uses the base margins when left
  and right pages differ in their top or bottom margin, and the code treats
  the index of a break point as if it were a page number (`number == 0`).
- **One page size.** `PagesCreate` and the viewport units take a single width
  and height.
- **Lines are fixed before pages.** The paginator can only cut between the
  lines that the paragraph breaker chose for the galley.
- **Monolithic containers.** Flex and grid containers and table rows are never
  split. Tables are laid out as grids (`taffy_layout/table.rs`) and split
  between rows only.
- **Margins at the end of a page count.** A page ends where the next element
  starts, so the margin in front of an unforced break is counted against the
  height of the page before it. The margin is only discarded at the start of
  the next page. The kernel discards it on both sides; the flow that is
  extracted from the galley has no `Gap` items yet and keeps the old behaviour.
- **A page can span a forced break.** A forced break has a large negative
  cost, not an infinite one. Where the pages on both sides of it would
  overflow, the paginator may ignore it. `Problem::stop_at_forced` is off to
  keep this, and goes away with the galley.
- **`break-before: left`/`right`** force a break but never insert the empty
  page that may be needed.

Two properties of the current code that the design builds on:

- helium implements Taffy's tree traits itself (`HeliumTree` in
  `taffy_layout/tree.rs`) instead of using `TaffyTree`. It controls which
  children and which styles Taffy sees.
- `paragraph::compute_paragraph_breakpoints` already takes the line width as a
  function of the line number.

## Overview

The paginator runs the outermost loop. Taffy answers its requests and never
lays out the document as a whole.

```
            +-----------+     candidates      +---------+
            |  Solver   | <-----------------> | Kernel  |
            +-----------+   (pages that can   +---------+
                  |          start here)           |
                  | Solution                       | queries
                  v                                v
            +-----------+   layout_page     +----------------+
            |  Apply    | ----------------> | Layout service |
            +-----------+                   | (Taffy, lines) |
                                            +----------------+
```

- The **layout service** knows the box tree. It answers small, cached
  questions (how wide is this paragraph on a page of this width, how tall is
  this unbreakable box) and lays out one chosen page for real.
- The **kernel** knows the rules of fragmentation: which breaks are legal,
  what is discarded at a page edge, how orphans and widows are counted, which
  page comes next. From a page start it enumerates the pages that can start
  there.
- A **solver** is a policy over those candidates. It returns a sequence of
  chosen pages.
- An **objective** gives the cost of a page. It is a parameter of a solver.
- **Metrics** describe a finished pagination. They are independent of any
  solver and of any objective.

The kernel, the paragraph breaker, the solvers and the metrics live in a crate
of their own, `helium_paginate`, next to `helium_derive`. It has no dependency
on the ECS, on fonts or on Taffy, so it builds in seconds and can run on
recorded problems.

## Concepts

### Pages

```rust
pub enum PageKind { First, Left, Right }

pub struct PageSpec {
    pub inline_size: f32,   // width of the content area
    pub block_size: f32,    // height of the content area
}

pub struct PageStart {
    pub at: BreakToken,
    pub index: usize,       // page number, starting at 0
}
```

The geometry of a page follows from its `PageKind`, from the page name of the
first content on it, and from whether it starts a page group. The name and the
group are properties of the break at which the page starts: a change of the
name already forces a break. The only thing that depends on the path taken is
the `PageKind`. The full state of a solver is therefore
(`BreakToken`, `PageKind`), which keeps an exact search small.

The inline size of a page is known before anything is laid out for it. A
layout is never discarded because of its width, only because of where it ends.

### Break tokens

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BreakToken(u32);
```

A break token says where a page starts. It is opaque and hash-consed. Only the
kernel and the layout service create tokens; a solver compares and stores
them.

Behind the handle, a token is a position in the flow:

```rust
enum Position {
    /// In front of an item of the flow.
    Item(ItemId),
    /// Inside a paragraph of the flow.
    InParagraph { item: ItemId, cut: Cut },
    /// Inside a container that is split: one position per parallel part
    /// (the cells of a grid row, the items of a flex line).
    InContainer { item: ItemId, parts: Vec<BreakToken> },
}
```

`InContainer` is the reason for the opaque handle. A break inside a grid row
cuts several cells at once, and a flat index cannot say that. The first
implementation only has the first two variants.

### The flow

The content that can be split is a sequence of items, in the manner of TeX:

```rust
pub enum Item {
    /// Cannot be split: a monolithic box, or the padding and border of a box
    /// that is open around the items that follow.
    Box { node: NodeRef },
    /// Collapsed margins. Discarded at the start and at the end of a page;
    /// kept after a forced break.
    Gap { node: NodeRef },
    /// A place where a page may end.
    Break { kind: BreakKind, name: Option<NameId>, starts_group: bool },
    /// A paragraph of the flow.
    Paragraph { id: ParId, orphans: u8, widows: u8, avoid_inside: bool },
    /// A container that can be split between or inside its parts.
    Container { node: NodeRef },
}

pub enum BreakKind { Allowed, Avoid, Forced, ForcedLeft, ForcedRight }
```

Items carry no sizes. The kernel asks the layout service for the extent of a
`Box` or `Gap` on a page of a given inline size.

Nesting is flattened when the flow is extracted, as the paginator does today:
no `Break` is emitted inside a monolithic box, and `break-inside: avoid`
turns the breaks inside the box into `Avoid`.

### Paragraphs and cuts

```rust
pub struct Cut {
    pub pos: TextPos,     // index of a break opportunity of the paragraph
    pub fitness: u8,      // fitness class of the line in front of the cut
    pub hyphen: bool,     // whether that line ends in a hyphen
}
```

A page break inside a paragraph is a text position, not a line number: a line
number means nothing once the width changes. The cut also carries the state of
the line breaker, so that the demerits for adjacent lines of very different
tightness and for consecutive hyphens are counted across the page break. The
breaker in `paragraph.rs` has 8 such states (4 fitness classes, times whether
the line ends in a hyphen).

Paragraphs come in two kinds:

- **Flexible**: all lines have the same height. The extent of the paragraph is
  its line count times that height. These paragraphs take part in the joint
  optimisation.
- **Rigid**: lines of differing height (inline math, `inline-block` boxes,
  mixed font sizes). The paragraph is broken once per width, as today, and a
  page can only cut between those lines.

The restriction keeps `InlineFormattingContext::line_height`, with its
vertical alignment and atoms, out of the kernel. It can be lifted later by
making profiles Pareto fronts over real extents.

For a flexible paragraph the breaker computes *profiles*: for each line count
within a tolerance of the optimum, the least demerits.

```rust
pub struct Profile { pub line_height: f32, pub entries: Vec<ProfileEntry> }
pub struct ProfileEntry { pub lines: u32, pub demerits: f64, pub end: Cut }

pub trait Paragraphs {
    /// The whole paragraph.
    fn whole(&mut self, id: ParId, width: f32) -> &Profile;
    /// From `from` (`None`: the start) to every cut that can be reached.
    fn heads(&mut self, id: ParId, width: f32, from: Option<Cut>) -> &CutTable;
    /// From a cut to the end of the paragraph.
    fn tail(&mut self, id: ParId, width: f32, from: Cut) -> &Profile;
    /// The breaks of one entry, to materialise the lines.
    fn breaks(&mut self, id: ParId, width: f32, from: Option<Cut>, to: Option<Cut>,
              lines: u32) -> Vec<TextPos>;
}
```

The tolerance is the equivalent of TeX's `\looseness`. It bounds the runtime
and how bad a paragraph may get in order to save a page.

The profile tables are the existing dynamic program in
`compute_paragraph_breakpoints` with one more dimension, the line count. That
program is quadratic in the number of opportunities of a paragraph and keeps no
list of active nodes. It needs one before it is run per width and per cut.

`paragraph.rs` moves into `helium_paginate`. The live and the recorded
implementation of `Paragraphs` then run the same code.

### Page fills

```rust
pub struct PageFill {
    pub end: BreakToken,
    pub spec: PageSpec,
    pub used: f32,
    pub end_kind: BreakKind,
    pub orphan_violation: bool,
    pub widow_violation: bool,
    pub hyphen_at_end: bool,
    /// A single unbreakable item is taller than the page.
    pub overflow: bool,
    /// The demerits of the lines on the page.
    pub line_demerits: f64,
    /// The line counts chosen for the paragraphs on the page.
    pub layout: LayoutId,
}
```

Because paragraphs can trade demerits for height, a page is not determined by
its start and its end. For one end there can be several fills: the Pareto
front in (`used`, `line_demerits`).

## The kernel

```rust
pub struct Problem { pub pages: PageTable, pub items: Vec<Item> }

impl Kernel<'_> {
    pub fn first_page(&self) -> PageStart;

    /// Every page that can start at `start`, by growing fill. Stops at a
    /// forced break and at the first item that does not fit.
    pub fn candidates(&mut self, start: &PageStart) -> Vec<PageFill>;

    /// The page after `fill`, or `None` at the end of the document. Inserts
    /// an empty page for `ForcedLeft` and `ForcedRight` where needed.
    pub fn next_page(&mut self, start: &PageStart, fill: &PageFill) -> Option<PageStart>;

    /// Checks a solution and returns its pages.
    pub fn realize(&mut self, solution: &Solution) -> Result<Vec<PageFill>, Invalid>;
}

pub struct Solution { pub pages: Vec<(BreakToken, LayoutId)> }
```

`candidates` walks the items from the start of the page and extends a set of
labels (position, extent used, demerits so far). Labels that are dominated in
extent and demerits at the same position are dropped. At every legal break it
emits a fill. All rules of fragmentation are in this one function:

- gaps at the start and the end of a page are discarded, except after a
  forced break (https://www.w3.org/TR/css-break-3/#break-margins);
- the lines of a paragraph on the page are counted against `orphans` and
  `widows`;
- an unbreakable item that is taller than the page gets a page of its own and
  overflows it, so that the content after it is not lost;
- no break between table rows that a cell spans.

`realize` is used by the apply step and by the metrics, so the two cannot
disagree about what a solution means.

## Solvers and objectives

```rust
pub trait Solver {
    fn solve(&self, kernel: &mut Kernel) -> Solution;
}

pub trait Objective {
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> f64;
}
```

Planned solvers:

- **Greedy.** Take the candidate with the largest fill. Step back while it
  violates `avoid`, orphans or widows and an alternative exists. One call of
  `candidates` per page.
- **Exact.** Shortest path over (`BreakToken`, `PageKind`), memoised, with an
  `Objective`. With the tolerance of all paragraphs set to zero this is the
  current paginator with the page class handled correctly.
- **Joint.** The exact solver with a positive tolerance.
- **Pruned and evolved solvers.** Any policy over `candidates`.

`CssObjective` holds the constants of the current paginator: the cube of the
unused height, `AVOID_BREAK_COST`, `ORPHANS_WIDOWS_COST`, and nothing for the
last page.

The joint solver adds line demerits and page costs, which needs an exchange
rate between the two. There is no principled value for it. It is a field of
the objective and something to tune against the metrics.

Rough size of the exact search for a book of 10^4 lines: with cuts per
(position, state) instead of per line there are perhaps 5·10^4 page starts,
each with a few hundred candidates. That is seconds, not milliseconds. The
numbers are estimates and have not been measured. Pruning the search is where
heuristics, evolved or not, have real work to do.

## Metrics

```rust
pub struct Metrics {
    pub pages: usize,
    pub avoid_violations: usize,
    pub orphan_violations: usize,
    pub widow_violations: usize,
    pub overflows: usize,
    pub hyphens_at_page_end: usize,
    pub slack: Vec<f32>,            // unused height of every page but the last
    pub line_demerits: f64,
    pub loosened_paragraphs: usize, // paragraphs not at their optimal line count
}

pub fn metrics(pages: &[PageFill]) -> Metrics;
```

The metrics are a vector. How it is turned into a score is decided by the
harness, not by the solver. A solver that could define the objective it is
judged by would make any comparison meaningless.

What makes a pagination good is the open problem of this design, more than
how to search for it. The metrics above are what can be counted; they are not
a definition of quality.

## The layout service

```rust
pub trait LayoutService {
    /// The width of the lines of a paragraph of the flow on a page.
    fn line_width(&mut self, par: ParId, page_inline: f32) -> f32;

    /// The extent of a `Box` or `Gap` on a page.
    fn extent(&mut self, node: NodeRef, page_inline: f32) -> f32;

    /// The ways to end a part of a container within `available`.
    fn fragment(&mut self, node: NodeRef, from: BreakToken, page_inline: f32,
                available: f32) -> Vec<Fragment>;

    /// Lays out one chosen page and writes the positions of its boxes.
    fn layout_page(&mut self, start: BreakToken, end: BreakToken, spec: PageSpec,
                   lines: &LineChoices) -> PageLayout;
}

pub struct Fragment { pub end: BreakToken, pub used: f32 }
pub struct PageLayout { pub used: f32, pub overflow: bool }
```

The first three are cheap and cached by (node, inline size). The kernel builds
`candidates` on them. For plain block flow they are lookups and sums. There
are no width classes: the cache is keyed by the width that is asked for.

`layout_page` runs once per chosen page and produces all final positions.
Positions that depend on the height of the page or of a split box (absolutely
positioned boxes with `bottom`, percentage heights, `vh`) are right per page
without a later fix-up.

If `layout_page` reports an overflow, an estimate was wrong. The kernel
records the true extent for that range and the solver solves again from that
page on. For plain block flow this must never happen, and a test asserts it.

### How Taffy serves a part of the document

Taffy has no notion of fragmentation and no way to stop early. The service
answers through a *window* on `HeliumTree`:

- `child_ids` yields only the children inside the window;
- the style of a box that is cut has no padding and no border at the cut edge,
  and no margin there;
- the measure function of a paragraph of the flow returns the extent of the
  lines that the paginator dictates (`LineChoices`), and does not run the line
  breaker;
- a container whose tracks are sized by its content gets its tracks pinned
  (see below).

All semantics of fragments are in this view. Taffy is used unchanged.

Subtrees that are not cut are served from Taffy's cache of each node. The
nodes on the edge of a window must not share cache entries with the same nodes
laid out whole.

### Two regimes

- **The flow**: the chain of block containers from the root in which breaks
  can occur. Taffy provides inline positions and sizes, the width of each
  paragraph, and the fixed vertical pieces. The heights of the containers of
  the flow follow from what the paginator puts on each page.
- **Monolithic subtrees**: replaced boxes, `inline-block` boxes, orthogonal
  flows, and, until they can be split, flex and grid containers and table
  rows. Taffy lays them out whole. Their paragraphs are broken as today. Each
  is one `Box`.

A block container of the flow with an explicit `height` is treated as
monolithic at first.

### Containers that are split

A table or a grid that is laid out for the rows of one page alone would size
its columns from those rows, and the columns would jump from page to page. A
container whose inline tracks depend on its content is therefore sized in two
steps: once whole, at the inline size of the page, to find the tracks; then
per part, with those tracks pinned in the style that the window hands to
Taffy.

Tables are grids already (`taffy_layout/table.rs`), with rows that are no
Taffy nodes. A table that is split between rows is the simplest case of
`fragment`: no parallel parts, pinned columns.

A break inside a grid row or a flex line cuts all its cells. Each cell is a
flow of its own with its own break token; the token of the row is
`InContainer` with one part per cell. The cut of each cell is a function of
the height that is available: every cell is filled as far as it fits. The
cells are not optimised jointly, and their paragraphs are rigid. Doing more
multiplies the state of the search for little gain.

## Recorded problems and the harness

For solver work without the engine, a problem is recorded:

```
helium --dump-pagination problem.json input.html out.pdf
```

The file holds the `Problem` and what a `RecordedService` needs to answer the
kernel: the extents of boxes and gaps for each inline size that the page table
contains, and for every paragraph the input of the line breaker (advances,
glue, penalties, hyphens, forced breaks, line height) and its line width per
inline size. `serde` is a dev-dependency today and becomes an optional
dependency behind a feature.

Recorded problems cover the flow only. Containers that are split need the real
service.

A binary in `helium_paginate` runs named solvers over a directory of problems
and prints the metrics and the time of each. The corpus starts with the
gallery (`gallery/book`, `gallery/report`, ...) and the documents in
`tests/documents`.

Tests:

- **Equivalence.** With one page geometry and zero tolerance, the exact solver
  with `CssObjective` chooses the breaks of the current paginator on the
  existing tests and the gallery. This is the gate for removing the old code.
- **Validity.** `realize` accepts the solution of every solver on every
  recorded problem.
- **No surprises.** `layout_page` reports no overflow on documents that are
  plain block flow.
- **Bounds.** No solver is worse than greedy and none is better than exact
  under the same objective.

For a search such as OpenEvolve, the unit that is evolved is one Rust file
with a `Solver`. The evaluator builds `helium_paginate`, runs the harness and
scores the metrics. The exact solver on small problems gives the reference.

## Changes to the engine

The pipeline becomes:

1. Construct, find line break opportunities (unchanged).
2. Build `HeliumTree` once. Extract the `Problem` from it. This replaces the
   first half of `Paginator::run`.
3. Solve.
4. Apply: `layout_page` for every page, then split the decorations of boxes
   that span pages and place the absolutely positioned boxes. This replaces
   `Writer::write_box` over the whole document and the second half of
   `Paginator::run`. `Writer::write_lines` takes the lines to write instead
   of calling `break_lines`.
5. The passes after pagination are unchanged.

Removed once the new path passes the equivalence test:

- `taffy_layout/page_groups.rs` and `PageGroupBox`;
- `PageMargins::shift`, `edges_of_page_start` and `left_right_conflicts`;
- the single-galley call of `taffy::compute_root_layout`.

Changed for a page size per page:

- `PagesCreate` and the PDF writer take the size of each page;
- margin boxes and page backgrounds use the geometry of their page;
- `set_viewport` is set per page before `layout_page`. Viewport units in
  content that is measured before its page is known use the page that is
  asked for.

Islands (`foreignObject`, `layout_islands`) keep one page and use the greedy
solver.

## Order of work

1. `helium_paginate` with tokens, the flat flow, the kernel and `CssObjective`;
   `paragraph.rs` moves there. The service is backed by the existing galley.
   The exact solver passes the equivalence test. No change in output.
2. The windowed view and `layout_page` for block flow. Pages of differing
   width work. Page groups and the shift are removed.
3. Greedy solver, recorded problems, the harness and the metrics.
4. Page size per page; empty pages for `left` and `right`.
5. Profiles and cuts with state in the line breaker; the joint solver.
6. `fragment` for tables between rows, with pinned columns.
7. `fragment` for grid and flex, with parallel parts.

Steps 1 to 3 are the base for iterating on solvers. Steps 5 to 7 are
independent of each other.

## Open questions

- **Taffy's cache.** How many entries Taffy 0.15 keeps per node, and whether
  that is enough for a tree that is asked for several widths and windows, has
  not been checked. The service may need a cache of its own.
- **Windows and margins.** Margins are collapsed by Taffy's block layout.
  Whether a windowed list of children gives the margins that fragmentation
  wants at the edges of a page has not been checked.
- **Widths that depend on heights.** The design relies on the inline sizes of
  the flow not depending on the heights of its content. That holds for plain
  blocks. `aspect-ratio` on a container of the flow would break it.
- **Collapsed borders.** A table that is split has borders at the cut that
  `taffy_layout/collapse.rs` resolved for the whole table. The decoration
  fragments handle this today; the window has to as well.
- **The exchange rate** between line demerits and page costs.
- **Runtime of the joint solver** on a book. Unmeasured.
- **Floats and footnotes** take space on a page out of the order of the flow.
  They would be a new kind of item, and `candidates` would no longer be a walk
  in one direction.
