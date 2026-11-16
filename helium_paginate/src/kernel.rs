use crate::flow::{BoxRef, BreakKind, Item, PageKind, PageSpec, Problem};
use crate::service::LayoutService;
use crate::token::{BreakToken, ContainerCut, Interner, Position};

/// A page that starts at a place of the flow.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PageStart {
    pub at: BreakToken,
    /// The page number, starting at 0. It gives the kind of the page.
    pub index: usize,
}

/// A page from its start to one of the places where it can end.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PageFill {
    /// Where the page ends, and the next one starts.
    pub end: BreakToken,
    pub spec: PageSpec,
    /// The kind of the page.
    pub kind: PageKind,
    /// The extent of the content of the page.
    pub used: f32,
    pub end_kind: BreakKind,
    /// The page has fewer lines of the paragraph that it ends in than `orphans`.
    pub orphan_violation: bool,
    /// The page has fewer lines of the paragraph that it starts in than `widows`.
    pub widow_violation: bool,
    /// A single unbreakable item is taller than the page.
    pub overflow: bool,
}

impl PageFill {
    /// Whether the page does not fit, ends where a break should be avoided, or has too few lines
    /// of a paragraph at its start or end.
    pub fn breaks_a_rule(&self) -> bool {
        self.overflow
            || self.end_kind == BreakKind::Avoid
            || self.orphan_violation
            || self.widow_violation
    }
}

/// A fragment of a container that is on a page: what the layout of the page needs to know to lay
/// out the container. The kernel asked the layout service for it when it listed the page
/// ([`Kernel::fragments`]).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PageFragment {
    pub container: BoxRef,
    /// Where the fragment starts in the container: `None` at its start.
    pub from: Option<ContainerCut>,
    /// The inline size of the page and the height that was left on it for the fragment.
    pub page_inline: f32,
    pub available: f64,
    /// Where the next fragment starts: `None` if the container ends in this one.
    pub end: Option<ContainerCut>,
}

/// The pages of a document: the place where each page starts and its number, the first being the
/// start of the document. The last page ends at the end of the document. A number that is
/// missing is an empty page that a break to a left or right page has made (see
/// [`Kernel::next_page`]).
///
/// The problem of a fragment of a container ([`Problem::fragment`]) does not end at the end of
/// the document: its solution has one more entry, which is not a page but the place where the
/// last page ends, as the start of the page that would follow it ([`Solution::fragment_end`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub pages: Vec<PageStart>,
}

impl Solution {
    /// The pages of a fragment of a container, and where the last of them ends.
    pub fn fragment_end(&self) -> Option<(&[PageStart], PageStart)> {
        self.pages.split_last().map(|(end, pages)| (pages, *end))
    }
}

/// Why a [`Solution`] is not valid.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// The first page does not start at the start of the document.
    FirstPage,
    /// The page with this number cannot end where the next one starts (or at the end of the
    /// document, if it is the last), or the next one does not have the number that it has to.
    NotACandidate { page: usize },
}

/// The rules of fragmentation.
///
/// The places where a page can start or end are the start and the end of the document, the
/// breaks between items and the cuts of paragraphs. The cuts of a paragraph are only known to
/// the layout service and depend on the width of the page, so they are numbered when they are
/// first asked for.
pub struct Kernel<'a> {
    problem: &'a Problem,
    service: &'a mut dyn LayoutService,
    tokens: Interner,
    left_right_matter: bool,
    /// The start of the first page.
    first: BreakToken,
}

/// How an end of a page is treated.
#[derive(Debug, Copy, Clone)]
struct Legal {
    kind: BreakKind,
    /// The lines of the paragraph that the page has in front of the cut, and the number that
    /// `orphans` asks for.
    cut: Option<(u32, u32)>,
    /// Whether a fragment of a container on the page broke the rules that it has inside.
    orphans: bool,
    widows: bool,
}

impl Legal {
    fn new(kind: BreakKind, cut: Option<(u32, u32)>) -> Self {
        Legal { kind, cut, orphans: false, widows: false }
    }
}

