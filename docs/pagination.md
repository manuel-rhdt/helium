# Pagination: design

Status: steps 1, 2 and 4 of "Order of work" are implemented. The section "Where the code stands"
describes the code as it is; the sections after it are the design, with notes where the code
differs.

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
- Multi-column layout is not part of steps 1 to 7. It has a section of its own, "Multi-column
  layout", and its own order of work.
- Forking Taffy to make it fragmentation-aware.

## Where the code stands

The pipeline in `src/layout/mod.rs` (`layout`) is:

1. `FindPossibleLineBreaks` finds the break opportunities of every paragraph.
2. `TaffyBlockLayout` (`src/layout/taffy_layout/mod.rs`) builds `HeliumTree` once and does the
   rest of the work on it:
   - `flow.rs` makes the *flow* from the tree: the items of the kernel. A block container is
     the box of its top edge, its kids with a break and a gap between each two of them, and the
     box of its bottom edge. A paragraph is its two edges and a paragraph item. Boxes that are
     not split (replaced boxes, flex and grid containers, boxes with a `height`, orthogonal
     flows) are one box. A table is split between its rows, row groups and captions, where no
     cell spans over the break. Every box knows its items (`Span`).
   - `service.rs` is the layout service. For every page width that the kernel asks about it
     lays the whole tree out once with Taffy (a galley) and reads the extents of boxes and gaps
     from it. The lines of a paragraph are what the line breaker makes of it for the width that
     the galley gave it; from a cut on, they are the lines of the whole paragraph if the cut is
     the end of a line of it, and else the rest of the paragraph broken for the width of the
     page. Everything is cached by the width that was asked for.
   - `paginate.rs` gives the kernel the geometry of the page that starts at every place: the
     name of the first content on it, whether it starts a page group, and its kind decide the
     margins and the size. The exact solver with `CssObjective` chooses the pages.
   - For every chosen page `window.rs` shows Taffy a *window* on the tree and the page is laid
     out for its own width and height. `Writer` writes the positions of the boxes that are
     on the page, in the coordinates of the page. A box on more than one page has a position
     for each of them (`DecorationFragment`).
3. The passes after it (`PlaceInlineBlocks`, `LineBoxCreator`, ...) are unchanged. `PagesCreate`
   and the margin boxes take the size of every page from its `PageContext`.

Limits of this, in order of importance:

- **Paragraphs are rigid.** There are no profiles and no joint optimisation (step 5). A paragraph
  that continues on a page of another width is broken again from the cut on, without the state of
  the line breaker.
- **Monolithic containers.** Flex and grid containers and table rows are never split. A table is
  split between rows, but it is laid out as a whole, for the width of the page where it starts:
  the columns of a table do not follow the width of later pages. A box with a `height` is not
  split either (a minimum height is dropped for a box that is cut).
- **Out-of-flow boxes.** An absolutely positioned box is laid out on the page of the content in
  front of it, in the content area of that page. If the box is positioned by `top` or `bottom`
  in the initial containing block, it is on the first page. A containing block that is cut
  counts as the box on the page of the box.
- **The height of the galley.** The galley is laid out for the height of the first page. Heights
  that are percentages or viewport units in boxes that are not split follow that page, and the
  viewport units follow the plain page.
- **`--crop`** has page size 0 and puts every box on a page of its own, as before.
- **Fragments reach the bottom of the page.** The part of a box that goes on on the next
  page is painted down to the bottom of the content area of its page, also where the content
  on the page ends above it. The layout of the page ends with its content; only the
  decoration is stretched.

Two properties of the code that the design builds on:

- helium implements Taffy's tree traits itself (`HeliumTree` in `taffy_layout/tree.rs`)
  instead of using `TaffyTree`. It controls which children and which styles Taffy sees.
- `paragraph::compute_paragraph_breakpoints` takes the line width as a function of the line
  number, and `InlineFormattingContext::compute_lines_from` breaks a paragraph from a break
  opportunity on.

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

The pipeline is now:

1. Construct, find line break opportunities (unchanged).
2. Build `HeliumTree` once. Make the flow from it.
3. Solve, with the layout service answering the kernel.
4. Apply: lay out every page through its window and write the positions. The decorations of
   boxes on more than one page are made from the parts that were written for each page. There
   is no `layout_page` function: setting the window, calling `compute_root_layout` and `Writer`
   are the steps of the loop in `TaffyBlockLayout::run`.
5. The passes after pagination are unchanged.

Removed: `taffy_layout/page_groups.rs` and `PageGroupBox`; `PageMargins::shift`,
`edges_of_page_start` and `left_right_conflicts` with its warning; `Problem::stop_at_forced`;
`Paginator` and its galley; the single-galley call of `taffy::compute_root_layout` (the galleys
that the service lays out only give extents).

