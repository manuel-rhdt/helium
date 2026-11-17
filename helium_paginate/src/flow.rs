use crate::token::Position;
use std::fmt;
use std::sync::Arc;

/// The index of an item in the flow.
pub type ItemId = usize;

/// A box or a gap of the document, which the layout service knows the extent of.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct NodeRef(pub u32);

/// A box of the document that the layout service can lay out on its own, at a width that is not
/// the width of any box around it: the root of a subtree (see `LayoutService::content_extent`).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BoxRef(pub u32);

/// A paragraph of the flow.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ParId(pub u32);

/// How a place where a page may end is treated.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BreakKind {
    Allowed,
    /// A break that is discouraged by `break-before`, `break-after` or `break-inside`.
    Avoid,
    Forced,
    ForcedLeft,
    ForcedRight,
}

impl BreakKind {
    pub fn is_forced(self) -> bool {
        matches!(
            self,
            BreakKind::Forced | BreakKind::ForcedLeft | BreakKind::ForcedRight
        )
    }
}

/// The kind of fragmentainer that a break is between two parts of: a page or a column.
///
/// A problem paginates one kind of fragmentainer ([`Problem::level`]) and only knows the breaks
/// of that level. A break of the other level is no break there: a forced column break outside of a
/// multi-column container is like `auto`, and a forced page break inside of a multi-column
/// container that is not split over pages ends nothing.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub enum BreakLevel {
    #[default]
    Page,
    Column,
}

/// A piece of the flow, in the manner of TeX. Items carry no sizes: the kernel asks the layout
/// service for them.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Item {
    /// Cannot be split: a monolithic box, or the padding and border of a box that is open
    /// around the items that follow.
    Box { node: NodeRef },
    /// Collapsed margins. Discarded at the start and at the end of a page; kept after a forced
    /// break and at the start of the document.
    Gap { node: NodeRef },
    /// A place where a page may end. The next page starts behind the item. The break is one of
    /// the `level`; if the problem is of the other level, the place is a break of the kind
    /// `Allowed`.
    Break { kind: BreakKind, level: BreakLevel },
    /// A paragraph of the flow. Its lines are rigid: a page can only cut between the lines that
    /// the layout service has for it, which depend on the width of the page and on where the
    /// paragraph starts on it.
    Paragraph {
        id: ParId,
        orphans: u32,
        widows: u32,
        /// Whether the breaks between the lines are `BreakKind::Avoid`.
        avoid_inside: bool,
    },
    /// A container that is split over pages: a multi-column container. Its content is a flow of
    /// its own that the layout service knows; the kernel asks it for the ways that a fragment of
    /// the container can end within the height that is left on a page
    /// ([`LayoutService::fragment`](crate::LayoutService::fragment)). The padding and the border
    /// of the container are part of the fragments (the top ones of the first fragment, the
    /// bottom ones of the last), and the margins around it are gaps of the flow.
    Container {
        node: BoxRef,
        /// Whether the breaks inside the container are `BreakKind::Avoid`.
        avoid_inside: bool,
    },
    /// A float, in the manner of TeX: the place in the flow where it is *called for*. The box of
    /// the float is not part of the flow. If it fits, it takes room at the top or the bottom of
    /// the page that has this place, and otherwise it waits for a later page (see
    /// [`FloatList`](crate::FloatList)). Floats keep their order.
    Float {
        /// The box of the float. The layout service knows its extent on a page
        /// ([`LayoutService::float_extent`](crate::LayoutService::float_extent)).
        node: BoxRef,
        side: FloatSide,
    },
}

/// Where a float is on its page.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum FloatSide {
    /// At the top of the page, above its text.
    Top,
    /// At the bottom of the page, below its text.
    Bottom,
}

/// The kinds of page that differ in their geometry.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum PageKind {
    First,
    Left,
    Right,
}

/// The geometry of the content area of a page.
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct PageSpec {
    /// The width of the content area.
    pub inline_size: f32,
    /// The height of the content area.
    pub block_size: f32,
}

/// The geometry of the pages: of the page that starts at a position and has a kind.
///
/// The geometry follows from the kind of the page, from the name of the first content on it and
/// from whether it starts a page group. The name and the group are properties of the place where
/// the page starts, the kind is the one of the page number. The state of an exact search is
/// therefore the place and the kind.
#[derive(Clone)]
pub struct PageTable {
    /// Whether the pages are bound on the right: the first page is a left page.
    pub rtl: bool,
    /// Whether the geometry of the pages depends on whether they are left or right pages. If it
    /// does not, a search does not have to tell the two kinds of page apart (breaks to a left or
    /// a right page do tell them apart, however).
    pub left_right_differ: bool,
    spec: Arc<dyn Fn(Position, PageKind) -> PageSpec + Send + Sync>,
}

