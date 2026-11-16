use crate::paragraph::compute_breakpoints;
use crate::*;
use std::collections::HashMap;

const ALLOWED: BreakKind = BreakKind::Allowed;

fn page(block_size: f32) -> PageTable {
    PageTable::uniform(PageSpec {
        inline_size: 300.0,
        block_size,
    })
}

const START: Position = Position::Item(0);

fn cut(item: usize, line: u32) -> Position {
    Position::InParagraph {
        item,
        cut: Cut(line),
    }
}

/// The fills of the page that starts at the position `at`.
fn fills(flow: GalleyFlow, block_size: f32, at: Position) -> Vec<PageFill> {
    let (problem, mut service) = flow.finish(page(block_size));
    let mut kernel = Kernel::new(&problem, &mut service);
    let start = PageStart {
        at: kernel.intern(at),
        index: usize::from(at != START),
    };
    kernel.candidates(&start)
}

fn used(fills: &[PageFill]) -> Vec<f32> {
    fills.iter().map(|fill| fill.used).collect()
}

#[test]
fn gaps_at_the_edges_of_a_page_are_discarded() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0).gap(5.0).content(10.0).break_(ALLOWED);
    flow.gap(5.0).content(10.0).gap(7.0);
    // the gap in the middle of the page stays, the one in front of the break goes
    assert_eq!(used(&fills(flow.clone(), 100.0, START)), [25.0, 40.0]);
    // the gaps at the start and at the end of the second page go
    assert_eq!(used(&fills(flow, 100.0, Position::Item(4))), [10.0]);
}

#[test]
fn a_gap_in_front_of_a_break_does_not_count_against_the_page() {
    let mut flow = GalleyFlow::new();
    flow.content(20.0).gap(5.0).break_(ALLOWED).content(20.0);
    let fills = fills(flow, 20.0, START);
    // the second box does not fit, and the end of the document is behind it
    assert_eq!(used(&fills), [20.0]);
    assert!(!fills[0].overflow);
}

#[test]
fn gaps_after_a_forced_break_and_at_the_document_start_are_kept() {
    let mut flow = GalleyFlow::new();
    flow.gap(3.0)
        .content(10.0)
        .break_(BreakKind::Forced)
        .gap(5.0)
        .content(10.0);
    assert_eq!(used(&fills(flow.clone(), 100.0, START)), [13.0]);
    assert_eq!(used(&fills(flow, 100.0, Position::Item(3))), [15.0]);
}

#[test]
fn a_gap_after_a_break_that_is_not_forced_is_discarded_even_if_it_follows_an_avoid() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::Avoid)
        .gap(5.0)
        .content(10.0);
    assert_eq!(used(&fills(flow, 100.0, Position::Item(2))), [10.0]);
}

fn paragraph(lines: usize, orphans: u32, widows: u32) -> GalleyFlow {
    let mut flow = GalleyFlow::new();
    flow.paragraph(&vec![10.0; lines], orphans, widows, false);
    flow
}

#[test]
fn orphans_are_counted_on_the_page_that_ends_in_the_paragraph() {
    // a paragraph of five lines, cut after 1, 2, 3 and 4 lines, and its end
    let fills = fills(paragraph(5, 2, 2), 100.0, START);
    assert_eq!(used(&fills), [10.0, 20.0, 30.0, 40.0, 50.0]);
    let orphans: Vec<_> = fills.iter().map(|fill| fill.orphan_violation).collect();
    assert_eq!(orphans, [true, false, false, false, false]);
    // the start of the paragraph is not a cut: no widow can be left behind
    assert!(fills.iter().all(|fill| !fill.widow_violation));
}

#[test]
fn widows_are_counted_on_the_page_that_starts_in_the_paragraph() {
    // the page that starts in front of the fourth line has two lines of the paragraph left
    let at_3 = fills(paragraph(5, 2, 2), 100.0, cut(0, 3));
    assert_eq!(used(&at_3), [10.0, 20.0]);
    // the fifth line alone ends the paragraph: it is a widow, and the page that ends in front
    // of it has the right number of lines
    let at_4 = fills(paragraph(5, 2, 2), 100.0, cut(0, 4));
    assert!(at_4[0].widow_violation);
    assert!(!at_3[1].widow_violation);
    // a page that starts at the third cut and ends at the next one has one line of the paragraph
    assert!(at_3[0].orphan_violation);
    assert!(at_3[0].widow_violation);
}

#[test]
fn orphans_and_widows_of_other_paragraphs_do_not_interact() {
    let mut flow = GalleyFlow::new();
    flow.paragraph(&[10.0; 5], 2, 2, false).break_(ALLOWED);
    flow.paragraph(&[10.0; 5], 2, 2, false);
    // the first page that starts in front of the second paragraph and ends after its first
    // line is an orphan; the first paragraph, which ends the page before, plays no role
    let (problem, mut service) = flow.finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    // the page that starts in front of the second paragraph (item 2)
    let at = kernel.intern(Position::Item(2));
    let fills = kernel.candidates(&PageStart { at, index: 1 });
    assert_eq!(fills.len(), 5);
    assert!(fills[0].orphan_violation);
    assert!(!fills[1].orphan_violation && !fills[1].widow_violation);
    // from the fourth cut of the first paragraph: the lines on this page are the last one
    let at = kernel.intern(cut(0, 4));
    let fills = kernel.candidates(&PageStart { at, index: 1 });
    assert!(fills[0].widow_violation);
    assert!(!fills[0].orphan_violation);
}

