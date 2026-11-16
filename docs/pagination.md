# Pagination: design

Status: steps 1, 2 and 4 of "Order of work" are implemented, and so are steps 1 and 2 of the
order of work of "Multi-column layout" (the service answers for a subtree at a width, and a
container with columns that is not split over pages has balanced columns). The section "Where the
code stands" describes the code as it is; the sections after it are the design, with notes where
the code differs.

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
     flows, containers with columns) are one box. A table is split between its rows, row groups
     and captions, where no cell spans over the break. Every box knows its items (`Span`). The
     content of a container with columns is a flow of its own, with its own numbers of items,
     of the break level `Column` (see "Multi-column layout").
   - `service.rs` is the layout service. For every page width that the kernel asks about it
     lays the whole tree out once with Taffy (a galley) and reads the extents of boxes and gaps
     from it. The lines of a paragraph are what the line breaker makes of it for the width that
     the galley gave it; from a cut on, they are the lines of the whole paragraph if the cut is
     the end of a line of it, and else the rest of the paragraph broken for the width of the
     page. Everything is cached by the width that was asked for. The service also answers for a
     box laid out *at a width of its own*, the content width of that box and not of a page (a
     *subtree*, `subtree.rs`; see "A subtree at a width").
   - `paginate.rs` gives the kernel the geometry of the page that starts at every place: the
     name of the first content on it, whether it starts a page group, and its kind decide the
     margins and the size. The exact solver with `CssObjective` chooses the pages.
   - `columns.rs` chooses and lays out the columns of a container with columns (see "Multi-column
     layout"). The container is a leaf for Taffy, so a page layout asks it for a height like any
     other box.
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
  split between rows, but it is laid out as a whole, again for the width of every page that it
  is on (the window shows Taffy all its cells, and the galley of each width lays out all of
  it). The rows of a page are therefore laid out for that page and the paginator counts them
  so, but the columns of a table follow the width of each page instead of staying the ones of
  the page where it starts. Pinning the columns is step 6 of "Order of work" (the test
  `the_columns_of_a_table_stay_the_ones_of_the_page_where_it_starts` is ignored until then).
  A box with a `height` is not split either (a minimum height is dropped for a box that is
  cut).
- **Multi-column containers.** A container with columns is one box of the flow of its page, so it
  is not split over pages: one that does not fit in the rest of a page moves to the next, and one
  that is taller than a page has a page of its own and overflows it. A forced page break inside of
  it ends nothing. The `inline-block` boxes in its lines are sized for the first width that was
  laid out (like all of them), a table in it is one box, an absolutely positioned box that it is the
  containing block of is not placed, and a container in a vertical flow is a block.
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
    /// A place where a page may end. A break is between pages or between columns.
    Break { kind: BreakKind, level: BreakLevel },
    /// A paragraph of the flow.
    Paragraph { id: ParId, orphans: u8, widows: u8, avoid_inside: bool },
    /// A container that can be split between or inside its parts.
    Container { node: NodeRef },
}

pub enum BreakKind { Allowed, Avoid, Forced, ForcedLeft, ForcedRight }
pub enum BreakLevel { Page, Column }
```

A `Problem` paginates one level (`Problem::level`), and a break of the other level is no break in
it: it is of the kind `Allowed` there. A forced column break outside of a container with columns
is therefore like `auto`, and a forced page break in a container with columns that is not split
ends nothing. A `Problem` can also limit its number of pages (`max_pages`): the last page that
there can be takes the rest of the content, whatever breaks it has, and overflows if it cannot
hold it. The columns of a container are such a problem.

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
    /// The cost of the pages so far and one more. The sum by default.
    fn combine(&self, so_far: f64, page: f64, index: usize) -> f64;
}
```

