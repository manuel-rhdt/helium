# Pagination: design

Status: steps 1, 2 and 4 of "Order of work" are implemented, and so are steps 1 to 3 of the
order of work of "Multi-column layout" (the service answers for a subtree at a width, a container
with columns has balanced columns, and one that is taller than the rest of its page is split over
pages, with the columns of a page filled in turn and the last fragment balanced). `column-span:
all` is not done. The section "Where the code stands" describes the code as it is; the sections
after it are the design, with notes where the code differs.

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
     cell spans over the break. A container with columns is one item, `Item::Container`, which
     the kernel asks the layout service for the fragments of. Every box knows its items
     (`Span`). The content of a container with columns is a flow of its own, with its own
     numbers of items, of the break level `Column` (see "Multi-column layout").
   - `service.rs` is the layout service. For every page width that the kernel asks about it
     lays the whole tree out once with Taffy (a galley) and reads the extents of boxes and gaps
     from it. The lines of a paragraph are what the line breaker makes of it for the width that
     the galley gave it; from a cut on, they are the lines of the whole paragraph if the cut is
     the end of a line of it, and else the rest of the paragraph broken for the width of the
     page. Everything is cached by the width that was asked for. The service also answers for a
     box laid out *at a width of its own*, the content width of that box and not of a page (a
     *subtree*, `subtree.rs`; see "A subtree at a width"), and for the fragment of a container
     with columns that fits in what is left of a page (`fragment`; see "A container that is
     split over pages").
   - `paginate.rs` gives the kernel the geometry of the page that starts at every place: the
     name of the first content on it, whether it starts a page group, and its kind decide the
     margins and the size. The exact solver with `CssObjective` chooses the pages.
   - `columns.rs` chooses and lays out the columns of a container with columns (see "Multi-column
     layout"): for a container that is not split, its balanced columns, for one that is split, the
     columns of the fragment on a page. The container is a leaf for Taffy, so a page layout asks it
     for a height like any other box.
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
- **Multi-column containers.** A container with columns whose height is the one of its content
  is split over pages. One that has a `height`, `max-height`, `min-height` or `aspect-ratio`, one
  inside of another container with columns, a flex or grid container or a table cell, and any in a
  document that is cropped, is one box: it is not split, a box in it that is taller than a page
  overflows it, and the part below the page is not printed (the program says so). A nested
  container is laid out inside of one column. The `inline-block` boxes in its lines are sized for
  the first width that was laid out (like all of them), a table in it is one box, an absolutely
  positioned box that it is the containing block of is not placed, and a container in a vertical
  flow is a block. The fragments of one container are not chosen together with the choice of the
  pages around it: the kernel gets one way to end a fragment for every start and every height
  (see "A container that is split over pages").
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
    /// Inside a container that is split: in front of the fragment of it that starts at the
    /// cut, a place in the flow of the content of the container.
    InContainer { item: ItemId, cut: ContainerCut },
}
```

`InContainer` is the reason for the opaque handle. A break inside a container is a place in
another flow, and the container may have parallel parts: a break inside a grid row cuts several
cells at once, and a flat index cannot say that. The code has the first three variants, with a
`ContainerCut` for a multi-column container (an item of the flow of its content, or a cut in a
paragraph item of it). One position per parallel part comes with the grid and flex containers.

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
    Container { node: BoxRef, avoid_inside: bool },
}

pub enum BreakKind { Allowed, Avoid, Forced, ForcedLeft, ForcedRight }
pub enum BreakLevel { Page, Column }
```

A `Problem` paginates one level (`Problem::level`), and a break of the other level is no break in
it: it is of the kind `Allowed` there. A forced column break outside of a container with columns
is therefore like `auto`. The exception is a forced page break in the columns of a container
(`Problem::page_breaks`): a page break is always a column break, too, and in the problem of a
fragment of a container it ends the fragment. A `Problem` can also limit its number of pages
(`max_pages`): the last page that there can be takes the rest of the content, whatever breaks it
has, and overflows if it cannot hold it. The columns of a container are such a problem. It can
start at a place of its items (`Problem::start`), and be the problem of a fragment of a container
(`Problem::fragment`, see "A container that is split over pages").

Items carry no sizes. The kernel asks the layout service for the extent of a
`Box` or `Gap` on a page of a given inline size.

Nesting is flattened when the flow is extracted, as the paginator does today:
no `Break` is emitted inside a monolithic box, and `break-inside: avoid`
turns the breaks inside the box into `Avoid`.

### Paragraphs and cuts

A cut is an opaque number to the kernel (`Cut`), which only stores it, compares the cuts of one
paragraph and makes tokens of them. The layout service makes them and gives them meaning. In the
engine (`LineCut` in `taffy_layout/tree.rs`) a cut is

```rust
pub(super) struct LineCut {
    at: usize,      // index of a break opportunity of the paragraph
    fitness: u8,    // fitness class of the line that ends at the cut (0 tight .. 3 very loose)
}
```

packed into the number so that cuts are in the order of the paragraph (the break first). Two cuts
at the same break that follow lines that fit differently are two places: the breaking of the rest
of the paragraph depends on both.

A page break inside a paragraph is a break of the paragraph, not a line number: a line number
means nothing once the width changes. The cut carries the state of the line breaker, so that
the demerits for lines of very different tightness in a row and for two hyphens in a row are counted
across a page break. The breaker in `paragraph.rs` has 8 states (4 fitness classes, times whether
the line ends in a hyphen); the hyphen of the line in front is that of the break itself, and
the fitness class is what the cut carries (`LineStart`, `compute_paragraph_lines`). The lines
after a cut that were found with the same state are the lines of the whole paragraph, as before;
the rest of the paragraph after a cut at another state (or at another width) is broken again from
the cut with that state.

**A page can end at any break of a paragraph.** The pages that `candidates` lists end where a
line ends when the layout service breaks the paragraph for the page. That is what the exact
solver and the greedy one choose among, but it is not what the kernel can express. A solution
may have a page that ends at any break of any paragraph, and the layout will lay it out:

- `LayoutService::lines_to(par, from, to, width)` gives the lines of the part of a paragraph
  from `from` to the break `to`. Its last line ends at `to` and is a line like the others (not the
  ragged last line of a paragraph), and the lines before it are broken to fit it
  (`compute_lines_from` with `until`, and `compute_paragraph_lines` with `ends_paragraph:
  false`). Where `to` is the end of a line of the paragraph, it is the lines up to it. Only the
  break of `to` is used; the cut that the last line has, with its fitness class, is the one that the
  next page starts with.
- `Kernel::fill_at(start, item, to)` is the page that starts at `start` and ends in the paragraph
  `item` at `to`, with its extent, the lines it has of the paragraph (for orphans and the
  widows of the page that has the rest) and the rule that it breaks. `Kernel::realize` uses it for
  a page whose end is not among the candidates, so a `Solution` with such pages is valid.
- The page layout (`lines_between`, `lines_until`) breaks the part of the paragraph that is on a
  page that ends at such a cut for that cut.

What is missing is a solver that uses it: nothing proposes such a break yet, and the paragraphs
are still rigid in the sense below, because the number of lines of a part that ends at a cut is
whatever the breaker finds, and the kernel does not choose among the ways to break a part. The
tests of the kernel use a layout service whose paragraphs can be broken anywhere
(`Units`), and the tests of the engine give the layout a solver that ends every page one break
before the page is full (`a_solution_with_pages_that_end_at_any_break_of_a_paragraph_is_laid_out`).

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

**Variants first.** The first version of joint optimisation is simpler than the profiles below:
the breaker gives a few complete breakings of a paragraph, each a rigid list of lines, and the
choice among them is part of the search. For a paragraph at a width, from a start state, the
breakings with the least demerits for the number of lines of the optimum and `extra` lines fewer
and more (`compute_paragraph_variants`, which is what TeX's `\looseness` asks for, with a
band). The first is the optimum, so `extra` = 0 is the breaking that there is now. Every line has
its demerits (`ParagraphLines::demerits`, including those for following a line of another fitness
class and a hyphen), so the demerits of the lines of one page are the share of that page. A
number of lines that no breaking has is not among the results.

The search is that of the optimum with the number of lines as a part of the state, and it is
tested against all breakings of random small paragraphs (with forced breaks, hyphens, a start state
and parts that end at a cut). In release builds it takes about 1 ms for a paragraph of 150 words
(11 lines) with `extra` of 1, against 0.08 ms for the optimum alone, and 6 ms for 400 words
(28 lines). It is meant for documents that ask for it, and the results are kept by the layout
service.

**The layout service gives them.** `LayoutService::variants(par, from, width, extra)` (and
`variants_in` for a subtree) returns the breaking that `lines` gives, and then, if `extra` is not 0,
the breakings with the other numbers of lines, in the order of their number of lines
(`Vec<Rc<[ParLine]>>`). A `ParLine` has its demerits (0 for a service that does not count them, as
the default `variants`, which gives only `lines`, is). The rest of a paragraph after a cut has
variants of its own, broken from the state that the cut carries, and not those of the whole
paragraph, so the state of the search is as it is without them. The service keeps the answers by
paragraph, cut, width and `extra`; the engine's service computes them with
`InlineFormattingContext::compute_variants_from` (`variants_from` in `taffy_layout/tree.rs`).

**The pages choose among them.** `Problem::variants` (`extra`, `tolerance`) asks for it, and the
engine sets it from the property `looseness` (not in CSS: the number of lines more or fewer than
the best breaking that a paragraph may have, inherited, 0 initially, which is the breaking that
there is without it; the value for the whole document is the greatest of its paragraphs, and a
paragraph has at most the ways that its own value allows). With 0 the kernel does what it did.

The walk through a page (`Kernel::walk`) carries a *label* for every way to break the paragraphs
that are on it so far: the room used, the demerits of the lines, which ways were chosen
(`Choice`: the item and the number in the list of the ways that the service gives for the
paragraph from the cut where the page starts in it, if it does, and else from its start), and, for
the paragraph that the page starts in, how many lines of it are on the page. A paragraph branches
every label into one for every way, and a page is listed for each (`PageFill` has
`line_demerits`, `line_extent` and `choices`). Of the labels that use the same room, the one with the
least demerits stays, so a page has at most about two labels for every line that the ways of its
paragraphs can differ by. A page that ends in a paragraph ends at a line of a way, and the next
page continues with ways of its own from the cut, so the state of a node is as it is without
ways. A way whose worst line is worse than the tolerance (TeX's `\tolerance`, 200) is left out,
and the first way is never left out. `Solution::choices` has the ways of the pages, and
`realize` checks them; the layout breaks the paragraphs of a page as the page chose
(`HeliumTree::page_variants`, `lines_between`).

**The exchange rate.** TeX has none: its page builder does not change how paragraphs are broken,
and an author who wants another number of lines sets `\looseness` by hand. What TeX gives are the
scales. Line demerits are squares of a badness-like number (`\linepenalty` + badness + penalties,
squared), and the cost of a page is a badness-like number as well (100 times the cube of how far
its glue stretches or shrinks, and penalties such as `\widowpenalty`, 150). The rule that is used
(`CssObjective::line_room`) is the one that the use of `\looseness` shows: an author accepts lines
as loose as the tolerance allows to gain a line. The demerits of the lines of a page are taken as
the room that the page would be short of, with the square root of the demerits as the badness and a
badness of the tolerance (200) as one line:

    room = line extent x sqrt(demerits of the lines on the page) / 200

and the cost of that room is the cost of the same room left on the page (the cube, in the same
units). A page with lines of badness 200 therefore costs as much as a page that is a line short,
and one with decent lines (badness near 0, demerits about 100 a line) costs next to nothing, so the
extra freedom is used for pages that are empty, not for pages that are full. This is an initial
value that follows the scales of TeX: it has not been compared with anything, and there is nothing
yet that says what is better (see the harness in "Order of work"). The tolerance of the ways and
the one of the rate are the same number, 200.

Restrictions: the ways are chosen for the paragraphs of the flow of the document. A container
with columns has the ways that its paragraphs have inside it narrowed to one (the room that it
has depends on what is in front of it, so the label with the least demerits is kept), the columns
of a container choose no ways, and `Kernel::fill_at` lists a page for the first way only.

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

The pagination of a document is a path in a graph, and a solver searches it. A node is a place
where a page can start, with everything that the pages after it depend on (the kind of the page,
and its number if the number of pages is limited) and nothing else. An edge is a page, from the
place where it starts to a place where it can end. The graph is a DAG that is never built as a
whole: the edges of a node are made when the search gets to it. The edges carry facts (a
`PageFill`), and an objective makes the cost of a path from them, so the graph and the costs can
change independently.

```rust
/// A graph that is searched for the cheapest path from its start to one of its ends.
pub trait Graph {
    type Node: Clone;
    type Edge;
    type Key: Copy + Eq + Hash;   // nodes with the same key have the same future
    type Rank: Ord;               // every edge goes to a node of a greater rank
    fn start(&mut self) -> Self::Node;
    fn key(&mut self, node: &Self::Node) -> Self::Key;
    fn rank(&mut self, key: Self::Key) -> Self::Rank;
    fn is_end(&mut self, node: &Self::Node) -> bool;
    fn edges(&mut self, node: &Self::Node) -> Vec<(Self::Edge, Self::Node)>;
}

/// The cheapest path, found by taking the nodes in the order of their rank.
pub fn shortest_path<G: Graph, C: Clone + PartialOrd>(
    graph: &mut G, zero: C,
    weigh: &mut dyn FnMut(&G::Edge) -> Option<C>,
    combine: &mut dyn FnMut(&C, &C, &G::Edge) -> C,
) -> Option<Path<G::Node>>;

pub trait Solver {
    fn solve(&self, kernel: &mut Kernel) -> Solution;
}

pub trait Objective {
    type Cost: Clone + Debug + PartialOrd;
    fn zero(&self) -> Self::Cost;
    /// The cost of a page, or `None` if it cannot be used.
    fn page_cost(&self, fill: &PageFill, is_last: bool) -> Option<Self::Cost>;
    /// The cost of the pages so far and one more: a sum, or a maximum.
    fn combine(&self, so_far: &Self::Cost, page: &Self::Cost, index: usize) -> Self::Cost;
}
```

`Kernel` is a graph through the adapter `PageGraph` (the node is a `PageStart`, the edge a
`PageEdge`: the `PageFill`, the number of the page and whether it is the last). The exact solver
is `shortest_path` on that graph. A policy that does not search everything (greedy, a beam, a
learned one) chooses among the edges of a node and is written against the same trait.

**A cost is a type.** The costs of CSS have a priority: a page that does not fit is worse than any
number of avoided breaks, which are worse than any number of orphans and widows, which are worse
than any amount of empty pages. This is a struct of fields in the order of their priority,
compared field by field (`CssCost`, with `#[derive(PartialOrd)]`), and not one number with large
constants for "very bad" (the paginator had 1e12 for an avoided break, 1e11 for orphans and
widows, 1e30 for a page that does not fit and -1e15 for a forced break). Those encoded the
priorities in magnitudes, and a sum with a large constant has no digits left for the costs below
it: with the 1e30 of one page that does not fit, every pagination of the document cost the same,
whatever else it broke; with the -1e15 of the forced breaks of a book of 61 chapters, differences
of the empty room below about 2 px were lost. A forced break is not a cost at all now:
`candidates` ends every page at it, so all paginations have the same ones.

The room that is left on a page is counted in whole units of 1/64 px (`CssCost::slack_of`). The
positions are single precision numbers and the room is a difference of two of them, so two
paginations with the same pages in another order differ in the last digits of their cost. A
rounding error should not decide between them: the rule of the exact solver does (of paginations
of the same cost the one that has the earlier start of the last page wins). Without the whole
units the layout of `gallery/book` changed at a page where a page with 20.8 px left and one with
1.5 px left swapped places.

The exact solver minimises what `combine` makes of the costs of the pages: the sum for pages, the
maximum for balanced columns (`ColumnCost`, `TallestColumn`, `FullestFirst`). If the number of
pages is limited, the number of the page is part of the state of the search.

Planned solvers:

- **Greedy.** Take the candidate with the largest fill. Step back while it
  violates `avoid`, orphans or widows and an alternative exists. One call of
  `candidates` per page.
- **Exact.** Shortest path over (`BreakToken`, `PageKind`), memoised, with an
  `Objective`. With the tolerance of all paragraphs set to zero this is the
  current paginator with the page class handled correctly.
- **Joint.** The exact solver with a positive tolerance.
- **Pruned and evolved solvers.** Any policy over `candidates`.

`CssObjective` has the costs of the paginator as a `CssCost`: pages that do not
fit, avoided breaks, orphans and widows, and the cube of the unused height, and nothing
for the last page and for pages that end at a forced break.

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

    /// The ways to end a fragment of a container that starts at `from` within `available`.
    fn fragment(&mut self, container: BoxRef, from: Option<ContainerCut>, page_inline: f32,
                available: f64) -> Rc<[Fragment]>;

    /// Lays out one chosen page and writes the positions of its boxes.
    fn layout_page(&mut self, start: BreakToken, end: BreakToken, spec: PageSpec,
                   lines: &LineChoices) -> PageLayout;
}

pub struct Fragment {
    /// Where the next fragment starts, or `None` if the container ends in this one.
    pub end: Option<ContainerCut>,
    pub used: f64,
    pub end_kind: BreakKind,
    /// Rules broken inside the fragment (orphans, widows, avoid), which count against the page.
    pub orphan_violation: bool, pub widow_violation: bool, pub avoid_violation: bool,
}
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
- `PageLayout` does not exist. `fragment` does, for multi-column containers: its container is a
  `BoxRef`, it starts at a `ContainerCut` and not at a token (the service does not know tokens), it
  has the height that is left on the page as an `f64`, and it returns one fragment (see "A container
  that is split over pages"). Tables are split between rows without it.

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
5. Cuts with state in the line breaker (done); a page that ends at any break of a paragraph (done);
   variants of a paragraph by number of lines in the breaker (done) and in the layout service
   (done); variants in the kernel and the layout, with TeX's tolerance and an exchange rate that
   follows its scales (done, opt-in with `looseness`); the harness that says whether it is better;
   a better rate from it; profiles after that.
6. `fragment` for tables between rows, with pinned columns.
7. `fragment` for grid and flex, with parallel parts.

Steps 1 to 3 are the base for iterating on solvers. Steps 5 to 7 are
independent of each other.

## Multi-column layout

Status: steps 1 to 3 of "Order of work" below are implemented: a container with `column-count` or
`column-width` whose content is block flow or one paragraph has balanced columns, rules and breaks
between columns, and one that is taller than the rest of its page is split over pages: the columns
of a page are filled in turn, and the last fragment is balanced (`column-fill`). `column-span: all`
(step 4) is not done. A container that has a height of its own is not split (see "A container that
is split over pages" for what is not).

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

**Nested kernels.** A container with columns is one item (`Item::Container`, or `Item::Box` if it
is not split) in the flow of its page, with a fragment on every page that is found by an inner
kernel run: a `Problem` whose pages are the N columns, as wide as a column, over the flow of the
content from the place where the fragment starts. (The `fragment` call that steps 6 and 7 add for
tables, grid and flex is the same thing for a container that is split, and it was built for the
containers with columns first: see "A container that is split over pages".)

**Balancing is a min-max problem.** With `column-fill: balance` (the default) a container with an
`auto` height is as short as it can be while its content still fits in N columns. The exact solver
adds up the costs of the pages by default. `Objective::combine` is the operator for this (the
sum for `CssObjective`, so everything else is as before). Balancing takes the maximum, in two
exact steps (`balance`, with the objectives `TallestColumn` and `FullestFirst`):

1. The maximum of the heights of the columns is minimised. The height of a column is its content
   plus the rules that it breaks: a column that does not fit is worse than any column that ends where a
   break is to be avoided, which is worse than any with orphans or widows, which is worse than any
   height (`ColumnCost`, compared field by field, in the order that `CssCost` has). So a column is
   only as tall as it takes to keep the rules, if that is possible.
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
the problem is a break in it, with one exception: a forced break of level `Page` in a container with
columns is a column break, too, and it ends the fragment of the container on its page (it is a
break to the left or the right page if it asks for one). A change of the page name between two
children of a container is not a break. A
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
- **`column-fill: auto`** is not balanced and the columns are filled in turn, but only for a
  container that is split (an item of the flow of a page: see "A container that is split over
  pages"). One that has a `height` is balanced within it whatever `column-fill` says (the
  columns of a container with a height that is filled in turn would need `CssObjective` over the same
  problem, with the height as the size of the columns; not done). `column-fill: balance-all` is
  parsed; it differs from `balance` only for a fragment that ends at a forced page break.
- **`min-height` and `max-height`** do not change the columns: the container is that high, and the
  columns are as high as the content needs. A container that has one is not split over pages.

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
the container is split over pages every fragment has the rules of its columns, as tall as the columns
of the fragment, because the rules are made from the columns that are written. A column rule is
only drawn between two columns that both have content, and fragmentation splits a multi-column
container into several lines of columns, each with its own gaps and rules (CSS Multi-column Layout
Level 1, sections 3 and 4).

The text of a paragraph that runs over two columns is one `Paragraph` of the display list with
a new line from one line to the next: the new line says where the next line is when it is not
directly below (at the top of the next column) and moves the text to it.

### A container that is split over pages

Steps 3a, 3b and 3c. A multi-column container is split over pages if it is an item of the flow of a
page (`Item::Container`, made by `Extractor::emit`): its height is the one of its content (no
`height`, `max-height`, `min-height` or `aspect-ratio`), the document has pages that have a size
(not `--crop`), and it is not inside of another container with columns, a flex or grid container, a
table or an `inline-block`. Every other container with columns is one `Item::Box`, as before.

**The item and the tokens.** The item stands for the whole border box of the container. The padding
and the border at the top are part of its first fragment, and the ones at the bottom of its last;
the margins around the container are gaps of the flow, as around any block. A page can end inside
the item, at `Position::InContainer { item, cut }`, and the next page starts there. A `ContainerCut`
is a place in the flow of the content of the container: in front of one of its items, or a cut in a
paragraph item of it, and its numbers are in the order of the content, so that the exact solver can
order the places of the outer flow and of the container together. Tokens stay opaque: the kernel
makes one for every end that the layout service answers with.

**The question.** `Kernel::candidates` walks the items of a page. At a container it asks the
service `fragment(container, from, page_inline, available)` for the fragment that starts at the
cut it was given (`None` at the start of the container) and has `available`: what is left of the
page after the content and the gaps in front of the container. The service answers with one
`Fragment`. If it has an `end` the page can end there, and nothing else is on the page (the
fills are listed at that token and the walk stops). If it has none the container ends in the
fragment: the walk goes on behind it, with the extent of the fragment added. `used` is the whole
extent: the columns, and the padding and border at the top and at the bottom where they are part of
the fragment. A fragment taller than what is available gets the rule of every unbreakable item: the
page may end behind it if nothing else is on it, and the page is an overflow. The rules that a
fragment breaks inside (orphans and widows in a column, a break that should be avoided in a column
that is not the last) are flags of the fragment; they count against the page like orphans and
widows at the end of a page. `break-inside: avoid` on the container or on a box around it makes
the end of a fragment inside it an `avoid` break. The page that a fragment is on is given to the
layout of the page by `Kernel::fragments(start, end)`: the same walk, which records for every
container what it asked and what it was answered. This is what a page needs to find the plan of
the fragment again, so that the layout of the page and the choice of the pages cannot disagree
(the numbers are the ones the kernel asked with, and the same arguments give the same answer).

**The inner problem of a fragment.** `helium_paginate::plan_fragment` makes the answer; it is in
the crate so that the kernel tests can run it, and the engine only gives it the extents and lines
of the content at the width of a column (`InColumns`, as for a container that is not split).

1. The columns that are as full as they can be. The problem has the N columns as its pages, as
   high as what is available (minus the padding and border at the top), over the items of the
   content, starting at the cut (`Problem::starting_at`). It is the problem of a fragment
   (`Problem::as_fragment`): it has exactly N pages, or fewer if the content ends first, and its
   last page is not the one that takes the rest, so that it may end at any of its breaks. A forced
   page break in the content is a column break and the end of the fragment. The exact solver with
   `CssObjective` chooses the columns, so `avoid`, orphans and widows are kept as far as possible
   and every column but the last is as full as it can be. The solver is made for a limited
   problem: the states are the places where a column starts and the number of the column, and
   the solution has one entry more than it has pages, the place where the last one ends
   (`Solution::fragment_end`, `Kernel::is_final`).
   The lines that the next fragment starts with are counted at the end of the last column: a
   cut in a paragraph that leaves fewer lines than `widows` is a widow violation of that column.
2. If the content ends within the N columns, this is the last fragment. The columns are balanced
   (`balance`) with `balance` and `balance-all`, within what is available minus the padding and
   border at the bottom, and not with `auto`, where they are the columns of step 1. If the padding
   at the bottom leaves too little room for the content, the fragment does not end the container
   (the columns of step 1 are made for the room that there is without it).
3. If the fragment ends at a forced page break, it is balanced with `balance-all` only, over the
   items in front of the break.
4. The fragment is as tall as its tallest column. A fragment that is not the last is therefore
   not stretched to the bottom of the page by the columns; its box is, as is every box that goes on
   on the next page. The fragment that starts a container in the middle of a page has what is
   left of the page, the others the height of the page.

The answer is cached by the arguments, and so is the plan (`ColumnsNode::fragments`), which has the
windows of the columns. The outer solver is given one way to end a fragment for every start and
every height: it chooses where the pages before and after the container start and end, and not where
a fragment ends among the possibilities of its columns. That is a restriction (see below).

**Windows.** The window of a page is the one of every page: the items of the flow from the start to the
end, and the container is the box that holds a fragment. The page ends inside the item, and its
span says nothing about that, so the window knows the containers that it starts and ends in
(`Window::start_container` and `end_container`): the box is cut there (no padding, border or margin
at the cut, no minimum height, and it has a fragment of its own for the decoration), as a box with
items on both sides of the window is. In the layout of the page the container is a leaf for Taffy, as
high as the plan found for it (`HeliumTree::page_fragments`, set from `Kernel::fragments` for the
page). Its columns are laid out one after the other, every one through a window of its own that
replaces the one of the page while it is set (`save_window` and `restore_window`), and written with
the layouts of that column. The windows are therefore nested: the one of the page, and the ones of
the columns of the fragments on it. In a layout that is not of a page (the galleys of the layout
service, which only give extents) the container is as high as nothing and its columns are not
laid out: it is not balanced for its whole content there. This is what makes a container of
thousands of lines cheap: the pagination with the balancing of 3,993 lines in one container took
0.22 s, and the pagination of the same lines split over 50 pages 0.014 s (release build).

**Decisions where this document was open.**

- *Rules* are drawn in every fragment, between the columns that have content on it, as tall as the
  columns of the fragment. The specification says that a multi-column container has lines of columns
  (one for every fragment), that rules are drawn between two columns that both have content, and
  that their length is the height of the column.
- *`column-fill: auto`* fills the columns in turn up to what is available on the page, whether the
  container is split or not: a container that fits is as high as its tallest column, and may have
  its content in the first column only. (This changes the layout of a container with `column-fill:
  auto` that was balanced before.) A container with a `height` is balanced as before.
- *`column-fill: balance-all`* is parsed and implemented: it balances the fragments that are not
  full, which are the last one and the ones that end at a forced page break. A fragment that has
  filled its columns is balanced already.
- *A forced page break* in a container ends the fragment; the break is to the left or the right
  page if it asks for one, and then the margins at the start of the next fragment are kept. A
  change of the page name between two children does not end it.
- *The height of a fragment* is its tallest column, and a fragment that is not the last is not made
  higher than that (the box of the container reaches the bottom of the page).
- *`min-height`* on a container makes it a box that is not split, because the minimum would have to
  be applied to the container as a whole.
- *A different page width* is found per fragment, as the specification has it (multicol 3.4): a page
  of another width has columns of its own count and width, because the service asks for the width
  of the content box of the container on that page. What the fragments of two pages have in common
  is the flow of the content.

**Restrictions.**

- A container with a `height` keeps the behaviour of step 2: its columns are balanced within the
  height and the content that does not fit overflows below the last column. (Divergence: in paged
  media the specification puts the columns that do not fit on the next page, multicol 8.2.)
- A multi-column container inside of a split one is one box in a column; it is not split. A table
  in a column is one box.
- A container inside of a flex or grid container, a table cell, an `inline-block` box or an
  absolutely positioned box, and a container in a document that is cropped (`--crop`, where
  every box has a page of its own), is not split. A container in a `foreignObject` is split over
  the pages of the island, and only the first is shown, as for any other content.
- A box in a column that is taller than a page, and cannot be split, overflows the page, and what is
  below the page is not printed (a warning says so).
- The outer solver has one way to end a fragment for every start and every available height, the
  one that the inner exact solver finds best for the columns. It cannot make a fragment end earlier
  or later than that to improve the pages around the container, except by moving the start of the
  container. The rules that a fragment breaks inside count against it, so a container that can
  only start on a page with a widow moves to the next page.
- The lines that are counted against `widows` at the end of a fragment are the ones that the
  paragraph has from the cut at the width of the same column. If the next page has another width,
  the lines are broken again there and there may be another number of them.
- The size of the `inline-block` boxes in a paragraph is the one of the first width, as everywhere.
- `column-span: all` is parsed and ignored.

**Divergences from the specifications.** The columns that do not fit in a container with a `height`
(above). The height of a fragment that ends at a forced break or at the end of the content with
`column-fill: auto` is the tallest column: the specification only gives an example for a
container with a height. The specification does not say how the content is divided between the
columns when a rule cannot be kept; the choice is the one of the exact solver with the costs of
`CssObjective`.

### The common case is a container over pages

For a renderer of pages the container that is taller than a page is the main case: a two-column
article whose text runs over several pages, with the columns of every page filled in turn. It was
a defect of the first versions: the container was one box balanced over all its content, and what
was below the page was laid out and not printed. The section above is what fixes it; the
tests in `tests/multicol.rs` (`the_columns_of_every_page_are_filled_in_turn_with_column_fill_auto`
and the ones next to it) describe the arithmetic. What is still missing for an article is not part
of this document: floats for figures in a column, footnotes, and spanning figures. Text in
columns works; `column-span: all` is step 4.

What the specifications say (CSS Multi-column Level 1, CSS Fragmentation Level 3, read on
2026-11-16), and what is done about it.

- `column-fill: balance`, the initial value: "In fragmented contexts, only the last fragment is
  balanced." `balance-all` balances all fragments, and `auto` fills the columns in turn
  (multicol 7.1). All three are parsed and done.
- Content that comes right in front of a spanner is balanced whatever `column-fill` says; the
  content after a spanner is balanced or not as `column-fill` says (multicol 7.1, from the
  description of the property and the names of its tests; the normative text was not read in
  full). Not done with the spanners.
- A page break inside a container is always both a page break and a column break: the container
  gets a new row of columns on the next page (fragmentation 2.2). A forced break "forces ensuing
  content into the next fragmentainer of the type associated with the break, breaking through as
  many fragmentation contexts as necessary"; one that is not inside a context of its type has no
  effect (fragmentation 4.3). So a forced page break inside a container ends the fragment, and a
  forced column break outside one is ignored, which is what the break levels do. Done.
- Spanners are honored in every fragment. The margins of two adjacent spanners collapse; the
  bottom margin of a spanner does not collapse with the top margin of what follows, because a
  column box is a new block formatting context (multicol 6.1). Not done.
- A container that has more columns than it has room for (a `height` that is too small, or
  forced column breaks) puts the extra columns on the next page in paged media (multicol 8.2).
  A container with a `height` lets such content overflow below the last column, which is the rule for
  continuous media only. A container without a height does it right: the columns of a fragment
  that are too many are the next fragment.
- The column count and the column width may be found per fragment in paged media (multicol 3.4).
  They are, per page width.
- A fragmentainer of another size is laid out as if the whole element were fragmented across
  fragmentainers of that size; intrinsic sizes are kept across the whole element, and the
  initial containing block is that of the first fragmentainer (fragmentation 5.1). This is what
  the windows do.
- Column rules are drawn between two columns that both have content, and a container that is
  fragmented has a line of columns for every fragment (multicol 3 and 4). Done: the rules are per
  fragment.

Not checked against the text: the rules for `box-decoration-break` and margins at breaks
(fragmentation 5.2 to 5.4); the fragments of a box that is cut have no border, padding or rounded
corners at the cut (`box-decoration-break: slice`), and the margins at an unforced break are
discarded and at a forced one kept, as for pages.

### Order of work

1. Done. The service answers the extent and the lines of a subtree at a given width of its
   content box, instead of reading them from a galley. It is what the columns are chosen from; it is
   also what pinning the columns of a table that is split needs.
2. Done. `column-count`, `column-width`, `column-gap` and `column-rule` for a container that is not
   split over pages and whose content is block flow or one paragraph. Balanced. The combine
   operator of `Objective`. Break levels, and `break-before: column`.
3. Done. A container that is split over pages (see "A container that is split over pages").
   a. Columns filled in turn (`column-fill: auto`) over pages, with the first fragment as high as
      the rest of its page, forced page breaks inside the container, and no loss of content for
      a container that is taller than a page.
   b. The last fragment balanced, and `column-fill` as the specification has it (`balance-all`
      too).
   c. `fragment` for a container in the middle of the flow: the kernel asks the layout service
      for the fragment that fits. Steps 6 and 7 above can use the same question for tables, grid
      and flex containers; the one for tables needs pinned columns, and the ones for grid and flex
      containers need parts that are side by side, which the multi-column container has as columns.
4. `column-span: all`.

The tests of step 3 are in `tests/multicol.rs`, below the others. The first versions of three of
them forgot that `column-gap: normal` is 1em in a multi-column container (16px at the default font
size), which puts the second column at x = 208 and not 200. They set `column-gap: 0` now.

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
  balancing of the last fragment only changes how tall they are. Done.
- **Whether the outer solver needs several ways to end a fragment.** It has one: the one that the
  exact solver finds best for the columns. More would let the outer solver trade the end of a fragment
  against the pages around it, at the price of one inner solution for every end (the number of
  starts of a fragment would be the number of places in the container, as it is for the pages of a
  paragraph). Not measured; the rules that a fragment breaks are costs of the page already.
- **`page` names inside a container.** A change of the page name between two children does not end
  the fragment, and the page that continues the container is of the name of the container. The
  specification asks for a break there; not done.
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
- Whether the column rules (`column-rule`) belong to the container or to its fragments: resolved,
  see "Multi-column layout".
- Orthogonal writing modes. A column is a block-axis fragmentainer, and the inline axis of a
  vertical document is the vertical one. Nothing here has been checked for them.

## How much `looseness` does

`tools/looseness_bench.py` renders generated documents (plain prose, chapters that each start on a
new page, prose with unbreakable figures; 20 seeds each) with `looseness` 0, 1 and 2 and measures the
pages, the room left at the foot of the pages that are not the last (from the words of the PDF, in
lines), and the badness of the lines (the log line `LINES:`).

| kind | pages | room | lines | lines worse than 100 | mean badness | documents with fewer pages |
| --- | --- | --- | --- | --- | --- | --- |
| prose | 19.70 to 19.60 | 7.06 to 4.31 | 454.6 to 455.7 | 23.8 to 24.4 | 26.5 to 26.8 | 2 of 20 |
| chapters | 56.40 to 56.20 | 130.7 to 122.6 | 1213.8 to 1216.5 | 64.0 to 65.9 | 28.0 to 28.3 | 4 of 20 |
| figures | 24.40 to 24.30 | 45.2 to 38.9 | 348.0 to 352.1 | 19.6 to 22.5 | 30.1 to 31.3 | 2 of 20 |

No document got a page more, and `looseness` 2 gives the same as 1. The effect is small: a third of a percent of the
pages and between a seventh and a third of the room, for a few lines that are looser. The synthetic
text has a short measure, and the lines that another way to break a paragraph has are bad
(the median of the worst badness is 2900 for a line more, and the paragraphs that can have a line
more within the tolerance of 200 are 5 of 78), so these numbers are a lower bound for text that is
set to a longer measure.

The two numbers that decide how much is taken can be changed without rebuilding
(`HELIUM_LINE_TOLERANCE`, the exchange rate of lines and room, and `HELIUM_VARIANT_TOLERANCE`, the
worst badness of a line of another breaking; both 200 by default), and the harness takes them as
`--knob LINE_TOLERANCE=400`. With `looseness: 2`, 20 documents of each kind:

| knob | chapters: pages, room | figures: pages, room, lines worse than 100, mean badness |
| --- | --- | --- |
| `looseness` 0 | 56.40, 130.7 | 24.40, 45.2, 19.6, 30.1 |
| line 50 | 56.20, 125.3 | 24.30, 41.3, 19.9, 30.2 |
| line 200 (default) | 56.20, 122.6 | 24.30, 38.9, 22.5, 31.3 |
| line 800 | 56.20, 121.4 | 24.30, 38.7, 22.9, 31.4 |
| variant 1000 | 56.15, 121.3 | 24.30, 35.9, 27.4, 34.7 |
| both open (variant 100000, line 20000) | | 24.30, 7.4, 153.7, 734 |

A greater tolerance of either kind trades worse lines for less room, in steps that get smaller
for the exchange rate, and not at all for the default. Opened up completely it takes the room
almost away, and the lines are bad (a mean badness of 734): the harness shows what the knobs buy,
and the price.

### Columns

The columns choose among the ways to break their paragraphs too (`plan_fragment` and `choose`
give the inner problems the variants, `ColumnCost` has the price of the lines in its height, and
the plan carries the ways of every column to the layout and to the writer). `looseness` in columns
is measured by the kinds `balanced` (containers that are not split) and `columns` (containers that
go over pages) of `tools/looseness_bench.py`:

| 20 documents, 300 containers | height of the containers, lines | containers shorter | taller |
| --- | --- | --- | --- |
| `looseness` 0 | 342.2 | | |
| `looseness` 1 and 2 | 341.1 | 22 (7%) by a line | 0 |
| both tolerances open | 340.7 | | |

A balanced container is shorter by a line only if a breaking with a line fewer is within the
tolerance, and only when the number of lines is odd for two columns, which is why a tenth of the
containers gain and the rest cannot: a breaking with a line more never makes a balanced container
shorter. In containers that go over pages the first fragments are filled, and a looser breaking
holds less text, so the container ends later; the pages are the same in number. Whether a taller
container is good depends on the page (a page that is left with room, an orphan), which the inner
choice cannot know: the page has to choose among fragments (see "Multi-column layout").

### Fragments of a container that end earlier

`LayoutService::fragment` has always returned a list, and the kernel makes a page of every
fragment in it (the `end` of a fragment is part of what the page records, `PageFragment`), but the
tree service gave only the fragment that fills the page. It gives now that one and up to three
that end earlier: the columns are chosen again for the room less one, two and three lines of the
first paragraph (`HELIUM_EARLIER_FRAGMENTS`, 3 by default, 0 for the old behaviour), and the ones
that end at a place that another has are left out. The plans are kept by where the fragment starts,
the page, the room, and where it ends. The page takes a fragment that ends earlier where that
costs less in total, which is where the room left on the pages is spread better.

Measured over 30 documents of two kinds that go over pages (the kinds `headings`, with headings
that must not end a column, and `columns`; the room is of the pages that do not end at a forced
break nor are the last):

| kind | pages | room in lines | sum of the cubes of the room | the most room on a page |
| --- | --- | --- | --- | --- |
| headings, only the fragment that fills the page | 398 | 141 | 1131 | 6 |
| headings, with the earlier ones | 398 | 113 | 209 | 3 |
| columns, only the fragment that fills the page | 370 | 145 | 1597 | 10 |
| columns, with the earlier ones | 370 | 130 | 190 | 2 |

(Measured after the margin between containers was counted, see below; before it the numbers were
smaller, because the pages were counted too short.)

No page broke a rule of orphans and widows or ended at an avoided break in either case, so the
gain is in the room, and not in the rules: the inner choice already keeps the rules, and what it
cannot do is leave room on the page for the next fragment to start better. The time that the
fragments take has not been looked at yet: it is more than without them, and it grows with the
size of a container.

`tools/check_quality.py` renders the generated documents with and without them and checks the
pages: the words are the same ones (none lost, none twice), every word is inside the content area of
its page, the layout and the paginator agree on the height of the pages, and no page breaks
orphans or widows or ends at an avoided break. All hold with and without the fragments, over 60
documents. The check found a defect that was not caused by them: a container that is split over
pages was laid out with no height where the extents of the document are taken, so its bottom margin
collapsed through it, and the gap between two containers was not counted: a page with several
of them had up to 20px of content below its bottom for each. Such a container has a height that is
not zero there now.

### What the first measurements got wrong

Two results of the first runs looked like bugs, and one was.

- *More looseness gave emptier pages.* The pages were charged for the demerits of all of their lines
  as soon as one paragraph of the document asked for variants, so a page cost more the more lines
  it had, and the search preferred pages with fewer of them. One paragraph with `looseness: 1`
  changed the pages of the whole document (a document of 12 pages whose first paragraph had it had
  pages of 20 to 22 lines instead of 23 and 24). A page is charged now for the demerits of its lines
  above those that the first ways to break the paragraphs have (`Label::baseline`), which are
  zero for a page that takes only those, and the document is paginated as without `looseness`.
- *The room was measured wrong.* The text of the layout dump counts a line more on pages where a
  paragraph goes on, so the harness took the room from the words of the PDF.
- *`looseness: 2` gave the same as 1.* It does not need to be a defect: the breaker gives the ways of
  two lines more, but none of 78 paragraphs has one that is within the tolerance, and their lines
  would cost 57 lines of room at the default exchange rate (20 lines for one line more). With both
  knobs opened up the results differ.

### Looking at the choices

`helium --dump-pagination FILE` (`-` for stdout) writes what the paginator chose and why, as JSON:
for every page its room, how it ends, orphans, widows, avoided breaks and overflow, its cost in
the parts of the typed cost, the price of its lines in room (`line_room`), the ways chosen for its
paragraphs, and for every paragraph on it the lines it has with their badness and the ways it
had (the number of lines, the demerits and the worst badness of each).
`python3 tools/pagination_report.py FILE [--ways]` shows it as a table.

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
