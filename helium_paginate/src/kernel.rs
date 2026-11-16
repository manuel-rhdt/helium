use crate::flow::{BreakKind, Item, PageKind, PageSpec, Problem};
use crate::service::LayoutService;
use crate::token::{BreakToken, Interner, Position};

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

/// The pages of a document: the place where each page starts and its number, the first being the
/// start of the document. The last page ends at the end of the document. A number that is
/// missing is an empty page that a break to a left or right page has made (see
/// [`Kernel::next_page`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub pages: Vec<PageStart>,
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
}

/// How an end of a page is treated.
#[derive(Debug, Copy, Clone)]
struct Legal {
    kind: BreakKind,
    /// The lines of the paragraph that the page has in front of the cut, and the number that
    /// `orphans` asks for.
    cut: Option<(u32, u32)>,
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
        let left_right_matter = problem.pages.left_right_differ
            || items.iter().any(|item| {
                matches!(item, Item::Break { kind: BreakKind::ForcedLeft | BreakKind::ForcedRight })
            });
        Kernel {
            problem,
            service,
            tokens,
            left_right_matter,
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
            at: BreakToken(0),
            index: 0,
        }
    }

    /// The kind of the page with the number `index`.
    pub fn kind(&self, index: usize) -> PageKind {
        self.problem.pages.kind(index)
    }

    /// What a search has to tell apart about a page: the place where it starts and its kind.
    /// Left and right pages are the same if nothing depends on which one they are.
    pub fn state(&self, start: &PageStart) -> (BreakToken, PageKind) {
        let kind = self.problem.pages.kind(start.index);
        if kind != PageKind::First && !self.left_right_matter {
            (start.at, PageKind::Right)
        } else {
            (start.at, kind)
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
    ///   that can overflow: later ones are not listed).
    pub fn candidates(&mut self, start: &PageStart) -> Vec<PageFill> {
        let problem = self.problem;
        let items = &problem.items;
        let position = self.tokens.position(start.at);
        let page_kind = problem.pages.kind(start.index);
        let spec = problem.pages.spec(position, page_kind);
        let mut fills = vec![];

        let (mut item, mut cut, keep_gaps) = match position {
            Position::Item(item) => {
                let after_forced =
                    item > 0 && matches!(items[item - 1], Item::Break { kind } if kind.is_forced());
                (item, None, item == 0 || after_forced)
            }
            Position::InParagraph { item, cut } => (item, Some(cut), false),
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
                Item::Break { kind } => {
                    item += 1;
                    if item < items.len() {
                        let token = self.tokens.intern(Position::Item(item));
                        let legal = Legal { kind, cut: None };
                        let widows = widow_violation(start_paragraph, None);
                        if !self.emit(token, legal, widows, spec, page_kind, used, &mut first, &mut fills) {
                            return fills;
                        }
                    }
                }
                Item::Paragraph {
                    id,
                    orphans,
                    avoid_inside,
                    ..
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
                        if number + 1 < lines.len() {
                            let on_page = number as u32 + 1;
                            let token = self.tokens.intern(Position::InParagraph {
                                item,
                                cut: line.end,
                            });
                            let legal = Legal {
                                kind: end_kind,
                                cut: Some((on_page, orphans)),
                            };
                            let widows = if in_start {
                                widow_violation(start_paragraph, Some(on_page))
                            } else {
                                widow_violation(start_paragraph, None)
                            };
                            if !self.emit(token, legal, widows, spec, page_kind, used, &mut first, &mut fills) {
                                return fills;
                            }
                        }
                    }
                    if let Some(start) = start_paragraph.as_mut().filter(|_| in_start) {
                        start.tail = Some(lines.len() as u32);
                    }
                    item += 1;
                }
            }
        }
        let end = self.tokens.intern(Position::Item(items.len()));
        if start.at != end {
            let legal = Legal {
                kind: BreakKind::Allowed,
                cut: None,
            };
            let widows = widow_violation(start_paragraph, None);
            self.emit(end, legal, widows, spec, page_kind, used, &mut first, &mut fills);
        }
        fills
    }

    /// Adds the fill of the page that ends at the token. Returns whether to go on.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        end: BreakToken,
        legal: Legal,
        widow_violation: bool,
        spec: PageSpec,
        kind: PageKind,
        used: f64,
        first: &mut bool,
        fills: &mut Vec<PageFill>,
    ) -> bool {
        let used = used as f32;
        let overflow = used > spec.block_size;
        // A page that is too full does not take the break after it, unless there is no break
        // in between: its content then gets a page of its own.
        if overflow && !*first {
            return false;
        }
        *first = false;
        let orphan_violation = legal
            .cut
            .is_some_and(|(on_page, orphans)| on_page < orphans);
        fills.push(PageFill {
            end,
            spec,
            kind,
            used,
            end_kind: legal.kind,
            orphan_violation,
            widow_violation,
            overflow,
        });
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
        let end = self.tokens.intern(Position::Item(self.problem.items.len()));
        for (page, &start) in solution.pages.iter().enumerate() {
            let fills = self.candidates(&start);
            let next = solution.pages.get(page + 1);
            let expected = next.map_or(end, |next| next.at);
            let fill = fills
                .into_iter()
                .find(|fill| fill.end == expected)
                .ok_or(Invalid::NotACandidate { page })?;
            if self.next_page(&start, &fill) != next.copied() {
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