The exact solver minimises what `combine` makes of the costs of the pages: the sum for pages, the
maximum for balanced columns (`BalanceObjective`). If the number of pages is limited, the number
of the page is part of the state of the search.

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

    /// The extent of the content of a box, and what is inside it, at a width of its own.
    fn content_extent(&mut self, root: BoxRef, inline: f32) -> f64;
    fn extent_in(&mut self, root: BoxRef, node: NodeRef, inline: f32) -> f64;
    fn lines_in(&mut self, root: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> Rc<[ParLine]>;

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

`content_extent`, `extent_in` and `lines_in` are the same questions for a subtree. They are
cached by (box, inline size) as well.

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

### A subtree at a width

`LayoutService::content_extent(root, inline)` and its companions `extent_in` and `lines_in` answer
for the box `root` laid out on its own. `BoxRef` names a box of the flow (in the engine, the index
of its node in `HeliumTree`).

- **What `inline` is.** The width of the *content box* of `root`. `root` is laid out as the root
  of a formatting context: its own `width`, `min-width` and `max-width` do not apply, its padding
  and border come around `inline` (so its border box is wider), and a percentage in its own
  padding refers to `inline`. Margins of `root` are not part of it, and margins of its children
  do not collapse through its edges. The content box is the width that a column has, and what the
  children of a box see, so a multi-column container asks for its content at the width of a
  column. Everything inside is laid out by the usual rules for that content width: percentages,
  `inline-block` boxes, nested boxes, tables. The height that is available is the one of the
  galleys (the height of the first page).
- **What it returns.** `content_extent` is the block size of the content box of `root`: its
  height without padding and border (for a table, from its first row to its last). `extent_in`
  is the extent of a `Box` or a `Gap` that is inside `root`, as `extent` has it on a page: the
  boxes and gaps of the flow between the start and the end of `root` have one. (A gap or an
  edge that is made from boxes around `root` has none, and asking for it is an error.) `lines_in`
  gives the lines of a paragraph inside `root` for the line width and the indent that its box
  has at that width, from a cut on, as `lines` does for a page.
- **Cache of the service.** By (box, `inline`), and for the lines by (box, paragraph, cut,
  `inline`). The same arguments give the same answers.
- **Cache of Taffy.** An entry is keyed by the known size, the available space and the size of the
  parent, and the cache of a node only knows the layout of that node: the layouts of its
  children live in the children. A layout at another width therefore leaves two kinds of
  damage, and both are handled explicitly (`subtree.rs`):
  1. *Below the box.* The caches of all nodes of the subtree are cleared in front of the layout
     (and the width that each paragraph was measured at is forgotten), so everything is
     computed from nothing at this width. Afterwards the entries of these nodes agree with the
     layouts below them, as after any layout.
  2. *Around the box.* The caches of its ancestors still say that their children are as they
     were in the last galley, while the layouts below `root` are now the ones of this width.
     A later layout of the page that gets such an entry would return the old size and keep the
     new positions. The caches of the ancestors are cleared behind the layout; their other
     children keep their entries.

  A test shows that the second kind matters: with that step left out, a page layout after a
  question differs (`the_boxes_around_a_subtree_are_laid_out_again_behind_a_question_about_it`).
  No test shows that the clearing in front is needed (the entries of the nodes below `root` are
  consistent with each other and would be found only for the same inputs); it is kept because
  the paragraphs' `measured` widths and the next point rely on every paragraph being laid out
  anew.
- **`inline-block` boxes.** The boxes of a paragraph are laid out once, for the line width of
  the first layout of the paragraph, and their widths are *added* to the inline context
  (`apply_atom_widths`). That state is not part of Taffy's cache. A question that came first
  would give every later layout the sizes of its width, and one that came later would use the
  sizes of an earlier width. Both are wrong for boxes with a percentage width or a width that
  fits their content. The inline context of a paragraph as it was before the widths were
  applied is therefore kept (`Node::atoms_before`); for the layout of a subtree the paragraph
  gets a copy of it, the boxes are sized for this width, and the lines of `lines_in` are made
  from that copy. Behind the layout the inline context, the sizes of the boxes and the lines
  that were cached for the paragraph are the ones from before. (The galleys of the pages still
  size the boxes once, for the first width; that limit is unchanged.)
- **What is not covered.** The root must be part of the flow and not an orthogonal flow.
  Out-of-flow boxes in the subtree whose containing block is outside it are not placed; no layout
  of the subtree uses them. A window is taken off the tree first, so a subtree is always a whole
  box. The galley path (`extent` and `lines` for a page) is not written with this call: the
  root of a galley is the root of the document, laid out by `compute_root_layout` with its own
  margins and the initial containing block, and the galley is not slower or different for it.
  Only the paragraph part of `lines` is shared.

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

Status: steps 1 and 2 of "Order of work" below are implemented: a container with `column-count` or
`column-width` that is not split over pages, whose content is block flow or one paragraph, has
balanced columns, rules and breaks between columns. Splitting a container over pages (step 3) and
`column-span: all` (step 4) are not done. **A container that is taller than a page loses its
content today** (see "The common case is a container over pages").

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

**Nested kernels.** A container with columns is one box (`Item::Box`) in the flow of its page,
with an extent that is found by an inner kernel run: a `Problem` whose pages are the N columns,
as wide as a column. (The `fragment` call that steps 6 and 7 add for tables, grid and flex is the
same thing for a container that is split; step 3 here needs it.)

**Balancing is a min-max problem.** With `column-fill: balance` (the default) a container with an
`auto` height is as short as it can be while its content still fits in N columns. The exact solver
adds up the costs of the pages by default. `Objective` has a combine operator for this
(`combine(so_far, page, index)`, with the sum as the default, so `CssObjective` and everything
else is as before). Balancing takes the maximum, in two exact steps (`balance`, `BalanceObjective`):

1. The maximum of the heights of the columns is minimised. The height of a column is its content
   plus a cost for every rule that it breaks: a break that is to be avoided, orphans and widows,
   and content that does not fit. Each of these costs more than any content (the costs are in the
   order that `CssObjective` has), so a column is only as tall as it takes to keep the rules, if
   that is possible.
2. Among the paginations in which no column is taller than the least maximum, the sum of the
   heights of the columns, each weighted by its number plus one, is minimised. With the same
   tallest column the first columns are therefore the fullest, as in browsers: five boxes in two
   columns are three and two, not two and three. It is a sum, so the exact solver finds it by the
   prefix of a pagination; the maximum of the first step is too, since the maximum of the first
   columns and the next one is the maximum of both. A tie that is left is decided by the exact
   solver (the earlier start of the last column wins), so the result is the same every time.

The variance of the heights was not used: it is not a sum of the costs of the columns, and with
equal tallest columns it would prefer the last columns to be as full as the first, where browsers
fill the first ones.

A problem that has a limit of N pages needs the number of the page as part of the state of the
exact solver (the pages that are left depend on it), so the states are (place, kind, number), and
the number is 0 if there is no limit. The last column takes the rest of the content, whatever
breaks it has.

**The service answers for a subtree at a width.** `LayoutService::extent` reads from a galley: the
whole document laid out at one page width. The content of a container needs its extent at the
width of a column, which is not the width of any of its ancestors. `content_extent`, `extent_in`
and `lines_in` do that (see "A subtree at a width"): the box is laid out alone with a definite
content width, on the tree without a window, and the cache of Taffy is protected around it. The
inner problem asks them through the container as the root, which is laid out as a plain block (its
own columns do not apply in these calls).

**Break levels.** `BreakKind` tells forced from avoid, and `BreakLevel` (`Page` or `Column`) says
what is broken. `break-before`, `break-after` and `break-inside` take `column` (a forced break
between columns) and `avoid-column`; `avoid-page` and `avoid-column` avoid breaks of their own
level only, and `avoid` avoids both. The flow of a page has the level `Page` and the flow of the
content of a container has the level `Column`. A break item has the level of the value that
decides it (a forced value of the level of the flow, else an avoid value of it, else a forced or
avoid value of the other level, which is no break there, else none). Only a break of the level of
the problem is a break in it. A forced break of level `Page` in a container with columns that is not
split ends nothing, and it will end the fragment of the container on that page in step 3. A
`break-inside` of a box (the container itself too) is an `avoid` for the breaks inside it of its
level; the `break-inside` of the boxes around a container does not reach the columns, which are
another fragmentation context. The breaks do not propagate out of a container: the `break-before`
of its first child is a break between columns of it, not before the container.

**A layout mode in `HeliumTree`.** Taffy has no multi-column layout. A container with columns is a
leaf for Taffy, like a paragraph: `compute_cached_layout` calls `compute_columns_layout`, which
asks the plan for a height and returns the size that `compute_leaf_layout` makes of it, with the
width, padding, border, `min-` and `max-` sizes and `height` of the box as for any block. The
columns are then laid out by helium, as a table is turned into a grid: for every column of the plan
the window is set to the part of the content that is in the column (the box of the container
is a window root: `Window::scope`), the container is laid out as a block of the width of the
column with no padding, border and margin, and the layouts of the boxes that Taffy has just laid
out in the window (`Snap`: layout, measured line width, shift of a cell) are copied for the
column. The `Writer` then writes the boxes of a column with the layouts of that column (an
overlay on the layouts of the nodes) and with its x offset (`column index * (width + gap)`,
mirrored for `direction: rtl`). The lines of a paragraph that is cut between two columns are
entities of their own, as over pages; the `Position` of a box that is cut is the one of its first
part and the other parts are fragments (see below).

The nodes of a container with columns do not use the cache of Taffy, because the layout of the
columns is a side effect that a cached layout would skip. The boxes around a container are
laid out again whenever it is, and a box with a cached layout has the columns of the layout that
the cache is of: the columns are laid out whenever the container is asked for its final layout,
not only in the page layouts, so the two always agree. The layouts that the plan asks for the
content at the width of a column take the window off the tree and put it back (`save_window`,
`restore_window`), because they happen in the middle of the layout of the page.

**`column-span: all`.** A spanning box splits the container into sections. Each section is
balanced for itself, and the spanning box is a `Box` in the flow around them. Not done: the
property is parsed and ignored.

### The inner problem

For a container whose content box is `W` wide, `count` columns of the width `(W - (count - 1) *
gap) / count` are made like this (`column_geometry`):

- `column-count` only: `count` columns. `column-width` only: `count = max(1, floor((W + gap) /
  (width + gap)))`. Both: the least of the two.
- `column-gap: normal` is `1em` of the container (a grid and flex container have 0), a percentage
  is of `W`.

The content is the flow of the container, extracted once after the tree is built
(`Extractor::emit_columns`): the children of a block container, or one paragraph item if the
container has inline content (the container is the paragraph; its own padding and border are
outside of the columns). The flow has no top and bottom edge boxes. The margin of the first child
is in the flow (a gap that a first column keeps), and the one of the last is not (the trailing gap
of a column is discarded). Page names do not change columns. Tables in it are one box.

- **Auto height.** The container is as high as its tallest column, which is what balancing finds.
  The problem has the columns as its pages with a block size that is only a bound for the search,
  so that the walk over the content for the next break stops: `1.15 * content / count` at first,
  where the content is the extent of the whole content at the width of a column. If the result
  breaks a rule, or the content did not fit, and the bound is lower than the content, the bound is
  raised (by 1.6) and the balancing is done again. A result that keeps all the rules within the bound
  is the least one: a pagination that is better has columns that are not taller than it, and the
  solver saw all of those.
- **Definite height.** The block size of the columns is the content height of the container. The
  columns are balanced within it as far as it allows, and the container keeps its height. If the
  content does not fit in N columns of that height, every column but the last is as full as it
  can be (the balancing finds the one with the least tallest column, which is the first columns
  that are full), and the last column takes the rest and overflows below the container.
- **`column-fill: auto`** is parsed and treated as `balance`. Filling the columns in order, in a
  container with a height, is the cost of `CssObjective` over the same problem, which step 3 needs.
- **`min-height` and `max-height`** do not change the columns: the container is that high, and the
  columns are as high as the content needs.

For a container that is not split (its content fits on the page) this reduces to `extent`: the
balanced height of the container, computed once for a width and a height. The plans are cached
for the content width and the definite height; a document is laid out for as many widths as it
has page widths.

The columns that have content are the ones of the plan. A rule is painted between two columns of
the plan, so a gap next to an empty column has none.

### Fragments, rules and painting

A box that is cut between columns is written as a box that is cut between pages (`open_start` and
`open_end` of the window of the column): the first part sets the `Position`, and every part is a
`DecorationFragment` with its own position and the decoration of the cut (no border, padding or
rounded corners at the cut). The part that goes on in the next column reaches the bottom of the
columns, as the one on a page reaches the bottom of the page.

A rule is a filled rectangle of the width `column-rule-width`, centered in the gap, as tall as the
content box of the container, in the colour of `column-rule-color` (`currentcolor` is the text
colour). It is an entity of its own with a `Position` and a `Decoration` (`ColumnRule`), so it is
in the display list and in `--dump-layout` `paint.rects` like any other filled rectangle. Every
visible `column-rule-style` is painted as a solid line. The rules belong to the container; where
the container is split over pages (step 3) every fragment has the rules of its columns, because the
rules are made from the columns that are written.

The text of a paragraph that runs over two columns is one `Paragraph` of the display list with
a new line from one line to the next: the new line says where the next line is when it is not
directly below (at the top of the next column) and moves the text to it.

### The common case is a container over pages

The steps below were first ordered as if a balanced container inside one page were the common
case. That is the common case on the web. For a renderer of pages it is not: a two-column article
whose text runs over several pages, with the columns of every page filled in turn, is. Steps 1
and 2 are only the smaller part of what is used most.

What happens today to a container taller than a page: it is one box, balanced over all its
content. A test with a container of about 12 paragraphs in two columns on A5 pages put the text
of the second column at a y position of about 1,738px on a page that is 793px high. The content
below the page is not on any page, and nothing is logged. The only test of a taller container
(`tests/multicol.rs`, test 12) asserts that nothing crashes, which is why this was not noticed.
This is a defect, and it should be fixed before more features are added.

What is wanted, in the order in which it is cheap:

- **Columns filled in turn over pages** (`column-fill: auto`). The columns of a page are filled to
  the height that is left on it, and the fragment of the container ends after the last column.
  The kernel already does most of it. The fragmentainers are a sequence of columns, the block
  size is the height that is available on the page (smaller on the first page of a container
  that starts in the middle of one), and the page of a column and its x offset follow from its
  number. The outer kernel gets at most the complete sets of columns as places to end a
  fragment, plus the end of the content. This needs no balancing, and no choice that depends on
  where the content ends. It does need tokens that say where in a container a fragment starts,
  and a window on the children of the container for the part on one page.
- **The last fragment balanced.** By default (`column-fill: balance`) only the last fragment of
  a container that is split is balanced; the others are full. Print also often leaves the last
  page unbalanced (the first column full, the second with the rest), which is `column-fill: auto`.
  Both are wanted, and today `column-fill: auto` is parsed and balances.
- **A forced page break inside a container** ends the fragment on that page and continues on the
  next one. A forced column break ends a column, as now.
- **`column-span: all`**, for the title block of an article and for wide figures. A spanning box
  splits the container into sections. Each is filled or balanced for itself.

A typical article needs more than this and helium has none of it: floats for figures in a
column, footnotes, and spanning figures that float to the top of a page. They are non-goals of
this document. Text in columns would work after the steps above; an article as a whole would not.

An earlier version of this document argued against treating a container at the top of the flow
as a special case. For the first item above the objection is weaker than it was: a container
that fills the page area from its top has no outer content to decide about. It is acceptable to
build that case first, if it is built so that the general nested case (a container in the middle
of a page, with content after it that depends on its last fragment) can replace it without
changes to the solvers.

What the specifications say (CSS Multi-column Level 1, CSS Fragmentation Level 3, read on
2026-11-16). This replaces what was written from memory above where they differ.

- `column-fill: balance`, the initial value: "In fragmented contexts, only the last fragment is
  balanced." `balance-all` balances all fragments, and `auto` fills the columns in turn
  (multicol 7.1). `balance-all` is not parsed yet.
- Content that comes right in front of a spanner is balanced whatever `column-fill` says; the
  content after a spanner is balanced or not as `column-fill` says (multicol 7.1, from the
  description of the property and the names of its tests; the normative text was not read in
  full).
- A page break inside a container is always both a page break and a column break: the container
  gets a new row of columns on the next page (fragmentation 2.2). A forced break "forces
  ensuing content into the next fragmentainer of the type associated with the break, breaking
  through as many fragmentation contexts as necessary"; one that is not inside a context of its
  type has no effect (fragmentation 4.3). So a forced page break inside a container ends the
  fragment, and a forced column break outside one is ignored, which is what the break levels
  do. The first half is not done yet: step 2 ignores a forced page break inside a container.
- Spanners are honored in every fragment. The margins of two adjacent spanners collapse; the
  bottom margin of a spanner does not collapse with the top margin of what follows, because a
  column box is a new block formatting context (multicol 6.1).
- A container that has more columns than it has room for (a `height` that is too small, or
  forced column breaks) puts the extra columns on the next page in paged media (multicol 8.2).
  Step 2 lets such content overflow below the last column, which is the rule for continuous
  media only.
- The column count and the column width may be found per fragment in paged media (multicol 3.4).
  They can differ between pages of different widths.
- A fragmentainer of another size is laid out as if the whole element were fragmented across
  fragmentainers of that size; intrinsic sizes are kept across the whole element, and the
  initial containing block is that of the first fragmentainer (fragmentation 5.1). This is what
  the windows do.

Not checked against the text: whether column rules are painted in every fragment, and the
rules for `box-decoration-break` and margins at breaks (fragmentation 5.2 to 5.4). What
`column-fill: auto` means for a container with an `auto` height that is not fragmented is
also open: the specification's example has the content in the first column only when the height
is definite.

### Order of work

1. Done. The service answers the extent and the lines of a subtree at a given width of its
   content box, instead of reading them from a galley. It is what the columns are chosen from; it is
   also what pinning the columns of a table that is split needs.
2. Done. `column-count`, `column-width`, `column-gap` and `column-rule` for a container that is not
   split over pages and whose content is block flow or one paragraph. Balanced. The combine
   operator of `Objective`. Break levels, and `break-before: column`.
3. A container that is split over pages. This is the most used case and comes next.
   a. Columns filled in turn (`column-fill: auto`) over pages, with the first fragment as high as
      the rest of its page, forced page breaks inside the container, and no loss of content for
      a container that is taller than a page (fix this first).
   b. The last fragment balanced, and `column-fill` as the specification has it.
   c. `fragment` for a container in the middle of the flow, where the outer solver has to ask for
      the ways a fragment can end (shared with step 6 above).
4. `column-span: all`.

The tests of step 3 are in `tests/multicol.rs`, below the others, and are ignored with the reason
that steps 3a and 3b are not done.

Step 1 does not need anything of the later ones, and the steps of "Order of work" above (3, 5, 6
and 7) are independent of it, except that step 3 here shares `fragment` with step 6 there.

### Open questions

- **Whether a greedy fill is good enough for balancing with `avoid` breaks.** For the height, yes,
  as long as every rule can be kept. The places where a column can end are then a fixed set
  (the breaks that are not to be avoided, and the cuts of the paragraphs that leave orphans and
  widows), and filling every column as far as it goes, with a bisection on the height, finds the
  least height that holds the content in N columns (the usual argument: a column that ends
  later never leaves more for the others). It also gives the fullest first columns. This was not
  built and compared; the code uses the exact solver. What a greedy fill cannot do is choose
  when a rule cannot be kept: which of an `avoid` break, widows and a box that is taller than the
  columns to break, and where, is a choice among costs, which the exact solver makes with the costs
  of `CssObjective` in their order. It also does not treat a height that the content does not
  reach, which the container has, differently from the one it balances to. The exact solver is used
  on every container; what it costs is in "What the exact solver costs for columns" below.
- **Whether the lines of a paragraph that is cut keep their positions in a window of several
  columns of one pass.** There is no such pass: a column is laid out on its own (the same boxes
  are laid out once per column), so every column has its own positions, and `Writer::write_lines`
  makes the lines of the part that is in the window. The one thing that assumed that the lines of a
  paragraph are one below the other was the "new line" of the text of the paragraph, which is
  fixed (see above).
- **Where the column rules belong if the container is split.** To the fragments: they are made
  from the columns that a page has, so a fragment on a page has the rules of its own columns, and
  balancing of the last fragment only changes how tall they are. Not done until step 3.
- **Orthogonal writing modes.** A container with a vertical flow (`writing-mode`) is not a container
  with columns: it is a block. A container with columns inside of a vertical flow whose own flow is
  horizontal works like any other; one that is an orthogonal flow is a block, too. Nothing
  more has been checked for them.
- **Containers that are laid out outside of the flow** (inside of a flex or grid container, a
  table cell, a box with a `height`, an absolutely positioned box, an `inline-block` box): their
  content is made into a flow all the same, and their columns are laid out whenever the
  box is asked for its layout, so they work. The widths of the `inline-block` boxes in their lines
  are the ones of the first layout, as everywhere.

## Open questions

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
  A subtree that is laid out at a width of its own breaks that reasoning for the boxes around
  it, which are not laid out again; see "A subtree at a width" for what is done.
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
- **A table that spans pages of different widths** is laid out for the width of each page: its
  columns are found again on every page, where the design asks for the columns of the page it
  starts on (step 6).
