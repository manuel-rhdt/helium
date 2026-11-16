use crate::token::Position;
use std::collections::HashMap;

/// The index of an item in the flow.
pub type ItemId = usize;

/// A box or a gap of the document, which the layout service knows the extent of.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct NodeRef(pub u32);

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
    /// A place where a page may end. The next page starts behind the item.
    Break { kind: BreakKind },
    /// A paragraph of the flow. Its lines are rigid: a page can only cut between the `lines`
    /// lines that the layout service has for it. There is no `Item` for a container that is
    /// split yet.
    Paragraph {
        id: ParId,
        lines: u32,
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

/// The geometry of the page that starts at a position.
#[derive(Debug, Clone)]
pub struct PageTable {
    /// Whether the pages are bound on the right: the first page is a left page.
    pub rtl: bool,
    /// The geometry of a page that is not in `starts`.
    pub default: PageSpec,
    /// The geometry of the page that starts at a position.
    pub starts: HashMap<Position, PageSpec>,
}

impl PageTable {
    /// All pages are the same.
    pub fn uniform(spec: PageSpec) -> Self {
        PageTable {
            rtl: false,
            default: spec,
            starts: HashMap::new(),
        }
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
    ///
    /// The table that the paginator has so far (step 1 of the plan) knows the geometry by the
    /// position alone. The kind is the one that the path to the position leads to, which for
    /// the paginator is guessed beforehand, so the table ignores `kind`. Because of this the
    /// state of the exact solver is the position only.
    pub fn spec(&self, at: Position, _kind: PageKind) -> PageSpec {
        self.starts.get(&at).copied().unwrap_or(self.default)
    }
}

/// What the kernel paginates.
#[derive(Debug, Clone)]
pub struct Problem {
    pub pages: PageTable,
    pub items: Vec<Item>,
    /// Whether the pages that [`Kernel::candidates`](crate::Kernel::candidates) lists end at
    /// the first forced break.
    ///
    /// The design says they do. The paginator that this one replaces does not stop there: a page
    /// may span a forced break when the pages on both sides of it do not fit. Until step 2 of
    /// the plan the paginator of helium sets this to `false` to keep its output.
    pub stop_at_forced: bool,
}

impl Problem {
    pub fn new(pages: PageTable, items: Vec<Item>) -> Self {
        Problem {
            pages,
            items,
            stop_at_forced: true,
        }
    }
}