/// The pages that can start at a place, and with the fragments of containers that are on each.
type Fills = (Vec<PageFill>, Vec<Vec<PageFragment>>);

/// A place where the page that is walked can end.
#[derive(Debug, Copy, Clone)]
struct Found {
    token: BreakToken,
    legal: Legal,
    widows: bool,
    spec: PageSpec,
    kind: PageKind,
}

/// What the walk through a page knows about the paragraph that the page starts in.
#[derive(Debug, Copy, Clone)]
struct StartParagraph {
    item: usize,
    widows: u32,
    /// The number of the lines from the start of the page to the end of the paragraph, known
    /// when the walk is behind the paragraph.
    tail: Option<u32>,
}

impl<'a> Kernel<'a> {
    pub fn new(problem: &'a Problem, service: &'a mut dyn LayoutService) -> Self {
        let items = &problem.items;
        let mut tokens = Interner::default();
        tokens.intern(Position::Item(0));
        for (item, &entry) in items.iter().enumerate() {
            // a break behind the last item is the end of the document
            if matches!(entry, Item::Break { .. }) && item + 1 < items.len() {
                tokens.intern(Position::Item(item + 1));
            }
        }
        tokens.intern(Position::Item(items.len()));
        let first = tokens.intern(problem.start);
        let left_right_matter = problem.pages.left_right_differ
            || items.iter().any(|item| {
                matches!(
                    item,
                    Item::Break { kind: BreakKind::ForcedLeft | BreakKind::ForcedRight, level }
                        if *level == problem.level
                )
            });
        Kernel {
            problem,
            service,
            tokens,
            left_right_matter,
            first,
        }
    }

    /// Every place where a page can start or end that is known so far: the start and the end of
    /// the document, the breaks between items, and the cuts that have been asked for. In no
    /// particular order.
    pub fn tokens(&self) -> impl Iterator<Item = BreakToken> + use<> {
        (0..self.tokens.len() as u32).map(BreakToken)
    }

    pub fn position(&self, token: BreakToken) -> Position {
        self.tokens.position(token)
    }

    /// The token of a position, which is made if there is none yet.
    pub fn intern(&mut self, position: Position) -> BreakToken {
        self.tokens.intern(position)
    }

    pub fn token(&self, position: Position) -> Option<BreakToken> {
        self.tokens.token(position)
    }

    /// Whether the token is the end of the document.
    pub fn is_end(&self, token: BreakToken) -> bool {
        self.tokens.position(token) == Position::Item(self.problem.items.len())
    }

    pub fn first_page(&self) -> PageStart {
        PageStart {
            at: self.first,
            index: 0,
        }
    }

    /// Whether the problem is a fragment of a container.
    pub fn is_fragment(&self) -> bool {
        self.problem.fragment
    }

    /// Whether nothing follows the page that starts here: the end of the document, or in the
    /// problem of a fragment of a container the end of the last page that there can be, or a
    /// forced break of the page level.
    pub fn is_final(&self, start: &PageStart) -> bool {
        if self.is_end(start.at) {
            return true;
        }
        let problem = self.problem;
        if !problem.fragment || start.index == 0 {
            return false;
        }
        if problem.max_pages.is_some_and(|max| start.index >= max) {
            return true;
        }
        match self.tokens.position(start.at) {
            Position::Item(item) if item > 0 => {
                matches!(problem.items[item - 1], Item::Break { kind, level } if problem.ends_fragment(kind, level))
            }
            _ => false,
        }
    }

    /// The kind of the page with the number `index`.
    pub fn kind(&self, index: usize) -> PageKind {
        self.problem.pages.kind(index)
    }

    /// What a search has to tell apart about a page: the place where it starts and its kind.
    /// Left and right pages are the same if nothing depends on which one they are. If the number
    /// of pages is limited, the number of the page is part of it (see [`Kernel::slot`]).
    pub fn state(&self, start: &PageStart) -> (BreakToken, PageKind) {
        let kind = self.problem.pages.kind(start.index);
        if kind != PageKind::First && !self.left_right_matter {
            (start.at, PageKind::Right)
        } else {
            (start.at, kind)
        }
    }

