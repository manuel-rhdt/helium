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
    /// paragraph starts on it. There is no `Item` for a container that is split yet.
    Paragraph {
        id: ParId,
        orphans: u32,
        widows: u32,
        /// Whether the breaks between the lines are `BreakKind::Avoid`.
        avoid_inside: bool,
    },
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
}

impl Problem {
    pub fn new(pages: PageTable, items: Vec<Item>) -> Self {
        Problem {
            pages,
            items,
            level: BreakLevel::Page,
            max_pages: None,
        }
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
    pub fn kind_of(&self, kind: BreakKind, level: BreakLevel) -> BreakKind {
        if level == self.level {
            kind
        } else {
            BreakKind::Allowed
        }
    }
}