impl fmt::Debug for PageTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageTable").field("rtl", &self.rtl).finish_non_exhaustive()
    }
}

impl PageTable {
    /// All pages are the same.
    pub fn uniform(spec: PageSpec) -> Self {
        PageTable::new(false, move |_, _| spec).left_right_differ(false)
    }

    /// The geometry of a page is given by a function of the position it starts at and its kind.
    pub fn new(
        rtl: bool,
        spec: impl Fn(Position, PageKind) -> PageSpec + Send + Sync + 'static,
    ) -> Self {
        PageTable { rtl, left_right_differ: true, spec: Arc::new(spec) }
    }

    pub fn left_right_differ(mut self, differ: bool) -> Self {
        self.left_right_differ = differ;
        self
    }

    /// The kind of the page with the number `index` (the first page has the number 0).
    pub fn kind(&self, index: usize) -> PageKind {
        if index == 0 {
            PageKind::First
        } else if (index % 2 == 1) != self.rtl {
            PageKind::Left
        } else {
            PageKind::Right
        }
    }

    /// The geometry of the page of the given kind that starts at `at`.
    pub fn spec(&self, at: Position, kind: PageKind) -> PageSpec {
        (self.spec)(at, kind)
    }
}

/// What the kernel paginates.
#[derive(Debug, Clone)]
pub struct Problem {
    pub pages: PageTable,
    pub items: Vec<Item>,
    /// What the pages are: the pages of a document, or the columns of a multi-column container.
    pub level: BreakLevel,
    /// The number of pages there can be. The last of them takes what is left of the content,
    /// whatever breaks there are in it, and overflows if it is more than it can hold. Without a
    /// limit there can be any number of pages.
    pub max_pages: Option<usize>,
    /// Where the first page starts: in front of the first item, unless the problem is the
    /// rest of a flow, from a place on.
    pub start: Position,
    /// Whether the pages are the columns of one fragment of a container, which has to end where
    /// the last of its `max_pages` pages does, or at a forced break of the page level, instead of
    /// at the end of the items: the solution ends at the place that its last page ends at
    /// ([`Solution::end`](crate::Solution::end)), the last page may end at any of its breaks, and
    /// a forced break of the page level ends the fragment (see [`Problem::kind_of`]).
    pub fragment: bool,
    /// Whether a forced break of the page level is a break here, too (the pages are the columns
    /// of a container: a page break is always a column break, too).
    pub page_breaks: bool,
    /// How many ways to break a paragraph a page chooses among.
    pub variants: Variants,
    /// For the problem of a fragment of a container: the item of the first float in front of
    /// `start` that waits for a column ([`ContainerCut::waiting`](crate::ContainerCut::waiting)).
    /// The floats that wait are the ones from this item to `start`.
    pub waiting: Option<usize>,
    /// The part of a page that floats can take together, where text goes on the page
    /// ([`FLOAT_FRACTION`](Problem::FLOAT_FRACTION)). A float that does not fit under it waits
    /// for a page of floats.
    pub float_fraction: f32,
    /// Boxes that are repeated at the top of the pages that go on in the middle of a table: the
    /// header rows (see [`Repeat`]).
    pub repeats: Vec<Repeat>,
}

/// A box that is repeated at the top of every page that starts inside of a table, after the
/// box: the header rows of the table. The box takes room on such a page, in front of the content.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Repeat {
    /// The first item behind the header. A page that starts at an item after this one (behind the
    /// break that follows the header) repeats it.
    pub from: usize,
    /// The end of the table: a page that starts at this item or after it does not repeat it.
    pub to: usize,
    /// The extent of the header.
    pub node: NodeRef,
}

/// The ways to break a paragraph that a page can choose among (see
/// [`LayoutService::variants`](crate::LayoutService::variants)).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Variants {
    /// The number of lines more or fewer than the optimum that a paragraph can have: 0 asks for
    /// the way that the layout service breaks the paragraph, and nothing else, as the kernel
    /// has done without them. It is TeX's `\looseness`, for every paragraph, and the kernel
    /// chooses it.
    pub extra: u32,
    /// The worst badness that a line of a way to break a paragraph can have, except that of the
    /// first way, which the paragraph has anyway. TeX's `\tolerance`: 200.
    pub tolerance: f64,
    /// The badness that is worth a whole line of room, for the objectives that put the lines and
    /// the room of a column in one number ([`CssObjective::line_tolerance`](crate::CssObjective)).
    pub line_tolerance: f64,
}

