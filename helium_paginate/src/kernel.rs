use crate::flow::{BreakKind, Item, PageSpec, Problem};
use crate::service::LayoutService;
use crate::token::{BreakToken, Interner, Position};

/// A page that starts at a place of the flow.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PageStart {
    pub at: BreakToken,
    /// The page number, starting at 0.
    pub index: usize,
}

/// A page from its start to one of the places where it can end.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PageFill {
    /// Where the page ends, and the next one starts.
    pub end: BreakToken,
    pub spec: PageSpec,
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

/// The pages of a document: the place where each page starts, the first being the start of the
/// document. The last page ends at the end of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub pages: Vec<BreakToken>,
}

/// Why a [`Solution`] is not valid.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// The first page does not start at the start of the document.
    FirstPage,
    /// The page with this number cannot end where the next one starts (or at the end of the
    /// document, if it is the last).
    NotACandidate { page: usize },
}

/// A legal break of a paragraph, from one of its lines to the next.
#[derive(Debug, Copy, Clone)]
struct Cut {
    item: usize,
    /// The number of lines in front of the cut.
    line: u32,
    lines: u32,
    orphans: u32,
    widows: u32,
}

/// A place where a page can start or end.
#[derive(Debug, Copy, Clone)]
struct Legal {
    kind: BreakKind,
    cut: Option<Cut>,
}

/// The rules of fragmentation.
///
/// The places where a page can start or end are numbered in the order of the flow. The start
/// and the end of the document are among them.
pub struct Kernel<'a> {
    problem: &'a Problem,
    service: &'a mut dyn LayoutService,
    tokens: Interner,
    /// By the number of the token.
    legal: Vec<Legal>,
    /// For a `Break` item: the number of the token behind it, if the item is not the last.
    after_break: Vec<usize>,
    /// For a `Paragraph` item: the number of the token of its first cut.
    first_cut: Vec<usize>,
}

impl<'a> Kernel<'a> {
    pub fn new(problem: &'a Problem, service: &'a mut dyn LayoutService) -> Self {
        let items = &problem.items;
        let mut tokens = Interner::default();
        let mut legal = vec![];
        let mut after_break = vec![usize::MAX; items.len()];
        let mut first_cut = vec![usize::MAX; items.len()];
        tokens.intern(Position::Item(0));
        legal.push(Legal {
            kind: BreakKind::Allowed,
            cut: None,
        });
        for (item, &entry) in items.iter().enumerate() {
            match entry {
                // a break behind the last item is the end of the document
                Item::Break { kind } if item + 1 < items.len() => {
                    after_break[item] = legal.len();
                    tokens.intern(Position::Item(item + 1));
                    legal.push(Legal { kind, cut: None });
                }
                Item::Paragraph {
                    lines,
                    orphans,
                    widows,
                    avoid_inside,
                    ..
                } => {
                    first_cut[item] = legal.len();
                    let kind = if avoid_inside {
                        BreakKind::Avoid
                    } else {
                        BreakKind::Allowed
                    };
                    for line in 1..lines {
                        tokens.intern(Position::InParagraph { item, line });
                        let cut = Cut {
                            item,
                            line,
                            lines,
                            orphans,
                            widows,
                        };
                        legal.push(Legal {
                            kind,
                            cut: Some(cut),
                        });
                    }
                }
                _ => {}
            }
        }
        if items.is_empty() {
            // the start is the end
        } else {
            tokens.intern(Position::Item(items.len()));
            legal.push(Legal {
                kind: BreakKind::Allowed,
                cut: None,
            });
        }
        Kernel {
            problem,
            service,
            tokens,
            legal,
            after_break,
            first_cut,
        }
    }

    /// Every place where a page can start or end, in the order of the flow.
    pub fn tokens(&self) -> impl Iterator<Item = BreakToken> + use<> {
        (0..self.legal.len() as u32).map(BreakToken)
    }

    pub fn position(&self, token: BreakToken) -> Position {
        self.tokens.position(token)
    }

    pub fn token(&self, position: Position) -> Option<BreakToken> {
        self.tokens.token(position)
    }

    /// Whether the token is the end of the document.
    pub fn is_end(&self, token: BreakToken) -> bool {
        token.0 as usize + 1 == self.legal.len()
    }