#[test]
fn cuts_in_a_paragraph_that_avoids_breaks_inside_are_avoid_breaks() {
    let mut flow = GalleyFlow::new();
    flow.paragraph(&[10.0; 3], 1, 1, true);
    let kinds: Vec<_> = fills(flow, 100.0, START)
        .iter()
        .map(|fill| fill.end_kind)
        .collect();
    assert_eq!(kinds, [BreakKind::Avoid, BreakKind::Avoid, ALLOWED]);
}

#[test]
fn an_unbreakable_item_taller_than_the_page_gets_a_page_of_its_own() {
    let mut flow = GalleyFlow::new();
    flow.content(50.0)
        .break_(ALLOWED)
        .content(150.0)
        .break_(ALLOWED)
        .content(50.0);
    let first = fills(flow.clone(), 100.0, START);
    // the second item does not fit: the page stops at the first break
    assert_eq!(used(&first), [50.0]);
    let second = fills(flow.clone(), 100.0, Position::Item(2));
    assert_eq!(used(&second), [150.0]);
    assert!(second[0].overflow);
    // the page after it is a normal page
    let third = fills(flow, 100.0, Position::Item(4));
    assert_eq!(used(&third), [50.0]);
    assert!(!third[0].overflow);
}

#[test]
fn an_overflowing_end_of_the_document_is_listed_when_nothing_else_is() {
    let mut flow = GalleyFlow::new();
    flow.content(150.0);
    let fills = fills(flow, 100.0, START);
    assert_eq!(fills.len(), 1);
    assert!(fills[0].overflow);
}

#[test]
fn forced_breaks_end_the_list_of_candidates() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::Forced)
        .content(10.0)
        .break_(ALLOWED)
        .content(10.0);
    let (problem, mut service) = flow.finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    let first = kernel.first_page();
    assert_eq!(used(&kernel.candidates(&first)), [10.0]);
}

#[test]
fn the_cost_of_a_page_follows_the_css_objective() {
    let spec = PageSpec {
        inline_size: 0.0,
        block_size: 100.0,
    };
    let fill = PageFill {
        end: BreakToken(1),
        spec,
        kind: PageKind::Right,
        used: 90.0,
        end_kind: ALLOWED,
        orphan_violation: false,
        widow_violation: false,
        overflow: false,
    };
    let css = CssObjective;
    assert_eq!(css.page_cost(&fill, false), 1000.0);
    assert_eq!(css.page_cost(&fill, true), 0.0);
    let avoid = PageFill {
        end_kind: BreakKind::Avoid,
        ..fill
    };
    assert_eq!(
        css.page_cost(&avoid, false),
        1000.0 + CssObjective::AVOID_BREAK_COST
    );
    let widow = PageFill {
        widow_violation: true,
        orphan_violation: true,
        ..fill
    };
    assert_eq!(
        css.page_cost(&widow, false),
        1000.0 + CssObjective::ORPHANS_WIDOWS_COST
    );
    let forced = PageFill {
        end_kind: BreakKind::Forced,
        ..fill
    };
    assert_eq!(
        css.page_cost(&forced, true),
        CssObjective::FORCED_BREAK_COST
    );
    let overflow = PageFill {
        overflow: true,
        used: 150.0,
        end_kind: BreakKind::Avoid,
        ..fill
    };
    assert_eq!(css.page_cost(&overflow, false), CssObjective::OVERFLOW_COST);
}

fn solve(flow: GalleyFlow, block_size: f32) -> (Vec<Position>, Vec<PageFill>) {
    let (problem, mut service) = flow.finish(page(block_size));
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let pages = kernel
        .realize(&solution)
        .expect("the solution of the exact solver is valid");
    (
        solution
            .pages
            .iter()
            .map(|start| kernel.position(start.at))
            .collect(),
        pages,
    )
}

#[test]
fn the_exact_solver_fills_pages_and_avoids_avoid_breaks() {
    let mut flow = GalleyFlow::new();
    flow.content(50.0)
        .break_(BreakKind::Avoid)
        .content(40.0)
        .break_(ALLOWED)
        .content(40.0);
    // [50 40] [40] costs 10^3; [50] [40 40] costs 50^3 and an avoid break
    let (starts, pages) = solve(flow, 100.0);
    assert_eq!(starts, [START, Position::Item(4)]);
    assert_eq!(used(&pages), [90.0, 40.0]);
}

