use crate::flow::{BoxRef, BreakKind, Item, PageKind, PageSpec, Problem};
use crate::service::LayoutService;
use crate::token::{BreakToken, ContainerCut, Cut, Interner, Position};
use std::collections::HashMap;
use std::rc::Rc;

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
    /// The demerits of the lines of the paragraphs on the page, where the page chooses among ways
    /// to break them ([`Variants`](crate::Variants)), and 0 where it does not. A page that has a
    /// part of a paragraph has the demerits of those lines.
    pub line_demerits: f64,
    /// The mean extent of the lines whose demerits are counted.
    pub line_extent: f32,
    /// The ways to break the paragraphs on the page that are not the first
    /// ([`Kernel::choices`]).
    pub choices: ChoiceId,
}

/// The way that a paragraph on a page is broken, if it is not the first of the ways that the
/// layout service gives ([`LayoutService::variants`](crate::LayoutService::variants)): the item of
/// the flow and the number in the list. The list is the one for the paragraph from the cut that
/// the page starts at, if it starts in the paragraph, and from its start if not.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Choice {
    pub item: usize,
    pub variant: usize,
}

/// A list of [`Choice`]s, which the kernel keeps. The first way for every paragraph is
/// [`ChoiceId::NONE`].
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub struct ChoiceId(pub u32);

impl ChoiceId {
    pub const NONE: ChoiceId = ChoiceId(0);
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
    /// The way to break the paragraphs of every page, if it is not the first way for all of them,
    /// by the number of the page ([`PageFill::choices`]). Empty if it is.
    pub choices: Vec<ChoiceId>,
}

impl Solution {
    /// The pages, with the first way to break every paragraph.
    pub fn new(pages: Vec<PageStart>) -> Self {
        Solution { pages, choices: vec![] }
    }

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
    /// While a page that ends at any cut of a paragraph is listed ([`Kernel::fill_at`]): the
    /// paragraph and the cut.
    force_end: Option<(usize, Cut)>,
    /// The lists of choices that pages have, by their number; the first is the empty one.
    choices: Vec<Rc<[Choice]>>,
    choice_ids: HashMap<Vec<Choice>, ChoiceId>,
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
    /// Whether the lines of the paragraph that the next fragment of a container starts with are
    /// fewer than `widows`.
    widows: bool,
    spec: PageSpec,
    kind: PageKind,
}

/// What the walk through a page knows about the paragraph that the page starts in.
#[derive(Debug, Copy, Clone)]
struct StartParagraph {
    item: usize,
    widows: u32,
}

/// A way to break the paragraphs on the page that is walked, so far.
#[derive(Debug, Clone, Default)]
struct Label {
    /// The extent of the content of the page up to the last box or line.
    used: f64,
    /// The demerits of the lines that are on the page.
    demerits: f64,
    /// The demerits that the same lines have in the first way to break their paragraphs (line by
    /// line, and none for the lines that it does not have): what the page has anyway. Only what
    /// the lines have above it is charged, or a page would cost more the more lines it has.
    baseline: f64,
    /// The extent and the number of the lines whose demerits are counted.
    line_extent: f64,
    lines: u32,
    /// The ways to break the paragraphs on the page that are not the first.
    choices: Vec<Choice>,
    /// The number of the lines from the start of the page to the end of the paragraph that the
    /// page starts in, known when the walk is behind the paragraph.
    tail: Option<u32>,
}

impl Label {
    /// The demerits of the lines above those of the first ways to break their paragraphs.
    fn excess(&self) -> f64 {
        (self.demerits - self.baseline).max(0.0)
    }
}

/// What listing a page that ends at a place did.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Emit {
    Go,
    /// The way is too full to end at a later place. The others may not be.
    Dropped,
    /// The page cannot go on behind this place for any way.
    Stop,
}