    /// What else a search has to tell apart about a page: its number, if the number of pages is
    /// limited (the pages that are left depend on it), else nothing.
    pub fn slot(&self, start: &PageStart) -> usize {
        if self.problem.max_pages.is_some() {
            start.index
        } else {
            0
        }
    }

    /// The page after `fill`, or `None` at the end of the document.
    ///
    /// A break to a left or a right page leaves a page empty if the next page would be of the
    /// other kind: the number of the next page is one higher then.
    /// https://www.w3.org/TR/css-break-3/#forced-breaks
    pub fn next_page(&self, start: &PageStart, fill: &PageFill) -> Option<PageStart> {
        if self.is_end(fill.end) {
            return None;
        }
        let mut index = start.index + 1;
        let wanted = match fill.end_kind {
            BreakKind::ForcedLeft => Some(PageKind::Left),
            BreakKind::ForcedRight => Some(PageKind::Right),
            _ => None,
        };
        if wanted.is_some_and(|wanted| self.problem.pages.kind(index) != wanted) {
            index += 1;
        }
        Some(PageStart {
            at: fill.end,
            index,
        })
    }

    /// Every page that can start at `start`, by growing fill. Stops at the first item that does
    /// not fit, and at a forced break.
    ///
    /// All rules of fragmentation are in this function:
    ///
    /// - gaps at the start and the end of a page are discarded, except after a forced break
    ///   and at the start of the document;
    /// - the lines of a paragraph on the page are counted against `orphans` and `widows`;
    /// - an unbreakable item that is taller than the page gets a page of its own and overflows
    ///   it, so that the content after it is not lost (the first fill, which is the only one
    ///   that can overflow: later ones are not listed);
    /// - a container is asked for the fragment that fits in what is left of the page, and the
    ///   page can end inside it where that fragment does.
    pub fn candidates(&mut self, start: &PageStart) -> Vec<PageFill> {
        self.walk(start, false).0
    }

    /// The fragments of containers that are on the page that starts at `start` and ends at `end`,
    /// in order. They were asked of the layout service when the page was listed, and the same
    /// arguments give the same fragments.
    pub fn fragments(&mut self, start: &PageStart, end: BreakToken) -> Vec<PageFragment> {
        let (fills, mut lists) = self.walk(start, true);
        match fills.iter().position(|fill| fill.end == end) {
            Some(index) => lists.swap_remove(index),
            None => vec![],
        }
    }