#[test]
fn the_exact_solver_takes_forced_breaks() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::Forced)
        .content(10.0)
        .break_(ALLOWED)
        .content(10.0);
    let (starts, _) = solve(flow, 100.0);
    assert_eq!(starts, [START, Position::Item(2)]);
}

#[test]
fn the_exact_solver_keeps_the_widows_of_a_paragraph() {
    // five lines on pages of three: [3] [2] has no violation, [4] [1] a widow
    let (starts, pages) = solve(paragraph(5, 2, 2), 40.0);
    assert_eq!(starts, [START, cut(0, 3)]);
    assert!(
        pages
            .iter()
            .all(|fill| !fill.orphan_violation && !fill.widow_violation)
    );
    // the cut after the second line is as good as the one after the third on pages of two
    // lines, and the better filled first page wins
    let (starts, _) = solve(paragraph(5, 2, 2), 30.0);
    assert_eq!(starts, [START, cut(0, 3)]);
}

#[test]
fn the_exact_solver_gives_an_overflowing_item_its_own_page() {
    let mut flow = GalleyFlow::new();
    flow.content(50.0)
        .break_(ALLOWED)
        .content(150.0)
        .break_(ALLOWED)
        .content(50.0);
    let (starts, pages) = solve(flow, 100.0);
    assert_eq!(starts, [START, Position::Item(2), Position::Item(4)]);
    assert!(pages[1].overflow);
}

#[test]
fn an_empty_document_has_one_page_that_realizes() {
    let (starts, pages) = solve(GalleyFlow::new(), 100.0);
    assert_eq!(starts, [START]);
    assert!(pages.is_empty());
}