/// The labels that cost the least where the room that they use is the same.
fn prune(labels: Vec<Label>) -> Vec<Label> {
    if labels.len() <= 1 {
        return labels;
    }
    let key = |label: &Label| ((label.used * 64.0).round() as i64, label.tail);
    let mut kept: Vec<Label> = vec![];
    for label in labels {
        match kept.iter_mut().find(|other| key(other) == key(&label)) {
            Some(other) => {
                if label.excess() < other.excess() {
                    *other = label;
                }
            }
            None => kept.push(label),
        }
    }
    kept
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
            force_end: None,
            choices: vec![Rc::from(vec![])],
            choice_ids: HashMap::from([(vec![], ChoiceId::NONE)]),
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

    /// The page that starts at `start` and ends in the paragraph `item` at the cut `to`, which can
    /// be any place where the paragraph can be broken. [`candidates`](Self::candidates) lists
    /// the pages that end where a line of the paragraph ends as the layout service breaks it for
    /// the page; this is the page that a solver gets that wants to break the paragraph somewhere
    /// else. The service breaks the part of the paragraph that is on the page to end at the cut
    /// ([`LayoutService::lines_to`]), and the end of the page is the cut as the service makes it
    /// (`fill.end`), which is not necessarily `to`.
    ///
    /// `None` if the page is not possible: its content does not fit on it, or `item` is not a
    /// paragraph at or after the start, or it is on a page of its own further on.
    pub fn fill_at(&mut self, start: &PageStart, item: usize, to: Cut) -> Option<PageFill> {
        if !matches!(self.problem.items.get(item), Some(Item::Paragraph { .. })) {
            return None;
        }
        self.force_end = Some((item, to));
        let fills = self.walk(start, false).0;
        self.force_end = None;
        fills.into_iter().last().filter(|fill| {
            matches!(self.tokens.position(fill.end), Position::InParagraph { item: found, .. } if found == item)
        })
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
    ///
    /// A page that chooses among ways to break paragraphs ([`Problem::variants`]) is walked with
    /// a label for every way that its content so far has, which are as many as the rooms that
    /// they use: a page is listed for every one of them.
    fn walk(&mut self, start: &PageStart, record: bool) -> Fills {
        let problem = self.problem;
        let items = &problem.items;
        let position = self.tokens.position(start.at);
        let page_kind = problem.pages.kind(start.index);
        let spec = problem.pages.spec(position, page_kind);
        let mut fills: Fills = (vec![], vec![]);
        // (a page that ends at a cut that was asked for is listed for the first way only)
        let variants = if self.force_end.is_some() { crate::Variants::default() } else { problem.variants };
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
        let start_paragraph = match (position, items.get(item)) {
            (Position::InParagraph { .. }, Some(&Item::Paragraph { widows, .. })) => {
                Some(StartParagraph { item, widows })
            }
            _ => None,
        };

        // The content of the page up to the last box or line, for every way to break the
        // paragraphs on it, and the gaps after it, which are only part of the page if more content
        // follows. `leading` is whether gaps are still discarded.
        let mut labels = vec![Label::default()];
        let mut pending = 0.0;
        let mut leading = !keep_gaps;
        let mut first = true;
        // the fragments of containers on the page, and what they broke inside
        let mut on_page: Vec<PageFragment> = vec![];
        let (mut broken_orphans, mut broken_widows) = (false, false);

        while item < items.len() {
            match items[item] {
                Item::Box { node } => {
                    let extent = pending + self.service.extent(node, spec.inline_size);
                    for label in &mut labels {
                        label.used += extent;
                    }
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
                        let found = Found { token, legal, widows: false, spec, kind: page_kind };
                        let mut stop = false;
                        labels.retain(|label| {
                            match self.emit(record, &found, label, 0.0, None, start_paragraph, &mut first, &on_page, &mut fills) {
                                Emit::Go => true,
                                Emit::Dropped => false,
                                Emit::Stop => {
                                    stop = true;
                                    true
                                }
                            }
                        });
                        if stop || labels.is_empty() {
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
                    // (the paragraph that the page is to end in, at a cut that the service may not
                    // have among the ends of its lines)
                    let forced_end = self.force_end.filter(|&(forced, _)| forced == item).map(|(_, to)| to);
                    let from_cut = cut.take();
                    // the ways to break the paragraph on this page, with their numbers in the list
                    // that the service has
                    let ways: Vec<(usize, Rc<[crate::ParLine]>)> = match forced_end {
                        Some(to) => vec![(0, self.service.lines_to(id, from_cut, to, spec.inline_size))],
                        None if variants.extra == 0 => vec![(0, self.service.lines(id, from_cut, spec.inline_size))],
                        None => self
                            .service
                            .variants(id, from_cut, spec.inline_size, variants.extra)
                            .into_iter()
                            .enumerate()
                            .filter(|(number, lines)| {
                                *number == 0 || lines.iter().all(|line| line.badness <= variants.tolerance)
                            })
                            .collect(),
                    };
                    let count_demerits = variants.extra > 0;
                    let in_start = start_paragraph.is_some_and(|start| start.item == item);
                    let end_kind = if avoid_inside {
                        BreakKind::Avoid
                    } else {
                        BreakKind::Allowed
                    };
                    let mut behind: Vec<Label> = vec![];
                    for label in &labels {
                        for (way, lines) in &ways {
                            let mut here = label.clone();
                            if *way > 0 {
                                here.choices.push(Choice { item, variant: *way });
                            }
                            let mut ahead = pending;
                            let mut alive = true;
                            for (number, line) in lines.iter().enumerate() {
                                here.used += ahead + line.extent;
                                ahead = 0.0;
                                if count_demerits {
                                    here.demerits += line.demerits;
                                    here.baseline += ways[0].1.get(number).map_or(0.0, |first| first.demerits);
                                    here.line_extent += line.extent;
                                    here.lines += 1;
                                }
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
                                    // The lines that the next fragment of the container starts with
                                    // are the rest of the paragraph.
                                    let widows = ends_fragment
                                        && ((lines.len() - lines_on_page as usize) as u32) < widows_wanted;
                                    let found = Found { token, legal, widows, spec, kind: page_kind };
                                    let on_this_page = in_start.then_some(lines_on_page);
                                    match self.emit(
                                        record,
                                        &found,
                                        &here,
                                        0.0,
                                        on_this_page,
                                        start_paragraph,
                                        &mut first,
                                        &on_page,
                                        &mut fills,
                                    ) {
                                        Emit::Go => {}
                                        // (the way is too full for the rest of the paragraph)
                                        Emit::Dropped | Emit::Stop => {
                                            alive = false;
                                            break;
                                        }
                                    }
                                }
                            }
                            if !alive {
                                continue;
                            }
                            if forced_end.is_some() {
                                // the page ends at the cut: the end of the last of the lines
                                if let Some(last) = lines.last() {
                                    let lines_on_page = lines.len() as u32;
                                    let token = self.tokens.intern(Position::InParagraph { item, cut: last.end });
                                    let legal = Legal {
                                        orphans: broken_orphans,
                                        widows: broken_widows,
                                        ..Legal::new(end_kind, Some((lines_on_page, orphans)))
                                    };
                                    let found = Found { token, legal, widows: false, spec, kind: page_kind };
                                    let on_this_page = in_start.then_some(lines_on_page);
                                    self.emit(
                                        record,
                                        &found,
                                        &here,
                                        0.0,
                                        on_this_page,
                                        start_paragraph,
                                        &mut first,
                                        &on_page,
                                        &mut fills,
                                    );
                                }
                                return fills;
                            }
                            if in_start {
                                here.tail = Some(lines.len() as u32);
                            }
                            behind.push(here);
                        }
                    }
                    // (of the ways that use the same room, the one with the least demerits)
                    labels = prune(behind);
                    pending = 0.0;
                    leading = false;
                    if labels.is_empty() {
                        return fills;
                    }
                    item += 1;
                }
                Item::Container { node, avoid_inside } => {
                    // The room that a container has depends on what is in front of it, so the
                    // ways to break the paragraphs in front of it are narrowed to the one with the
                    // least demerits.
                    let best = labels
                        .iter()
                        .min_by(|left, right| left.excess().total_cmp(&right.excess()))
                        .cloned()
                        .expect("a page has a way to break its paragraphs");
                    labels = vec![best];
                    let used = labels[0].used;
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
                        if self.emit(
                            record,
                            &found,
                            &labels[0],
                            pending + fragment.used,
                            None,
                            start_paragraph,
                            &mut first,
                            &here,
                            &mut fills,
                        ) != Emit::Go
                        {
                            return fills;
                        }
                    }
                    // The page cannot go on behind a container that continues on the next one.
                    let Some((fragment, page_fragment)) = ended else {
                        return fills;
                    };
                    labels[0].used += pending + fragment.used;
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
            let found = Found { token: end, legal, widows: false, spec, kind: page_kind };
            for label in &labels {
                self.emit(record, &found, label, 0.0, None, start_paragraph, &mut first, &on_page, &mut fills);
            }
        }
        fills
    }

    /// Adds the fill of the page that ends at the token, with the way to break its paragraphs that
    /// the label is, and `add` more room used. `on_this_page` is the number of lines of the
    /// paragraph that the page starts in that it has, if it ends in it.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        record: bool,
        found: &Found,
        label: &Label,
        add: f64,
        on_this_page: Option<u32>,
        start_paragraph: Option<StartParagraph>,
        first: &mut bool,
        fragments: &[PageFragment],
        fills: &mut Fills,
    ) -> Emit {
        let Found { token: end, legal, widows, spec, kind } = *found;
        let used = (label.used + add) as f32;
        let overflow = used > spec.block_size;
        // A page that is too full does not take the break after it, unless there is no break
        // in between: its content then gets a page of its own.
        if overflow && !*first {
            return Emit::Dropped;
        }
        *first = false;
        let orphan_violation = legal.orphans
            || legal
                .cut
                .is_some_and(|(on_page, orphans)| on_page < orphans);
        let widows = widows || widow_violation(start_paragraph, label.tail, on_this_page);
        let choices = self.choice_id(&label.choices);
        fills.0.push(PageFill {
            end,
            spec,
            kind,
            used,
            end_kind: legal.kind,
            orphan_violation,
            widow_violation: widows || legal.widows,
            overflow,
            line_demerits: label.excess(),
            line_extent: if label.lines > 0 { (label.line_extent / f64::from(label.lines)) as f32 } else { 0.0 },
            choices,
        });
        // (the fragments are only recorded if there are some to ask for)
        if record {
            fills.1.push(fragments.to_vec());
        }
        if legal.kind.is_forced() {
            Emit::Stop
        } else {
            Emit::Go
        }
    }

    /// The number of a list of choices.
    fn choice_id(&mut self, choices: &[Choice]) -> ChoiceId {
        if choices.is_empty() {
            return ChoiceId::NONE;
        }
        if let Some(&id) = self.choice_ids.get(choices) {
            return id;
        }
        let id = ChoiceId(self.choices.len() as u32);
        self.choices.push(Rc::from(choices));
        self.choice_ids.insert(choices.to_vec(), id);
        id
    }

    /// The ways to break the paragraphs of a page that are not the first.
    pub fn choices(&self, id: ChoiceId) -> Rc<[Choice]> {
        self.choices[id.0 as usize].clone()
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
            let wanted = solution.choices.get(page).copied();
            let found = fills
                .into_iter()
                .find(|fill| fill.end == expected && wanted.is_none_or(|choices| fill.choices == choices));
            let fill = match found {
                Some(fill) => fill,
                // A page can end at any cut of a paragraph, not only where a line ends when the
                // paragraph is broken for the page.
                None => match self.tokens.position(expected) {
                    Position::InParagraph { item, cut } => self
                        .fill_at(&start, item, cut)
                        .filter(|fill| fill.end == expected)
                        .ok_or(Invalid::NotACandidate { page })?,
                    _ => return Err(Invalid::NotACandidate { page }),
                },
            };
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
fn widow_violation(start: Option<StartParagraph>, tail: Option<u32>, on_page: Option<u32>) -> bool {
    start.is_some_and(|start| on_page.or(tail).is_some_and(|lines| lines < start.widows))
}