    /// The pages that can start at `start`, and with `record` the fragments of the containers
    /// that are on each.
    fn walk(&mut self, start: &PageStart, record: bool) -> Fills {
        let problem = self.problem;
        let items = &problem.items;
        let position = self.tokens.position(start.at);
        let page_kind = problem.pages.kind(start.index);
        let spec = problem.pages.spec(position, page_kind);
        let mut fills: Fills = (vec![], vec![]);
        // The last page that there can be takes the rest of the content, whatever breaks it has,
        // unless it is the last page of a fragment of a container.
        let last_page = problem.max_pages.is_some_and(|max| start.index + 1 >= max);
        let (take_rest, ends_fragment) = (last_page && !problem.fragment, last_page && problem.fragment);

        let (mut item, mut cut, keep_gaps) = match position {
            Position::Item(item) => {
                let after_forced = item > 0
                    && matches!(items[item - 1], Item::Break { kind, level } if problem.kind_of(kind, level).is_forced());
                (item, None, item == 0 || after_forced)
            }
            Position::InParagraph { item, cut } => (item, Some(cut), false),
            Position::InContainer { item, .. } => (item, None, false),
        };
        let mut container_from = match position {
            Position::InContainer { cut, .. } => Some(cut),
            _ => None,
        };
        let mut start_paragraph = match (position, items.get(item)) {
            (Position::InParagraph { .. }, Some(&Item::Paragraph { widows, .. })) => {
                Some(StartParagraph {
                    item,
                    widows,
                    tail: None,
                })
            }
            _ => None,
        };

        // The content of the page up to the last box or line, and the gaps after it, which are
        // only part of the page if more content follows. `leading` is whether gaps are still
        // discarded.
        let mut used = 0.0;
        let mut pending = 0.0;
        let mut leading = !keep_gaps;
        let mut first = true;
        // the fragments of containers on the page, and what they broke inside
        let mut on_page: Vec<PageFragment> = vec![];
        let (mut broken_orphans, mut broken_widows) = (false, false);

        while item < items.len() {
            match items[item] {
                Item::Box { node } => {
                    used += pending + self.service.extent(node, spec.inline_size);
                    pending = 0.0;
                    leading = false;
                    item += 1;
                }
                Item::Gap { node } => {
                    if !leading {
                        pending += self.service.extent(node, spec.inline_size);
                    }
                    item += 1;
                }
                Item::Break { kind, level } => {
                    let kind = problem.kind_of(kind, level);
                    item += 1;
                    if item < items.len() && !take_rest {
                        let token = self.tokens.intern(Position::Item(item));
                        let legal = Legal { orphans: broken_orphans, widows: broken_widows, ..Legal::new(kind, None) };
                        let widows = widow_violation(start_paragraph, None);
                        let found = Found { token, legal, widows, spec, kind: page_kind };
                        if !self.emit(record, found, used, &mut first, &on_page, &mut fills) {
                            return fills;
                        }
                    }
                }
                Item::Paragraph {
                    id,
                    orphans,
                    widows: widows_wanted,
                    avoid_inside,
                } => {
                    let lines = self.service.lines(id, cut.take(), spec.inline_size);
                    let in_start = start_paragraph.is_some_and(|start| start.item == item);
                    let end_kind = if avoid_inside {
                        BreakKind::Avoid
                    } else {
                        BreakKind::Allowed
                    };
                    for (number, line) in lines.iter().enumerate() {
                        used += pending + line.extent;
                        pending = 0.0;
                        leading = false;
                        if number + 1 < lines.len() && !take_rest {
                            let lines_on_page = number as u32 + 1;
                            let token = self.tokens.intern(Position::InParagraph {
                                item,
                                cut: line.end,
                            });
                            let legal = Legal {
                                orphans: broken_orphans,
                                widows: broken_widows,
                                ..Legal::new(end_kind, Some((lines_on_page, orphans)))
                            };
                            let mut widows = if in_start {
                                widow_violation(start_paragraph, Some(lines_on_page))
                            } else {
                                widow_violation(start_paragraph, None)
                            };
                            // The lines that the next fragment of the container starts with are
                            // the rest of the paragraph.
                            if ends_fragment && ((lines.len() - lines_on_page as usize) as u32) < widows_wanted {
                                widows = true;
                            }
                            let found = Found { token, legal, widows, spec, kind: page_kind };
                            if !self.emit(record, found, used, &mut first, &on_page, &mut fills) {
                                return fills;
                            }
                        }
                    }
                    if let Some(start) = start_paragraph.as_mut().filter(|_| in_start) {
                        start.tail = Some(lines.len() as u32);
                    }
                    item += 1;
                }
                Item::Container { node, avoid_inside } => {
                    let from = container_from.take();
                    let available = f64::from(spec.block_size) - used - pending;
                    let fragments = self.service.fragment(node, from, spec.inline_size, available);
                    let mut ended = None;
                    for fragment in fragments.iter() {
                        let page_fragment = PageFragment {
                            container: node,
                            from,
                            page_inline: spec.inline_size,
                            available,
                            end: fragment.end,
                        };
                        let Some(end) = fragment.end else {
                            ended = Some((*fragment, page_fragment));
                            continue;
                        };
                        // the page ends inside the container
                        let token = self.tokens.intern(Position::InContainer { item, cut: end });
                        let kind = if avoid_inside && !fragment.end_kind.is_forced() {
                            BreakKind::Avoid
                        } else {
                            fragment.end_kind
                        };
                        let legal = Legal {
                            kind,
                            cut: None,
                            orphans: broken_orphans || fragment.orphan_violation || fragment.avoid_violation,
                            widows: broken_widows || fragment.widow_violation,
                        };
                        let found = Found { token, legal, widows: false, spec, kind: page_kind };
                        let mut here = on_page.clone();
                        if record {
                            here.push(page_fragment);
                        }
                        if !self.emit(record, found, used + pending + fragment.used, &mut first, &here, &mut fills) {
                            return fills;
                        }
                    }
                    // The page cannot go on behind a container that continues on the next one.
                    let Some((fragment, page_fragment)) = ended else {
                        return fills;
                    };
                    used += pending + fragment.used;
                    pending = 0.0;
                    leading = false;
                    broken_orphans |= fragment.orphan_violation || fragment.avoid_violation;
                    broken_widows |= fragment.widow_violation;
                    if record {
                        on_page.push(page_fragment);
                    }
                    item += 1;
                }
            }
        }
        let end = self.tokens.intern(Position::Item(items.len()));
        if start.at != end {
            let legal = Legal { orphans: broken_orphans, widows: broken_widows, ..Legal::new(BreakKind::Allowed, None) };
            let widows = widow_violation(start_paragraph, None);
            let found = Found { token: end, legal, widows, spec, kind: page_kind };
            self.emit(record, found, used, &mut first, &on_page, &mut fills);
        }
        fills
    }