Changed for a page size per page:

- `PagesCreate` takes the size of each page (and creates the empty pages);
- margin boxes and page backgrounds use the geometry of their page;
- `set_viewport` is still set once, for the plain page: viewport units are resolved when the
  style is computed.

Where the code differs from the design above:

- The kernel asks `LayoutService::lines(par, from, inline)` for the lines of a paragraph, and
  a cut is an opaque `Cut`: the index of a break opportunity. A paragraph item has no line
  count. `Position::InParagraph` holds the cut. The tokens of cuts are made when a page that
  ends there is first asked for.
- `PageTable` is a function from the position and the kind of a page to its geometry.
- `PageStart` has the page number. A `Solution` is a list of `PageStart`s; a number that is
  missing is an empty page that `next_page` made for a break to a left or right page.
- `fragment` and `PageLayout` do not exist: tables are split between rows without them.

Islands (`foreignObject`, `layout_islands`) keep one page; they use the exact solver and show
the first page.

## Order of work

1. Done. `helium_paginate` with tokens, the flat flow, the kernel and `CssObjective`;
   `paragraph.rs` moves there. The exact solver passes the equivalence test.
2. Done. The windowed view and the layout of a page for block flow. Pages of differing
   width work. Page groups, the shift, the guessed page class and `stop_at_forced` are removed.
   Tables are split between rows, with the columns of the table as a whole.
3. Greedy solver, recorded problems, the harness and the metrics.
4. Done. Page size per page; empty pages for `left` and `right`.
5. Profiles and cuts with state in the line breaker; the joint solver.
6. `fragment` for tables between rows, with pinned columns.
7. `fragment` for grid and flex, with parallel parts.

Steps 1 to 3 are the base for iterating on solvers. Steps 5 to 7 are
independent of each other.

## Multi-column layout

Status: proposal. helium has no `column-count`, `column-width`, `column-fill`, `column-span` or
`column-rule`, and `break-before: column` does not parse. (`column-gap` exists, but only for grid.)

A column is a fragmentainer like a page: a box with an inline size and a block size that content
is cut into. The kernel, the solvers, the break tokens and the re-breaking of paragraphs carry
over. What does not carry over is that a multi-column container sits in the middle of the flow of
a page. It is a fragmentation context inside another one, and that is what this section is about.

### What carries over

- A column is a `PageSpec`. Its inline size is `(width of the container - gaps) / N`, which does
  not depend on heights, so extents keyed by inline size work as they are.
- Orphans, widows, `avoid` breaks, and the re-breaking of a paragraph at the width of the column
  it continues in, work per column as they do per page.
- A box that is split over columns gets its parts as decoration fragments, as over pages. A
  fragment has an x position, so only its page index needs a column as well.

### What does not carry over

**Nested kernels.** A multi-column container is an `Item::Container`. Its extent on a page
is found by an inner kernel run: a `Problem` whose page table has N columns of the width of a
column. This is the `fragment` call of the layout service that steps 6 and 7 add for tables, grid
and flex. Multi-column layout is a second user of it, not an independent feature.

**Balancing is a min-max problem.** With `column-fill: balance` (the default) a container with an
`auto` height is as short as it can be while its content still fits in N columns. The exact
solver adds up the costs of the pages; balancing needs the maximum of the heights of the columns,
and then a tie-break (the variance of the heights). `Objective` therefore gets a combine
operator, sum or maximum, and the exact solver uses it for its path costs. Balancing applies to
the last fragment of a container that is split over pages only. The earlier fragments fill all
their columns to the height that is available, as `column-fill: auto` does.

**The service must answer for a subtree at a width.** `LayoutService::extent` reads from a
galley: the whole document laid out at one page width. The content of a multi-column container
needs its extent at the width of a column, which is not the width of any of its ancestors. The
service has to lay out a subtree with a given available width. The windowed tree can do that;
the service does not use it for this yet. The same limit applies today to a table that continues
on a page of another width.

**Break levels.** `BreakKind` tells forced from avoid, not the level. A forced column break ends
a column. A forced page break in a multi-column container ends the fragment of the container on
that page and goes on to the outer flow. A break is therefore (kind, level), with the levels
`Column` and `Page`, and `break-before`, `break-after` and `break-inside` take the values
`column` and `avoid-column`.

**A layout mode in `HeliumTree`.** Taffy has no multi-column layout. The container is laid out by
helium, as a table is turned into a grid: for every column, the window is set to the part of the
children that is in the column, they are laid out at the width of the column through the windowed
path, and the result is moved by the x offset of the column. The same box is then laid out in
several columns in one pass. That relies on nodes that a window changes not using Taffy's cache,
which is true today (`Node::windowed`), and on positions that are stored per entity: the lines
are entities of their own, the boxes that are split get decoration fragments.

