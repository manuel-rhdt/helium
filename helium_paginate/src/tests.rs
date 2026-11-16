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

/// The fills of the page that starts at the document start or at the token `at`.
fn fills(flow: GalleyFlow, block_size: f32, at: usize) -> Vec<PageFill> {
    let (problem, mut service) = flow.finish(page(block_size));
    let mut kernel = Kernel::new(&problem, &mut service);
    let start = PageStart {
        at: BreakToken(at as u32),
        index: at.min(1),
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
    assert_eq!(used(&fills(flow.clone(), 100.0, 0)), [25.0, 40.0]);
    // the gaps at the start and at the end of the second page go
    assert_eq!(used(&fills(flow, 100.0, 1)), [10.0]);
}

#[test]
fn a_gap_in_front_of_a_break_does_not_count_against_the_page() {
    let mut flow = GalleyFlow::new();
    flow.content(20.0).gap(5.0).break_(ALLOWED).content(20.0);
    let fills = fills(flow, 20.0, 0);
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
    assert_eq!(used(&fills(flow.clone(), 100.0, 0)), [13.0]);
    assert_eq!(used(&fills(flow, 100.0, 1)), [15.0]);
}

#[test]
fn a_gap_after_a_break_that_is_not_forced_is_discarded_even_if_it_follows_an_avoid() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::Avoid)
        .gap(5.0)
        .content(10.0);
    assert_eq!(used(&fills(flow, 100.0, 1)), [10.0]);
}

fn paragraph(lines: usize, orphans: u32, widows: u32) -> GalleyFlow {
    let mut flow = GalleyFlow::new();
    flow.paragraph(&vec![10.0; lines], orphans, widows, false);
    flow
}

#[test]
fn orphans_are_counted_on_the_page_that_ends_in_the_paragraph() {
    // a paragraph of five lines, cut after 1, 2, 3 and 4 lines, and its end
    let fills = fills(paragraph(5, 2, 2), 100.0, 0);
    assert_eq!(used(&fills), [10.0, 20.0, 30.0, 40.0, 50.0]);
    let orphans: Vec<_> = fills.iter().map(|fill| fill.orphan_violation).collect();
    assert_eq!(orphans, [true, false, false, false, false]);
    // the start of the paragraph is not a cut: no widow can be left behind
    assert!(fills.iter().all(|fill| !fill.widow_violation));
}

#[test]
fn widows_are_counted_on_the_page_that_starts_in_the_paragraph() {
    // the page that starts in front of the fourth line has two lines of the paragraph left
    let at_3 = fills(paragraph(5, 2, 2), 100.0, 3);
    assert_eq!(used(&at_3), [10.0, 20.0]);
    // the fifth line alone ends the paragraph: it is a widow, and the page that ends in front
    // of it has the right number of lines
    let at_4 = fills(paragraph(5, 2, 2), 100.0, 4);
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
    let tokens: Vec<_> = kernel.tokens().collect();
    // start, four cuts of the first paragraph, the break, four cuts of the second, the end
    assert_eq!(tokens.len(), 11);
    let fills = kernel.candidates(&PageStart {
        at: tokens[5],
        index: 1,
    });
    assert_eq!(fills.len(), 5);
    assert!(fills[0].orphan_violation);
    assert!(!fills[1].orphan_violation && !fills[1].widow_violation);
    // from the fourth cut of the first paragraph: the lines on this page are the last one
    let fills = kernel.candidates(&PageStart {
        at: tokens[4],
        index: 1,
    });
    assert!(fills[0].widow_violation);
    assert!(!fills[0].orphan_violation);
}

#[test]
fn cuts_in_a_paragraph_that_avoids_breaks_inside_are_avoid_breaks() {
    let mut flow = GalleyFlow::new();
    flow.paragraph(&[10.0; 3], 1, 1, true);
    let kinds: Vec<_> = fills(flow, 100.0, 0)
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
    let first = fills(flow.clone(), 100.0, 0);
    // the second item does not fit: the page stops at the first break
    assert_eq!(used(&first), [50.0]);
    let second = fills(flow.clone(), 100.0, 1);
    assert_eq!(used(&second), [150.0]);
    assert!(second[0].overflow);
    // the page after it is a normal page
    let third = fills(flow, 100.0, 2);
    assert_eq!(used(&third), [50.0]);
    assert!(!third[0].overflow);
}