    /// Adds the fill of the page that ends at the token. Returns whether to go on.
    fn emit(
        &self,
        record: bool,
        found: Found,
        used: f64,
        first: &mut bool,
        fragments: &[PageFragment],
        fills: &mut Fills,
    ) -> bool {
        let Found { token: end, legal, widows, spec, kind } = found;
        let used = used as f32;
        let overflow = used > spec.block_size;
        // A page that is too full does not take the break after it, unless there is no break
        // in between: its content then gets a page of its own.
        if overflow && !*first {
            return false;
        }
        *first = false;
        let orphan_violation = legal.orphans
            || legal
                .cut
                .is_some_and(|(on_page, orphans)| on_page < orphans);
        fills.0.push(PageFill {
            end,
            spec,
            kind,
            used,
            end_kind: legal.kind,
            orphan_violation,
            widow_violation: widows || legal.widows,
            overflow,
        });
        // (the fragments are only recorded if there are some to ask for)
        if record {
            fills.1.push(fragments.to_vec());
        }
        !legal.kind.is_forced()
    }

    /// Checks a solution and returns its pages.
    pub fn realize(&mut self, solution: &Solution) -> Result<Vec<PageFill>, Invalid> {
        let first = self.first_page();
        if solution.pages.first() != Some(&first) {
            return Err(Invalid::FirstPage);
        }
        let mut pages = vec![];
        if self.is_end(first.at) {
            // nothing to paginate
            return Ok(pages);
        }
        // the last entry of the solution of a fragment is where the last page ends
        let (starts, last_end) = match solution.fragment_end().filter(|_| self.problem.fragment) {
            Some((starts, end)) => (starts, Some(end)),
            None => (&solution.pages[..], None),
        };
        let end = self.tokens.intern(Position::Item(self.problem.items.len()));
        for (page, &start) in starts.iter().enumerate() {
            let fills = self.candidates(&start);
            let next = starts.get(page + 1).or(last_end.as_ref());
            let expected = next.map_or(end, |next| next.at);
            let fill = fills
                .into_iter()
                .find(|fill| fill.end == expected)
                .ok_or(Invalid::NotACandidate { page })?;
            let valid = match next {
                // (the end of a fragment is not the end of the document)
                Some(next) if page + 1 == starts.len() => {
                    self.is_end(fill.end) || self.next_page(&start, &fill).is_some_and(|after| after.at == next.at)
                }
                _ => self.next_page(&start, &fill) == next.copied(),
            };
            if !valid {
                return Err(Invalid::NotACandidate { page });
            }
            pages.push(fill);
        }
        Ok(pages)
    }
}

/// Whether the page has fewer lines of the paragraph that it starts in than `widows`, where the
/// page ends after `on_page` lines of that paragraph, or not in it.
fn widow_violation(start: Option<StartParagraph>, on_page: Option<u32>) -> bool {
    start.is_some_and(|start| {
        on_page
            .or(start.tail)
            .is_some_and(|lines| lines < start.widows)
    })
}