#[test]
fn realize_rejects_what_is_not_a_page_sequence() {
    let mut flow = GalleyFlow::new();
    flow.content(60.0)
        .break_(ALLOWED)
        .content(60.0)
        .break_(ALLOWED)
        .content(60.0);
    let (problem, mut service) = flow.finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    let mut solution = |pages: &[usize]| Solution {
        pages: pages
            .iter()
            .enumerate()
            .map(|(index, &item)| PageStart {
                at: kernel.intern(Position::Item(item)),
                index,
            })
            .collect(),
    };
    let (ok, from_second, two_at_once, backwards) = (
        solution(&[0, 2, 4]),
        solution(&[2, 4]),
        solution(&[0, 4]),
        solution(&[0, 2, 2]),
    );
    assert!(kernel.realize(&ok).is_ok());
    // not from the start
    assert_eq!(kernel.realize(&from_second), Err(Invalid::FirstPage));
    // two items do not fit on a page
    assert_eq!(
        kernel.realize(&two_at_once),
        Err(Invalid::NotACandidate { page: 0 })
    );
    // going backwards
    assert_eq!(
        kernel.realize(&backwards),
        Err(Invalid::NotACandidate { page: 1 })
    );
    assert_eq!(
        kernel.realize(&Solution { pages: vec![] }),
        Err(Invalid::FirstPage)
    );
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

/// The break points of the paginator that the exact solver replaces, as they were handed to
/// `compute_breakpoints`.
struct Reference {
    advance: Vec<f32>,
    forced: Vec<bool>,
    avoid: Vec<bool>,
    /// (paragraph, lines in front of the break, total, orphans, widows)
    in_lines: Vec<Option<(usize, usize, usize, usize, usize)>>,
    heights: Vec<f32>,
}

fn reference_violation(reference: &Reference, from: usize, to: usize) -> bool {
    if let Some(end) = reference.in_lines[to] {
        let start = reference.in_lines[from]
            .filter(|start| start.0 == end.0)
            .map_or(0, |start| start.1);
        if end.1 - start < end.3 {
            return true;
        }
    }
    if let Some(start) = reference.in_lines[from] {
        let end = reference.in_lines[to]
            .filter(|end| end.0 == start.0)
            .map_or(start.2, |end| end.1);
        if end - start.1 < start.4 {
            return true;
        }
    }
    false
}

fn reference_solution(reference: &Reference) -> Vec<usize> {
    let count = reference.advance.len();
    let last = count - 1;
    let mut sequence = compute_breakpoints(count, &mut |from, to| {
        // a page does not span a forced break
        if reference.forced[from + 1..to].iter().any(|&forced| forced) {
            return f64::INFINITY;
        }
        let height = reference.advance[to] - reference.advance[from];
        let content = reference.heights[from];
        if height > content {
            if to == from + 1 { 1e30 } else { f64::INFINITY }
        } else {
            let mut cost = if reference.forced[to] {
                -1e15
            } else if to == last {
                0.0
            } else {
                (content as f64 - height as f64).powi(3)
            };
            if reference.avoid[to] {
                cost += 1e12;
            }
            if reference_violation(reference, from, to) {
                cost += 1e11;
            }
            cost
        }
    });
    assert_eq!(sequence.pop(), Some(last));
    sequence
}

/// A flow of boxes, paragraphs and breaks that has many ties, with the break points that the
/// old paginator would have found in it.
fn random_problem(rng: &mut Rng) -> (Problem, GalleyService, Reference, Vec<Position>) {
    let mut flow = GalleyFlow::new();
    let mut advance = vec![0.0f32];
    let mut y = 0.0;
    let mut forced = vec![false];
    let mut avoid = vec![false];
    let mut in_lines = vec![None];
    // the number of lines of the paragraphs
    let mut line_counts: HashMap<usize, u32> = HashMap::new();
    let elements = 2 + rng.next(14);
    for element in 0..elements {
        let extent = |rng: &mut Rng| 10.0 * rng.next(5) as f64;
        match rng.next(4) {
            0 | 1 => {
                let e = extent(rng);
                flow.content(e);
                y += e;
            }
            2 => {
                let lines = 2 + rng.next(6) as usize;
                let heights: Vec<f64> = (0..lines)
                    .map(|_| 10.0 * (1 + rng.next(2)) as f64)
                    .collect();
                let (orphans, widows) = (1 + rng.next(3) as u32, 1 + rng.next(3) as u32);
                let avoid_inside = rng.next(4) == 0;
                let id = flow.len();
                line_counts.insert(id, lines as u32);
                flow.paragraph(&heights, orphans, widows, avoid_inside);
                for (line, h) in heights.iter().enumerate() {
                    y += h;
                    if line + 1 < lines {
                        advance.push(y as f32);
                        forced.push(false);
                        avoid.push(avoid_inside);
                        in_lines.push(Some((
                            id,
                            line + 1,
                            lines,
                            orphans as usize,
                            widows as usize,
                        )));
                    }
                }
            }
            _ => {
                // a break is only a break between two items
                if flow.is_empty() || element + 1 == elements {
                    continue;
                }
                let kind = match rng.next(6) {
                    0 => BreakKind::Forced,
                    1 | 2 => BreakKind::Avoid,
                    _ => ALLOWED,
                };
                flow.break_(kind);
                advance.push(y as f32);
                forced.push(kind.is_forced());
                avoid.push(kind == BreakKind::Avoid);
                in_lines.push(None);
            }
        }
    }
    advance.push(y as f32);
    forced.push(false);
    avoid.push(false);
    in_lines.push(None);
    // a trailing break is the end of the document, not a place to break
    if matches!(
        flow.clone().finish(page(0.0)).0.items.last(),
        Some(Item::Break { .. })
    ) {
        advance.remove(advance.len() - 2);
        forced.remove(forced.len() - 2);
        avoid.remove(avoid.len() - 2);
        in_lines.remove(in_lines.len() - 2);
    }
    let count = advance.len();
    let heights: Vec<f32> = (0..count)
        .map(|_| [60.0, 80.0, 100.0][rng.next(3) as usize])
        .collect();
    let (problem, service) = flow.finish(page(0.0));
    if problem.items.is_empty() {
        return random_problem(rng);
    }
    // the places where a page can start, in the order of the flow
    let mut positions = vec![Position::Item(0)];
    for (item, entry) in problem.items.iter().enumerate() {
        match entry {
            Item::Break { .. } if item + 1 < problem.items.len() => {
                positions.push(Position::Item(item + 1))
            }
            Item::Paragraph { .. } => {
                for line in 1..line_counts[&item] {
                    positions.push(cut(item, line));
                }
            }
            _ => {}
        }
    }
    positions.push(Position::Item(problem.items.len()));
    (
        problem,
        service,
        Reference {
            advance,
            forced,
            avoid,
            in_lines,
            heights,
        },
        positions,
    )
}

#[test]
fn the_exact_solver_chooses_what_the_least_weight_subsequence_chose() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for _ in 0..3000 {
        let (mut problem, mut service, reference, positions) = random_problem(&mut rng);
        assert_eq!(positions.len(), reference.advance.len());
        let specs: HashMap<Position, f32> = positions
            .iter()
            .copied()
            .zip(reference.heights.iter().copied())
            .collect();
        problem.pages = PageTable::new(false, move |position, _| PageSpec {
            inline_size: 0.0,
            block_size: specs[&position],
        })
        .left_right_differ(false);
        let mut kernel = Kernel::new(&problem, &mut service);
        let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
        let chosen: Vec<usize> = solution
            .pages
            .iter()
            .map(|start| {
                let position = kernel.position(start.at);
                positions.iter().position(|&other| other == position).unwrap()
            })
            .collect();
        assert_eq!(
            chosen,
            reference_solution(&reference),
            "{:?}",
            problem.items
        );
        assert!(kernel.realize(&solution).is_ok());
        *counts.entry(chosen.len()).or_default() += 1;
    }
    // the test is only worth something if it paginates into many pages
    assert!(counts.keys().any(|&pages| pages >= 4), "{counts:?}");
}