#[test]
fn an_overflowing_end_of_the_document_is_listed_when_nothing_else_is() {
    let mut flow = GalleyFlow::new();
    flow.content(150.0);
    let fills = fills(flow, 100.0, 0);
    assert_eq!(fills.len(), 1);
    assert!(fills[0].overflow);
}

#[test]
fn forced_breaks_end_the_list_of_candidates_if_the_problem_says_so() {
    let mut flow = GalleyFlow::new();
    flow.content(10.0)
        .break_(BreakKind::Forced)
        .content(10.0)
        .break_(ALLOWED)
        .content(10.0);
    let (mut problem, mut service) = flow.finish(page(100.0));
    assert!(problem.stop_at_forced);
    let mut kernel = Kernel::new(&problem, &mut service);
    let first = kernel.first_page();
    assert_eq!(used(&kernel.candidates(&first)), [10.0]);
    problem.stop_at_forced = false;
    let mut kernel = Kernel::new(&problem, &mut service);
    assert_eq!(used(&kernel.candidates(&first)), [10.0, 20.0, 30.0]);
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

fn solve(flow: GalleyFlow, block_size: f32) -> (Vec<usize>, Vec<PageFill>) {
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
            .map(|token| token.0 as usize)
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
    assert_eq!(starts, [0, 2]);
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
    assert_eq!(starts, [0, 1]);
}

#[test]
fn the_exact_solver_keeps_the_widows_of_a_paragraph() {
    // five lines on pages of three: [3] [2] has no violation, [4] [1] a widow
    let (starts, pages) = solve(paragraph(5, 2, 2), 40.0);
    assert_eq!(starts, [0, 3]);
    assert!(
        pages
            .iter()
            .all(|fill| !fill.orphan_violation && !fill.widow_violation)
    );
    // the cut after the second line is as good as the one after the third on pages of two
    // lines, and the better filled first page wins
    let (starts, _) = solve(paragraph(5, 2, 2), 30.0);
    assert_eq!(starts, [0, 3]);
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
    assert_eq!(starts, [0, 1, 2]);
    assert!(pages[1].overflow);
}

#[test]
fn an_empty_document_has_one_page_that_realizes() {
    let (starts, pages) = solve(GalleyFlow::new(), 100.0);
    assert_eq!(starts, [0]);
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
    let tokens: Vec<_> = kernel.tokens().collect();
    let solution = |pages: &[usize]| Solution {
        pages: pages.iter().map(|&n| tokens[n]).collect(),
    };
    assert!(kernel.realize(&solution(&[0, 1, 2])).is_ok());
    // not from the start
    assert_eq!(kernel.realize(&solution(&[1, 2])), Err(Invalid::FirstPage));
    // two items do not fit on a page
    assert_eq!(
        kernel.realize(&solution(&[0, 2])),
        Err(Invalid::NotACandidate { page: 0 })
    );
    // going backwards
    assert_eq!(
        kernel.realize(&solution(&[0, 1, 1])),
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
fn random_problem(rng: &mut Rng) -> (Problem, GalleyService, Reference) {
    let mut flow = GalleyFlow::new();
    let mut advance = vec![0.0f32];
    let mut y = 0.0;
    let mut forced = vec![false];
    let mut avoid = vec![false];
    let mut in_lines = vec![None];
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
    let (mut problem, service) = flow.finish(page(0.0));
    problem.stop_at_forced = false;
    if problem.items.is_empty() {
        return random_problem(rng);
    }
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
    )
}

#[test]
fn the_exact_solver_chooses_what_the_least_weight_subsequence_chose() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for _ in 0..3000 {
        let (mut problem, mut service, reference) = random_problem(&mut rng);
        let positions: Vec<_> = {
            let kernel = Kernel::new(&problem, &mut service);
            kernel
                .tokens()
                .map(|token| kernel.position(token))
                .collect()
        };
        assert_eq!(positions.len(), reference.advance.len());
        for (position, &block_size) in positions.iter().zip(&reference.heights) {
            problem.pages.starts.insert(
                *position,
                PageSpec {
                    inline_size: 0.0,
                    block_size,
                },
            );
        }
        let mut kernel = Kernel::new(&problem, &mut service);
        let solution = ExactSolver::new(CssObjective).solve(&mut kernel);
        let chosen: Vec<usize> = solution
            .pages
            .iter()
            .map(|token| token.0 as usize)
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