**`column-span: all`.** A spanning box splits the container into sections. Each section is
balanced for itself, and the spanning box is a `Box` in the flow around them.

### The inner problem

```rust
pub struct Columns {
    pub count: usize,
    pub width: f32,      // the width of a column
    pub gap: f32,
}
```

For the fragment of a container that starts at `from` with `available` block size:

- the columns are filled in order, each up to `available`; the fragment ends at the end of the
  last column, or where the content ends;
- if the content ends in this fragment and `column-fill` is `balance`, the container gets the
  least height `h <= available` such that the content fits in N columns of height `h`. The inner
  solver finds it as a path of N columns with the maximum of their heights as the cost. With
  neither `avoid` nor orphans and widows to respect, a greedy fill gives the answer, and it can
  be searched for by bisection on `h`. With them it cannot, which is why the exact solver is
  used;
- the candidates that the inner kernel returns are the ends of complete sets of columns, plus the
  end of the content. The outer kernel treats them like the ends of any other `fragment`.

For a container that is not split (its content fits on the page) this reduces to `extent`: the
balanced height of the container, computed once for a width.

### Order of work

1. The service answers the extent and the lines of a subtree at a given available width, instead
   of reading them from a galley. This is also what splitting tables on pages of another width
   needs.
2. `column-count`, `column-width`, `column-gap` and `column-rule` for a container that is not
   split over pages and whose content is block flow. Balanced. The combine operator of
   `Objective`. Break levels, and `break-before: column`.
3. A container that is split over pages: `fragment` for it, `column-fill: auto`, and balancing of
   the last fragment only.
4. `column-span: all`.

Step 1 does not need anything of the later ones, and the steps of "Order of work" above (3, 5, 6
and 7) are independent of it, except that step 3 here shares `fragment` with step 6 there.

### Open questions

- Whether a greedy fill is good enough for balancing with `avoid` breaks, or whether the exact
  solver is needed on every container. Unmeasured.
- Whether the lines that are in a window of several columns of one pass keep correct positions:
  `Writer::write_lines` sets positions per entity, and a paragraph that is cut between two
  columns has lines of one entity in both. This is how it works over pages, one pass per page.
- Whether the column rules (`column-rule`) belong to the container or to its fragments, which
  matters where the container is split over pages.
- Orthogonal writing modes. A column is a block-axis fragmentainer, and the inline axis of a
  vertical document is the vertical one. Nothing here has been checked for them.

## Open questions

- **Taffy's cache.** Resolved by reading `taffy-0.15.1/src/tree/cache.rs` and the layout
  functions. A node has one entry for its final layout and nine for measurements. The key is
  the known size, the available space and the size of the parent; the mode (sizing mode and
  whether the margins are collapsible) is kept once for the node. Neither the style nor the
  children are in the key. A node that is laid out in a window (other children, other style)
  must therefore not use the cache: `HeliumTree` returns no entry and stores none for the
  nodes that the window changes (`Node::windowed`). The nodes inside the window that are not
  changed keep sharing their entries, and since a parent that is laid out for another width
  replaces the final entry of its child, the layout of a node's descendants is always the one
  that its last final entry was computed with. For a document with several page widths this
  means that a node is laid out again whenever the width changes, which is what the galleys do.
- **Windows and margins.** Resolved by testing. A margin that adjoins the edge of a page is
  discarded by setting it to zero on the boxes along the first and last kid of the window, down
  to the first box that has padding or border at that edge (a margin below it does not adjoin
  the page). After a forced break the margin of the first box is the gap of the flow, which is
  the collapsed margin of all the boxes at the break, and its descendants get none. Taffy
  collapses the margins that are left as it does in a whole document. The test
  `the_height_of_a_page_is_what_the_paginator_counted_in_every_document` checks that the height of
  every page of the documents in `tests/documents` and `gallery` is what the kernel counted.
- **Widths that depend on heights.** Unchanged. Boxes with an `aspect-ratio` or a `height` are
  not split, so the widths of the boxes in the flow do not depend on heights.
- **Collapsed borders.** A table is laid out as a whole, so the borders are the ones that
  `collapse.rs` resolved for the whole table. The fragments at a cut get the edges they had
  before (`paint` off at the cut and no extension of the side borders), as before.
- **The exchange rate** between line demerits and page costs.
- **Runtime of the joint solver** on a book. Unmeasured. The exact solver (no joint
  optimisation) paginates `gallery/book` (368 pages) in about 230 ms including the galley
  that gives its extents; the old paginator needed about 195 ms for the galley and the
  paginator together (release build).
- **Floats and footnotes** take space on a page out of the order of the flow. They would be a
  new kind of item, and `candidates` would no longer be a walk in one direction.
- **A table that spans pages of different widths** keeps the columns of the width of the page it
  starts on.