    pub fn first_page(&self) -> PageStart {
        PageStart {
            at: BreakToken(0),
            index: 0,
        }
    }

    /// The page after `fill`, or `None` at the end of the document.
    ///
    /// (An empty page for `ForcedLeft` and `ForcedRight` where needed is not inserted yet.)
    pub fn next_page(&self, start: &PageStart, fill: &PageFill) -> Option<PageStart> {
        if self.is_end(fill.end) {
            None
        } else {
            Some(PageStart {
                at: fill.end,
                index: start.index + 1,
            })
        }
    }

    /// Every page that can start at `start`, by growing fill. Stops at the first item that does
    /// not fit, and at a forced break if the problem says so.
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
        let kind = problem.pages.kind(start.index);
        let spec = problem.pages.spec(position, kind);
        let mut fills = vec![];

        let (mut item, mut line, keep_gaps) = match position {
            Position::Item(item) => {
                let after_forced =
                    item > 0 && matches!(items[item - 1], Item::Break { kind } if kind.is_forced());
                (item, 0, item == 0 || after_forced)
            }
            Position::InParagraph { item, line } => (item, line, false),
        };
        let start_cut = self.legal[start.at.0 as usize].cut;

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
                Item::Break { .. } => {
                    item += 1;
                    if item < items.len() {
                        let token = self.after_break[item - 1];
                        if !self.emit(token, start_cut, spec, used, &mut first, &mut fills) {
                            return fills;
                        }
                    }
                }
                Item::Paragraph { id, lines, .. } => {
                    while line < lines {
                        used += pending + self.service.line_extent(id, line, spec.inline_size);
                        pending = 0.0;
                        leading = false;
                        line += 1;
                        if line < lines {
                            let token = self.first_cut[item] + line as usize - 1;
                            if !self.emit(token, start_cut, spec, used, &mut first, &mut fills) {
                                return fills;
                            }
                        }
                    }
                    line = 0;
                    item += 1;
                }
            }
        }
        let end = self.legal.len() - 1;
        if start.at.0 as usize != end {
            self.emit(end, start_cut, spec, used, &mut first, &mut fills);
        }
        fills
    }

    /// Adds the fill of the page that ends at the token. Returns whether to go on.
    fn emit(
        &self,
        end: usize,
        start_cut: Option<Cut>,
        spec: PageSpec,
        used: f64,
        first: &mut bool,
        fills: &mut Vec<PageFill>,
    ) -> bool {
        let legal = self.legal[end];
        let used = used as f32;
        let overflow = used > spec.block_size;
        // A page that is too full does not take the break after it, unless there is no break
        // in between: its content then gets a page of its own.
        if overflow && !*first {
            return false;
        }
        *first = false;
        let orphan_violation = legal.cut.is_some_and(|end| {
            let start = start_cut
                .filter(|start| start.item == end.item)
                .map_or(0, |start| start.line);
            end.line - start < end.orphans
        });
        let widow_violation = start_cut.is_some_and(|start| {
            let end = legal
                .cut
                .filter(|end| end.item == start.item)
                .map_or(start.lines, |end| end.line);
            end - start.line < start.widows
        });
        fills.push(PageFill {
            end: BreakToken(end as u32),
            spec,
            used,
            end_kind: legal.kind,
            orphan_violation,
            widow_violation,
            overflow,
        });
        !(legal.kind.is_forced() && self.problem.stop_at_forced)
    }

    /// Checks a solution and returns its pages.
    pub fn realize(&mut self, solution: &Solution) -> Result<Vec<PageFill>, Invalid> {
        let first = self.first_page();
        if solution.pages.first() != Some(&first.at) {
            return Err(Invalid::FirstPage);
        }
        let mut pages = vec![];
        if self.is_end(first.at) {
            // nothing to paginate
            return Ok(pages);
        }
        let end = BreakToken(self.legal.len() as u32 - 1);
        for (page, &at) in solution.pages.iter().enumerate() {
            let expected = solution.pages.get(page + 1).copied().unwrap_or(end);
            let start = PageStart { at, index: page };
            let fill = self
                .candidates(&start)
                .into_iter()
                .find(|fill| fill.end == expected)
                .ok_or(Invalid::NotACandidate { page })?;
            pages.push(fill);
        }
        Ok(pages)
    }
}