#[test]
fn a_break_to_a_right_page_leaves_a_left_page_empty() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::ForcedRight)
        .content(10.0)
        .break_(BreakKind::ForcedLeft)
        .content(10.0)
        .break_(BreakKind::ForcedLeft)
        .content(10.0);
    let (problem, mut service) = flow.finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let numbers: Vec<_> = solution.pages.iter().map(|start| start.index).collect();
    // the first page is a right page, the second is left and empty, the third is right and
    // has the content; the next page is left; the fifth is right, so the sixth is the one
    assert_eq!(numbers, [0, 2, 3, 5]);
    assert!(kernel.realize(&solution).is_ok());
    // a solution that does not leave the page empty is not valid
    let wrong = Solution {
        pages: solution
            .pages
            .iter()
            .enumerate()
            .map(|(index, start)| PageStart { index, ..*start })
            .collect(),
    };
    assert_eq!(kernel.realize(&wrong), Err(Invalid::NotACandidate { page: 0 }));
}

#[test]
fn the_pages_of_a_right_to_left_document_start_on_the_left() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::ForcedLeft)
        .content(10.0);
    let (mut problem, mut service) = flow.finish(page(100.0));
    problem.pages.rtl = true;
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    // the first page is a left page, so the second is a right page, and the third a left page
    let numbers: Vec<_> = solution.pages.iter().map(|start| start.index).collect();
    assert_eq!(numbers, [0, 2]);
}

#[test]
fn the_geometry_of_a_page_follows_its_kind() {
    // right pages hold two boxes, left pages one
    let table = PageTable::new(false, |_, kind| PageSpec {
        inline_size: 100.0,
        block_size: if kind == PageKind::Left { 45.0 } else { 100.0 },
    });
    let mut flow = GalleyFlow::new();
    for _ in 0..5 {
        flow.content(40.0).break_(ALLOWED);
    }
    flow.content(40.0);
    let (problem, mut service) = flow.finish(table);
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let pages = kernel.realize(&solution).unwrap();
    let kinds: Vec<_> = pages.iter().map(|fill| fill.kind).collect();
    assert_eq!(
        kinds,
        [PageKind::First, PageKind::Left, PageKind::Right, PageKind::Left]
    );
    assert_eq!(used(&pages), [80.0, 40.0, 80.0, 40.0]);
}

/// A paragraph of 80 characters. A line has as many characters as fit the inline size of the
/// page: one for every 10 units. A cut is the number of the character that the next line starts
/// with, so a page that continues the paragraph breaks the rest of it for its own width.
struct Characters;

impl LayoutService for Characters {
    fn extent(&mut self, _: NodeRef, _: f32) -> f64 {
        0.0
    }

    fn lines(&mut self, _: ParId, from: Option<Cut>, page_inline: f32) -> std::rc::Rc<[ParLine]> {
        let per_line = (page_inline / 10.0) as u32;
        let mut lines = vec![];
        let mut at = from.map_or(0, |cut| cut.0);
        while at < 80 {
            at = (at + per_line).min(80);
            lines.push(ParLine {
                extent: 10.0,
                end: Cut(at),
            });
        }
        lines.into()
    }

    // (the paragraph does not have subtrees)
    fn content_extent(&mut self, _: BoxRef, _: f32) -> f64 {
        0.0
    }

    fn extent_in(&mut self, _: BoxRef, _: NodeRef, _: f32) -> f64 {
        0.0
    }

    fn lines_in(&mut self, _: BoxRef, par: ParId, from: Option<Cut>, inline: f32) -> std::rc::Rc<[ParLine]> {
        self.lines(par, from, inline)
    }
}

#[test]
fn a_paragraph_is_cut_for_the_width_of_each_page() {
    let items = vec![Item::Paragraph {
        id: ParId(0),
        orphans: 1,
        widows: 1,
        avoid_inside: false,
    }];
    // the first page is 80 wide and 40 high: four lines of eight characters. The other pages are
    // 40 wide: four lines of four characters.
    let table = PageTable::new(false, |_, kind| PageSpec {
        inline_size: if kind == PageKind::First { 80.0 } else { 40.0 },
        block_size: 40.0,
    });
    let problem = Problem::new(table, items);
    let mut service = Characters;
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let cuts: Vec<_> = solution
        .pages
        .iter()
        .map(|start| kernel.position(start.at))
        .collect();
    // 32 characters on the first page, then 16 on each of the three others
    assert_eq!(cuts, [START, cut(0, 32), cut(0, 48), cut(0, 64)]);
    let pages = kernel.realize(&solution).unwrap();
    assert_eq!(used(&pages), [40.0; 4]);
    assert!(pages.iter().all(|fill| !fill.overflow));
}

/// The columns of a flow that is balanced in `columns` columns that can hold `block_size`, as
/// the content of every column.
fn balanced(flow: GalleyFlow, columns: usize, block_size: f32) -> Vec<f32> {
    let (problem, mut service) = flow.finish(page(block_size));
    let problem = problem.with_level(BreakLevel::Column).with_page_limit(columns);
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = balance(&mut kernel);
    let pages = kernel.realize(&solution).expect("the solution of balancing is valid");
    used(&pages)
}

fn items(count: usize) -> GalleyFlow {
    let mut flow = GalleyFlow::new();
    for index in 0..count {
        if index > 0 {
            flow.break_at(ALLOWED, BreakLevel::Column);
        }
        flow.content(20.0);
    }
    flow
}