impl Variants {
    /// TeX's `\tolerance`.
    pub const TOLERANCE: f64 = 1000.0;
}

impl Default for Variants {
    fn default() -> Self {
        Variants {
            extra: 0,
            tolerance: Self::TOLERANCE,
            line_tolerance: crate::CssObjective::LINE_TOLERANCE,
        }
    }
}

impl Problem {
    pub fn new(pages: PageTable, items: Vec<Item>) -> Self {
        Problem {
            pages,
            items,
            level: BreakLevel::Page,
            max_pages: None,
            start: Position::Item(0),
            fragment: false,
            page_breaks: false,
            variants: Variants::default(),
            float_fraction: Self::FLOAT_FRACTION,
            waiting: None,
            repeats: vec![],
        }
    }

    /// The header rows of the tables are repeated on the pages that go on inside of them.
    pub fn with_repeats(mut self, repeats: Vec<Repeat>) -> Self {
        self.repeats = repeats;
        self
    }

    /// The part of a page that floats take at the most, where there is text on the page: LaTeX's
    /// `\topfraction` (0.7) for the floats at the top and its `\bottomfraction` (0.3) for the
    /// ones at the bottom, which are not told apart here.
    pub const FLOAT_FRACTION: f32 = 0.7;

    /// The floats from the item `first` to the start wait for a page (see [`Problem::waiting`]).
    pub fn with_waiting(mut self, first: Option<usize>) -> Self {
        self.waiting = first;
        self
    }

    /// Floats take at most this part of a page that has text on it.
    pub fn with_float_fraction(mut self, fraction: f32) -> Self {
        self.float_fraction = fraction;
        self
    }

    /// A page chooses among the ways to break a paragraph that have up to `extra` lines more or
    /// fewer than the optimum, and lines that are not worse than `tolerance`.
    pub fn with_variants(mut self, extra: u32, tolerance: f64) -> Self {
        self.variants = Variants { extra, tolerance, ..self.variants };
        self
    }

    /// The exchange rate of the lines and the room of a column ([`Variants::line_tolerance`]).
    pub fn with_line_tolerance(mut self, line_tolerance: f64) -> Self {
        self.variants.line_tolerance = line_tolerance;
        self
    }

    /// The first page starts at this place.
    pub fn starting_at(mut self, start: Position) -> Self {
        self.start = start;
        self
    }

    /// The pages are the columns of a fragment of a container (see [`Problem::fragment`]). There
    /// must be a limit on the number of pages.
    pub fn as_fragment(mut self) -> Self {
        self.fragment = true;
        self.page_breaks = true;
        self
    }

    /// A forced break of the page level is a break in this problem of the columns of a container,
    /// too.
    pub fn with_page_breaks(mut self) -> Self {
        self.page_breaks = true;
        self
    }

    /// Whether a break ends a fragment: it is a forced break of the page level in the columns of
    /// a container that is split.
    pub fn ends_fragment(&self, kind: BreakKind, level: BreakLevel) -> bool {
        self.fragment && self.is_page_break(kind, level)
    }

    fn is_page_break(&self, kind: BreakKind, level: BreakLevel) -> bool {
        self.page_breaks && self.level == BreakLevel::Column && level == BreakLevel::Page && kind.is_forced()
    }

    /// The pages are fragmentainers of this level.
    pub fn with_level(mut self, level: BreakLevel) -> Self {
        self.level = level;
        self
    }

    /// There are at most `pages` pages (at least one).
    pub fn with_page_limit(mut self, pages: usize) -> Self {
        self.max_pages = Some(pages.max(1));
        self
    }

    /// The kind of a break of the level in this problem: a break of the other level is no break.
    /// The exception is a forced page break in the columns of a fragment: it is always a column
    /// break, too (https://www.w3.org/TR/css-break-3/#forced-breaks).
    pub fn kind_of(&self, kind: BreakKind, level: BreakLevel) -> BreakKind {
        if level == self.level || self.is_page_break(kind, level) {
            kind
        } else {
            BreakKind::Allowed
        }
    }
}