#[test]
fn a_break_of_the_other_level_is_no_break() {
    let flow = |level| {
        let mut flow = GalleyFlow::new();
        flow.content(10.0)
            .break_at(BreakKind::Forced, level)
            .content(10.0)
            .break_at(BreakKind::Avoid, level)
            .content(10.0);
        flow
    };
    let used_of = |flow: GalleyFlow, level| {
        let (problem, mut service) = flow.finish(page(100.0));
        let problem = problem.with_level(level);
        let mut kernel = Kernel::new(&problem, &mut service);
        let first = kernel.first_page();
        used(&kernel.candidates(&first))
    };
    // on pages, a forced page break ends the page, and nothing else is possible
    assert_eq!(used_of(flow(BreakLevel::Page), BreakLevel::Page), [10.0]);
    // a column break is no break on a page: the avoid break is a break that can be taken, and so
    // is the one that was forced
    assert_eq!(used_of(flow(BreakLevel::Column), BreakLevel::Page), [10.0, 20.0, 30.0]);
    // in columns it is the other way round
    assert_eq!(used_of(flow(BreakLevel::Column), BreakLevel::Column), [10.0]);
    assert_eq!(used_of(flow(BreakLevel::Page), BreakLevel::Column), [10.0, 20.0, 30.0]);
    // an avoid break of the other level is not avoided
    let (problem, mut service) = flow(BreakLevel::Column).finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    let first = kernel.first_page();
    let ends: Vec<BreakKind> = kernel.candidates(&first).iter().map(|fill| fill.end_kind).collect();
    assert!(ends.iter().all(|kind| *kind == BreakKind::Allowed), "{ends:?}");
}

#[test]
fn the_last_page_that_there_can_be_takes_the_rest() {
    // ten boxes of 20 on pages that hold 60, but there are two pages at most
    let (problem, mut service) = items(10).finish(page(60.0));
    let problem = problem.with_page_limit(2);
    let mut kernel = Kernel::new(&problem, &mut service);
    let first = kernel.first_page();
    // the first page ends after one, two or three boxes
    assert_eq!(used(&kernel.candidates(&first)), [20.0, 40.0, 60.0]);
    let fill = kernel.candidates(&first)[2];
    let second = kernel.next_page(&first, &fill).unwrap();
    // the second takes the rest, and does not fit: 140 for 7 boxes with the break in between
    let fills = kernel.candidates(&second);
    assert_eq!(used(&fills), [140.0]);
    assert!(fills[0].overflow);
    assert!(kernel.is_end(fills[0].end));
}

#[test]
fn balanced_columns_are_as_low_as_they_can_be_and_the_first_ones_are_the_fullest() {
    assert_eq!(balanced(items(6), 3, 1000.0), [40.0, 40.0, 40.0]);
    assert_eq!(balanced(items(5), 2, 1000.0), [60.0, 40.0]);
    // 5 boxes in three columns: two, two and one
    assert_eq!(balanced(items(5), 3, 1000.0), [40.0, 40.0, 20.0]);
    // there are more columns than boxes
    assert_eq!(balanced(items(2), 3, 1000.0), [20.0, 20.0]);
    assert_eq!(balanced(items(1), 2, 1000.0), [20.0]);
    // one column
    assert_eq!(balanced(items(3), 1, 1000.0), [60.0]);
}

#[test]
fn balanced_columns_are_within_the_height_that_there_is() {
    // six boxes in a height of 100: three and three
    assert_eq!(balanced(items(6), 2, 100.0), [60.0, 60.0]);
    // ten boxes in two columns of 60: the first is as full as it can be, and the last overflows
    let (problem, mut service) = items(10).finish(page(60.0));
    let problem = problem.with_level(BreakLevel::Column).with_page_limit(2);
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = balance(&mut kernel);
    let pages = kernel.realize(&solution).unwrap();
    assert_eq!(used(&pages), [60.0, 140.0]);
    assert_eq!([pages[0].overflow, pages[1].overflow], [false, true]);
}

#[test]
fn balanced_columns_respect_forced_breaks_and_break_where_they_are_allowed_to() {
    // a forced column break after the first of five boxes leaves the balance nothing to choose
    let mut flow = GalleyFlow::new();
    flow.content(20.0).break_at(BreakKind::Forced, BreakLevel::Column);
    for index in 0..4 {
        if index > 0 {
            flow.break_at(ALLOWED, BreakLevel::Column);
        }
        flow.content(20.0);
    }
    assert_eq!(balanced(flow, 2, 1000.0), [20.0, 80.0]);

    // A (20) B (20) C (40, unbreakable) D (20) in two columns: [A, B] and [C, D]
    let mut flow = GalleyFlow::new();
    flow.content(20.0)
        .break_at(ALLOWED, BreakLevel::Column)
        .content(20.0)
        .break_at(ALLOWED, BreakLevel::Column)
        .content(40.0)
        .break_at(ALLOWED, BreakLevel::Column)
        .content(20.0);
    assert_eq!(balanced(flow, 2, 1000.0), [40.0, 60.0]);

    // the break after the second of four boxes is to be avoided: three and one
    let mut flow = GalleyFlow::new();
    flow.content(20.0)
        .break_at(ALLOWED, BreakLevel::Column)
        .content(20.0)
        .break_at(BreakKind::Avoid, BreakLevel::Column)
        .content(20.0)
        .break_at(ALLOWED, BreakLevel::Column)
        .content(20.0);
    assert_eq!(balanced(flow, 2, 1000.0), [60.0, 20.0]);
}

#[test]
fn balanced_columns_cut_a_paragraph_where_the_widows_allow_it() {
    // A paragraph of 7 lines of 10 in two columns, at least 2 lines at each end: 4 and 3
    let mut flow = GalleyFlow::new();
    flow.paragraph(&[10.0; 7], 2, 2, false);
    assert_eq!(balanced(flow, 2, 1000.0), [40.0, 30.0]);
    // 3 lines with orphans and widows of 2 cannot be cut: they stay in one column
    let mut flow = GalleyFlow::new();
    flow.paragraph(&[10.0; 3], 2, 2, false);
    assert_eq!(balanced(flow, 2, 1000.0), [30.0]);
}

/// An objective that takes the maximum of the costs of the pages, which is the cost of a page.
struct Tallest;

impl Objective for Tallest {
    fn page_cost(&self, fill: &PageFill, _is_last: bool) -> f64 {
        fill.used as f64
    }

    fn combine(&self, so_far: f64, page: f64, _index: usize) -> f64 {
        so_far.max(page)
    }
}

#[test]
fn the_exact_solver_minimises_what_the_objective_combines() {
    // the sum of the heights is the same for any pagination of these boxes in two pages, and
    // the sum of the squares is least where they are the same height: the objective takes the
    // maximum, which is the same
    let mut flow = GalleyFlow::new();
    flow.content(30.0)
        .break_(ALLOWED)
        .content(30.0)
        .break_(ALLOWED)
        .content(30.0)
        .break_(ALLOWED)
        .content(30.0);
    let (problem, mut service) = flow.finish(page(100.0));
    let problem = problem.with_page_limit(2);
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(Tallest).solve(&mut kernel);
    let pages = kernel.realize(&solution).unwrap();
    assert_eq!(used(&pages), [60.0, 60.0]);
    // with the sum, which is what pages are made of, the fullest pages win
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let pages = kernel.realize(&solution).unwrap();
    assert_eq!(used(&pages), [90.0, 30.0]);
}

// Containers that are split over pages.

/// A container of `count` boxes of 20 in `columns` columns, as the content of a flow.
fn container(count: usize, columns: usize, fill: ColumnFill) -> GalleyFlow {
    let mut inner = items(count);
    // the container is one box in the flow of the page
    let mut flow = GalleyFlow::new();
    flow.container(std::mem::take(&mut inner), columns, fill, (0.0, 0.0));
    flow
}

/// The pages of a flow of the exact solver: where each one starts, and what it holds.
fn container_pages(flow: GalleyFlow, block_size: f32) -> (Vec<Position>, Vec<PageFill>) {
    solve(flow, block_size)
}

fn in_container(item: usize, inner: usize) -> Position {
    Position::InContainer { item, cut: ContainerCut::at_item(inner) }
}

#[test]
fn a_container_that_fits_is_one_fragment_with_balanced_columns() {
    let (pages, fills) = container_pages(container(5, 2, ColumnFill::Balance), 100.0);
    assert_eq!(pages, [START]);
    // three boxes and two (the breaks between the boxes are items, too)
    assert_eq!(used(&fills), [60.0]);
    // filled in turn, the columns are the ones of the page
    let (pages, fills) = container_pages(container(5, 2, ColumnFill::Auto), 100.0);
    assert_eq!(pages, [START]);
    assert_eq!(used(&fills), [100.0]);
}

#[test]
fn a_container_that_does_not_fit_has_a_fragment_on_every_page() {
    // 25 boxes in two columns of five: two full fragments, then five boxes balanced as 3 and 2
    let (pages, fills) = container_pages(container(25, 2, ColumnFill::Balance), 100.0);
    assert_eq!(used(&fills), [100.0, 100.0, 60.0]);
    // the container is one item, and a fragment starts inside it, in front of its eleventh and
    // twenty-first box (the breaks are items, too: the box number n is the item 2 * n)
    assert_eq!(pages, [START, in_container(0, 20), in_container(0, 40)]);
    let (_, fills) = container_pages(container(25, 2, ColumnFill::Auto), 100.0);
    assert_eq!(used(&fills), [100.0, 100.0, 100.0]);
}

#[test]
fn only_the_last_fragment_is_balanced_unless_all_of_them_are() {
    // a forced break of the page level after the sixth box: the first fragment has six boxes
    let content = |fill| {
        let mut inner = items(6);
        inner.break_at(BreakKind::Forced, BreakLevel::Page);
        inner.content(20.0);
        let mut flow = GalleyFlow::new();
        flow.container(inner, 2, fill, (0.0, 0.0));
        flow
    };
    // (the sixth box is in the first column of five and the second: not balanced)
    let (_, fills) = container_pages(content(ColumnFill::Balance), 100.0);
    assert_eq!(used(&fills), [100.0, 20.0]);
    assert_eq!(fills[0].end_kind, BreakKind::Forced);
    let (_, fills) = container_pages(content(ColumnFill::BalanceAll), 100.0);
    assert_eq!(used(&fills), [60.0, 20.0]);
    assert_eq!(fills[0].end_kind, BreakKind::Forced);
}

#[test]
fn a_forced_page_break_in_a_container_continues_it_on_the_next_page() {
    // three boxes, a break, three boxes
    let mut inner = items(3);
    inner.break_at(BreakKind::Forced, BreakLevel::Page);
    inner.content(20.0).break_at(ALLOWED, BreakLevel::Column).content(20.0);
    inner.break_at(ALLOWED, BreakLevel::Column).content(20.0);
    let mut flow = GalleyFlow::new();
    flow.container(inner, 2, ColumnFill::Auto, (0.0, 0.0));
    let (pages, fills) = container_pages(flow, 100.0);
    // (items: box, break, box, break, box, forced break, box, ...)
    assert_eq!(pages, [START, in_container(0, 6)]);
    assert_eq!(used(&fills), [60.0, 60.0]);
}

#[test]
fn a_container_in_the_middle_of_a_page_has_what_is_left_of_it() {
    // 40 of content in front of the container leave 60: three boxes per column
    let mut flow = GalleyFlow::new();
    flow.content(40.0).break_(ALLOWED);
    let mut inner = items(8);
    inner.break_at(ALLOWED, BreakLevel::Column);
    flow.container(inner, 2, ColumnFill::Auto, (0.0, 0.0));
    let (pages, fills) = container_pages(flow, 100.0);
    // the first page has the 40 and a full fragment of six boxes, the second the other three
    assert_eq!(used(&fills), [100.0, 40.0]);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1], in_container(2, 12));
}

#[test]
fn the_content_after_a_container_starts_where_its_last_fragment_ends() {
    let mut flow = GalleyFlow::new();
    flow.container(items(12), 2, ColumnFill::Balance, (0.0, 0.0));
    flow.break_(ALLOWED).content(30.0);
    let (pages, fills) = container_pages(flow, 100.0);
    // two columns of five and a fragment of two boxes, balanced (one and one), and the
    // box after it
    assert_eq!(used(&fills), [100.0, 20.0 + 30.0]);
    assert_eq!(pages.len(), 2);
}

#[test]
fn a_container_that_cannot_hold_a_box_gets_a_page_of_its_own_for_it() {
    // a box of 150 in a container on a page of 100
    let mut inner = GalleyFlow::new();
    inner.content(150.0);
    let mut flow = GalleyFlow::new();
    flow.content(10.0).break_(ALLOWED);
    flow.container(inner, 2, ColumnFill::Balance, (0.0, 0.0));
    let (pages, fills) = container_pages(flow, 100.0);
    // the container does not start on the first page: nothing of it fits
    assert_eq!(pages.len(), 2);
    assert_eq!(used(&fills), [10.0, 150.0]);
    assert!(fills[1].overflow);
}

#[test]
fn the_padding_of_a_container_is_part_of_its_first_and_last_fragment() {
    // an edge of 10 on top and 5 at the bottom
    let mut flow = GalleyFlow::new();
    flow.container(items(15), 2, ColumnFill::Balance, (10.0, 5.0));
    let (_, fills) = container_pages(flow, 100.0);
    // 15 boxes: the first fragment has 90 for its columns (the edge is on top): four boxes in
    // each, 80 and 10 for the edge. The seven boxes that are left are balanced as four and three,
    // 80 and the edge at the bottom: 85.
    assert_eq!(used(&fills), [90.0, 85.0]);
}

#[test]
fn the_fragments_that_a_page_has_are_listed_for_its_layout() {
    let mut flow = GalleyFlow::new();
    flow.content(40.0).break_(ALLOWED);
    flow.container(items(8), 2, ColumnFill::Auto, (0.0, 0.0));
    let (problem, mut service) = flow.finish(page(100.0));
    let mut kernel = Kernel::new(&problem, &mut service);
    let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
    let pages = kernel.realize(&solution).unwrap();
    let first = kernel.fragments(&solution.pages[0], pages[0].end);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].from, None);
    assert_eq!(first[0].available, 60.0);
    assert_eq!(first[0].end, Some(ContainerCut::at_item(12)));
    let second = kernel.fragments(&solution.pages[1], pages[1].end);
    assert_eq!(second[0].from, Some(ContainerCut::at_item(12)));
    assert_eq!(second[0].available, 100.0);
    assert_eq!(second[0].end, None);
}
