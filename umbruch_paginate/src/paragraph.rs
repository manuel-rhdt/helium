//! Contains an algorithm for Knuth-style optimal paragraph formatting

#[derive(Default, Copy, Clone)]
struct Node {
    position: usize,
    weight: f64,
    previous: Option<usize>,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct BreakOpportunity {
    pub forced: bool,
    pub hyphen: bool,
    pub penalty: i32,
    /// For the last opportunity of a paragraph: the demerits of its last line for following a line
    /// that ends in a hyphen (TeX's `\finalhyphendemerits`; [`FINAL_HYPHEN_DEMERITS`] if `None`).
    pub final_hyphen_demerits: Option<f64>,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct GlueTotals {
    pub stretch: f64,
    pub shrink: f64,
}

#[derive(Debug, Copy, Clone)]
struct ParagraphNode {
    demerits: f64,
    /// The badness of the line that ends at the node.
    badness: f64,
    previous: Option<(usize, usize)>,
}

const FITNESS_CLASSES: usize = 4;
const HYPHEN_STATES: usize = 2;
const STATE_COUNT: usize = FITNESS_CLASSES * HYPHEN_STATES;
const LINE_PENALTY: f64 = 10.0;
const ADJACENT_FITNESS_DEMERITS: f64 = 3_000.0;
const DOUBLE_HYPHEN_DEMERITS: f64 = 10_000.0;
/// What the line before the last line of a paragraph has for ending in a hyphen, as in TeX.
pub const FINAL_HYPHEN_DEMERITS: f64 = 5_000.0;

/// Choose paragraph breaks using adjustment-ratio badness and TeX-style demerits.
///
/// `line_metrics` returns the natural width and the total stretch and shrink of glue strictly
/// inside the line. Glue at a break is discarded by the caller and must not be included.
pub fn compute_paragraph_breakpoints(
    opportunities: &[BreakOpportunity],
    line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    line_width: impl FnMut(usize) -> f64,
) -> Vec<usize> {
    compute_paragraph_lines(opportunities, line_metrics, line_width, LineStart::PARAGRAPH, true).breaks
}

/// What the line in front of the first line is, for a paragraph that does not start where the
/// breaking starts: the breaker looks at the line before for the demerits of two lines of very
/// different tightness in a row and of two hyphens in a row, as TeX does within a paragraph.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LineStart {
    /// The fitness class of the line in front (0 tight, 1 decent, 2 loose, 3 very loose).
    pub fitness: u8,
    /// Whether the line in front ends in a hyphen.
    pub hyphen: bool,
}

impl LineStart {
    /// The fitness class of a line that is neither tight nor loose, which is what the line in front
    /// of the first line of a paragraph is taken to be.
    pub const DECENT: u8 = 1;
    /// The start of a paragraph: there is no line in front.
    pub const PARAGRAPH: LineStart = LineStart {
        fitness: Self::DECENT,
        hyphen: false,
    };
}

/// The breaks of a paragraph, and how every line fits.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphLines {
    /// The indices of the break opportunities where lines start and end: the first is 0.
    pub breaks: Vec<usize>,
    /// The fitness class of every line: the line that ends at `breaks[i + 1]` has `fitness[i]`.
    pub fitness: Vec<u8>,
    /// The demerits of every line, counted as the breaker counts them: the demerits of a
    /// line include those that it has for following a line of another fitness class and for
    /// following a hyphen, so that the demerits of the lines of a page are the demerits of that
    /// part of the paragraph. A line that no breaking can have has no demerits that mean
    /// anything (`f64::INFINITY`).
    pub demerits: Vec<f64>,
    /// The badness of every line, which TeX's `\tolerance` is about: 100 times the cube of how far
    /// its spaces stretch or shrink (the adjustment ratio), at most 10 000. The last line of a
    /// paragraph that is allowed to be short has none.
    pub badness: Vec<f64>,
}

impl ParagraphLines {
    /// The number of lines.
    pub fn lines(&self) -> usize {
        self.breaks.len().saturating_sub(1)
    }

    /// The demerits of all the lines.
    pub fn total_demerits(&self) -> f64 {
        self.demerits.iter().sum()
    }
}

/// Like [`compute_paragraph_breakpoints`], for a paragraph that follows a line `start`, and with
/// the fitness class of every line, so that the lines in front of a cut can be given to the next
/// call as its `start`.
///
/// `ends_paragraph` is whether the last opportunity is the end of the paragraph. If it is not,
/// the part of a paragraph that ends at a cut is broken: its last line is a line like the
/// others (it is not a short last line that is allowed to be ragged, and it has no demerits for
/// following a hyphen as the last line of a paragraph has), and it ends at the last opportunity.
pub fn compute_paragraph_lines(
    opportunities: &[BreakOpportunity],
    line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize) -> f64,
    start: LineStart,
    ends_paragraph: bool,
) -> ParagraphLines {
    compute_paragraph_lines_shaped(opportunities, line_metrics, |_, from| line_width(from), 0, start, ends_paragraph)
}

/// Like [`compute_paragraph_lines`], for a paragraph whose first `shaped` lines have widths of
/// their own, like the lines of TeX's `\parshape` (a paragraph that wraps around a float).
/// `line_width(line, from)` is the width of the line that is the number `line` (counted from the
/// start of the breaking) and starts at the break `from`; it is asked for the numbers up to
/// `shaped` only, and the width of `shaped` is the one of all the lines after the shaped ones.
///
/// The search keeps the number of the line, up to `shaped`, as a part of its state, so it takes
/// `shaped + 1` times as long (and as long as before where there is no shape).
pub fn compute_paragraph_lines_shaped(
    opportunities: &[BreakOpportunity],
    mut line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize, usize) -> f64,
    shaped: usize,
    start: LineStart,
    ends_paragraph: bool,
) -> ParagraphLines {
    if opportunities.len() < 2 {
        return ParagraphLines {
            breaks: opportunities.first().map_or_else(Vec::new, |_| vec![0]),
            fitness: vec![],
            demerits: vec![],
            badness: vec![],
        };
    }

    // (a state is the number of the line, up to `shaped`, and then the fitness class and the hyphen)
    let slots = STATE_COUNT * (shaped + 1);
    let mut paths = vec![vec![None; slots]; opportunities.len()];
    let mut forced_prefix = vec![0usize; opportunities.len() + 1];
    for (index, opportunity) in opportunities.iter().enumerate() {
        forced_prefix[index + 1] = forced_prefix[index] + usize::from(opportunity.forced);
    }
    let line_widths: Vec<Vec<f64>> = (0..=shaped)
        .map(|line| (0..opportunities.len() - 1).map(|from| line_width(line, from)).collect())
        .collect();
    let initial_state = state_index(usize::from(start.fitness).min(FITNESS_CLASSES - 1), start.hyphen);
    paths[0][initial_state] = Some(ParagraphNode {
        demerits: 0.0,
        badness: 0.0,
        previous: None,
    });

    for to in 1..opportunities.len() {
        for from in 0..to {
            if forced_prefix[to] != forced_prefix[from + 1] {
                continue;
            }
            let Some((natural_width, glue)) = line_metrics(from, to) else {
                continue;
            };
            for line in 0..=shaped {
            let delta = line_widths[line][from] - natural_width;
            // A line that cannot be shortened enough is overfull. Like TeX does in its last
            // resort, accept it only for the smallest piece that can end a line, so that a word
            // wider than the line gets a line of its own with the worst possible badness.
            let overfull = delta < 0.0 && (glue.shrink <= 0.0 || -delta > glue.shrink);
            if overfull && to != from + 1 {
                continue;
            }
            let ragged_final_line = ends_paragraph && to + 1 == opportunities.len() && delta >= 0.0;
            let ratio = if ragged_final_line {
                0.0
            } else {
                adjustment_ratio(delta, glue)
            };
            let badness = if ragged_final_line {
                0.0
            } else {
                line_badness(ratio)
            };
            let fitness = fitness_class(ratio);
            let opportunity = opportunities[to];
            let next_line = (line + 1).min(shaped);

            for previous_state in 0..STATE_COUNT {
                let previous_slot = line * STATE_COUNT + previous_state;
                let Some(previous) = paths[from][previous_slot] else {
                    continue;
                };
                let (previous_fitness, previous_hyphen) = decode_state(previous_state);
                // like in TeX the penalty of a break counts towards the badness of the line before
                // it is squared: a hyphen is worth a looser line
                let penalty = if opportunity.penalty > 0 && opportunity.penalty < 10_000 {
                    f64::from(opportunity.penalty)
                } else {
                    0.0
                };
                let mut demerits = previous.demerits + (LINE_PENALTY + badness + penalty).powi(2);

                if opportunity.penalty >= 10_000 {
                    demerits += f64::from(opportunity.penalty).powi(2);
                } else if opportunity.penalty < 0 && opportunity.penalty > -10_000 {
                    demerits -= f64::from(opportunity.penalty).powi(2);
                }
                if fitness.abs_diff(previous_fitness) > 1 {
                    demerits += ADJACENT_FITNESS_DEMERITS;
                }
                if previous_hyphen && opportunity.hyphen {
                    demerits += DOUBLE_HYPHEN_DEMERITS;
                }
                if ends_paragraph && to + 1 == opportunities.len() && previous_hyphen {
                    demerits += opportunity.final_hyphen_demerits.unwrap_or(FINAL_HYPHEN_DEMERITS);
                }

                let next_slot = next_line * STATE_COUNT + state_index(fitness, opportunity.hyphen);
                let replace =
                    paths[to][next_slot].map_or(true, |current| demerits < current.demerits);
                if replace {
                    paths[to][next_slot] = Some(ParagraphNode {
                        demerits,
                        badness,
                        previous: Some((from, previous_slot)),
                    });
                }
            }
            }
        }
    }

    let last = opportunities.len() - 1;
    let Some((mut state, _)) = paths[last]
        .iter()
        .enumerate()
        .filter_map(|(state, node)| node.map(|node| (state, node.demerits)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
    else {
        return ParagraphLines {
            breaks: vec![0, last],
            fitness: vec![LineStart::DECENT],
            demerits: vec![f64::INFINITY],
            badness: vec![f64::INFINITY],
        };
    };

    let mut index = last;
    let mut sequence = vec![last];
    let mut fitness = vec![decode_state(state % STATE_COUNT).0 as u8];
    let mut cumulative = vec![paths[last][state].map_or(0.0, |node| node.demerits)];
    let mut badness = vec![paths[last][state].map_or(0.0, |node| node.badness)];
    while let Some((previous_index, previous_state)) =
        paths[index][state].and_then(|node| node.previous)
    {
        index = previous_index;
        state = previous_state;
        sequence.push(index);
        fitness.push(decode_state(state % STATE_COUNT).0 as u8);
        cumulative.push(paths[index][state].map_or(0.0, |node| node.demerits));
        badness.push(paths[index][state].map_or(0.0, |node| node.badness));
    }
    // (the last of them is the state in front of the first line)
    fitness.pop();
    badness.pop();
    sequence.reverse();
    fitness.reverse();
    badness.reverse();
    cumulative.reverse();
    let demerits = cumulative.windows(2).map(|window| window[1] - window[0]).collect();
    ParagraphLines {
        breaks: sequence,
        fitness,
        demerits,
        badness,
    }
}

#[derive(Debug, Copy, Clone)]
struct CountedNode {
    demerits: f64,
    /// The badness of the line that ends at the node.
    badness: f64,
    /// The break, the state and the number of lines of the node in front.
    previous: Option<(usize, usize, usize)>,
}

/// The breakings of a paragraph that have the least demerits for their number of lines, for the
/// number of lines of the optimum and `extra` lines fewer and more, like what TeX's `\looseness`
/// asks for.
///
/// The first of them is the optimum, as [`compute_paragraph_lines`] finds it. The others follow
/// by their number of lines. A number of lines that no breaking has (the lines cannot be made
/// longer or shorter than they are, or a forced break needs more) has no breaking in the result.
///
/// The search is the one of [`compute_paragraph_lines`] with the number of lines as a part of the
/// state. It takes `extra` + the number of lines of the optimum times as long.
pub fn compute_paragraph_variants(
    opportunities: &[BreakOpportunity],
    line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize) -> f64,
    start: LineStart,
    ends_paragraph: bool,
    extra: u32,
) -> Vec<ParagraphLines> {
    compute_paragraph_variants_shaped(opportunities, line_metrics, |_, from| line_width(from), 0, start, ends_paragraph, extra)
}

/// Like [`compute_paragraph_variants`], for a paragraph whose first `shaped` lines have widths of
/// their own (see [`compute_paragraph_lines_shaped`]).
pub fn compute_paragraph_variants_shaped(
    opportunities: &[BreakOpportunity],
    mut line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize, usize) -> f64,
    shaped: usize,
    start: LineStart,
    ends_paragraph: bool,
    extra: u32,
) -> Vec<ParagraphLines> {
    let optimum =
        compute_paragraph_lines_shaped(opportunities, &mut line_metrics, &mut line_width, shaped, start, ends_paragraph);
    let optimum_lines = optimum.lines();
    if extra == 0 || opportunities.len() < 2 || optimum_lines == 0 || optimum.demerits.iter().any(|d| d.is_infinite()) {
        return vec![optimum];
    }
    let max_lines = optimum_lines + extra as usize;
    let min_lines = optimum_lines.saturating_sub(extra as usize).max(1);
    let count = opportunities.len();
    let width = max_lines + 1;
    let slot = |state: usize, lines: usize| state * width + lines;

    let mut paths: Vec<Vec<Option<CountedNode>>> = vec![vec![None; STATE_COUNT * width]; count];
    let mut forced_prefix = vec![0usize; count + 1];
    for (index, opportunity) in opportunities.iter().enumerate() {
        forced_prefix[index + 1] = forced_prefix[index] + usize::from(opportunity.forced);
    }
    let line_widths: Vec<Vec<f64>> =
        (0..=shaped).map(|line| (0..count - 1).map(|from| line_width(line, from)).collect()).collect();
    let initial_state = state_index(usize::from(start.fitness).min(FITNESS_CLASSES - 1), start.hyphen);
    paths[0][slot(initial_state, 0)] = Some(CountedNode {
        demerits: 0.0,
        badness: 0.0,
        previous: None,
    });

    for to in 1..count {
        for from in 0..to {
            if forced_prefix[to] != forced_prefix[from + 1] {
                continue;
            }
            let Some((natural_width, glue)) = line_metrics(from, to) else {
                continue;
            };
            let opportunity = opportunities[to];

            for previous_state in 0..STATE_COUNT {
                let (previous_fitness, previous_hyphen) = decode_state(previous_state);
                for lines_before in 0..max_lines {
                    let Some(previous) = paths[from][slot(previous_state, lines_before)] else {
                        continue;
                    };
                    let delta = line_widths[lines_before.min(shaped)][from] - natural_width;
                    let overfull = delta < 0.0 && (glue.shrink <= 0.0 || -delta > glue.shrink);
                    if overfull && to != from + 1 {
                        continue;
                    }
                    let ragged_final_line = ends_paragraph && to + 1 == count && delta >= 0.0;
                    let ratio = if ragged_final_line { 0.0 } else { adjustment_ratio(delta, glue) };
                    let badness = if ragged_final_line { 0.0 } else { line_badness(ratio) };
                    let fitness = fitness_class(ratio);
                    // (the same demerits as in compute_paragraph_lines)
                    let penalty = if opportunity.penalty > 0 && opportunity.penalty < 10_000 {
                        f64::from(opportunity.penalty)
                    } else {
                        0.0
                    };
                    let mut demerits = previous.demerits + (LINE_PENALTY + badness + penalty).powi(2);
                    if opportunity.penalty >= 10_000 {
                        demerits += f64::from(opportunity.penalty).powi(2);
                    } else if opportunity.penalty < 0 && opportunity.penalty > -10_000 {
                        demerits -= f64::from(opportunity.penalty).powi(2);
                    }
                    if fitness.abs_diff(previous_fitness) > 1 {
                        demerits += ADJACENT_FITNESS_DEMERITS;
                    }
                    if previous_hyphen && opportunity.hyphen {
                        demerits += DOUBLE_HYPHEN_DEMERITS;
                    }
                    if ends_paragraph && to + 1 == count && previous_hyphen {
                        demerits += opportunity.final_hyphen_demerits.unwrap_or(FINAL_HYPHEN_DEMERITS);
                    }

                    let next = slot(state_index(fitness, opportunity.hyphen), lines_before + 1);
                    let replace = paths[to][next].map_or(true, |current| demerits < current.demerits);
                    if replace {
                        paths[to][next] = Some(CountedNode {
                            demerits,
                            badness,
                            previous: Some((from, previous_state, lines_before)),
                        });
                    }
                }
            }
        }
    }

    let last = count - 1;
    let mut variants = vec![optimum];
    for lines in min_lines..=max_lines {
        if lines == optimum_lines {
            continue;
        }
        let Some((mut state, _)) = (0..STATE_COUNT)
            .filter_map(|state| paths[last][slot(state, lines)].map(|node| (state, node.demerits)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
        else {
            continue;
        };
        let (mut index, mut lines_left) = (last, lines);
        let mut breaks = vec![last];
        let mut fitness = vec![decode_state(state).0 as u8];
        let mut cumulative = vec![paths[last][slot(state, lines)].map_or(0.0, |node| node.demerits)];
        let mut badness = vec![paths[last][slot(state, lines)].map_or(0.0, |node| node.badness)];
        while let Some((previous_index, previous_state, previous_lines)) =
            paths[index][slot(state, lines_left)].and_then(|node| node.previous)
        {
            index = previous_index;
            state = previous_state;
            lines_left = previous_lines;
            breaks.push(index);
            fitness.push(decode_state(state).0 as u8);
            cumulative.push(paths[index][slot(state, lines_left)].map_or(0.0, |node| node.demerits));
            badness.push(paths[index][slot(state, lines_left)].map_or(0.0, |node| node.badness));
        }
        fitness.pop();
        badness.pop();
        breaks.reverse();
        fitness.reverse();
        badness.reverse();
        cumulative.reverse();
        let demerits = cumulative.windows(2).map(|window| window[1] - window[0]).collect();
        variants.push(ParagraphLines {
            breaks,
            fitness,
            demerits,
            badness,
        });
    }
    variants
}

fn state_index(fitness: usize, hyphen: bool) -> usize {
    fitness * HYPHEN_STATES + usize::from(hyphen)
}

fn decode_state(state: usize) -> (usize, bool) {
    (state / HYPHEN_STATES, state % HYPHEN_STATES != 0)
}

fn adjustment_ratio(delta: f64, glue: GlueTotals) -> f64 {
    if delta >= 0.0 {
        if delta == 0.0 {
            0.0
        } else if glue.stretch > 0.0 {
            delta / glue.stretch
        } else {
            10.0
        }
    } else if glue.shrink > 0.0 {
        delta / glue.shrink
    } else {
        -10.0
    }
}

fn line_badness(ratio: f64) -> f64 {
    (100.0 * ratio.abs().powi(3)).min(10_000.0)
}

fn fitness_class(ratio: f64) -> usize {
    if ratio < -0.5 {
        0
    } else if ratio <= 0.5 {
        1
    } else if ratio <= 1.0 {
        2
    } else {
        3
    }
}

/// Use the Knuth-Plass paragraph layout algorithm to compute the optimal breakpoints.
///
/// # Discussion
///
/// The problem solved by this function is know in the literature as the "least-weight subsequence
/// problem". Here we employ an algorithm to solve this problem in the context of paragraph layout.
///
/// The input to this function is a paragraph and a list of possible break points. The output of
/// this function is the sequence of break points that lead to the best paragraph layout.
///
/// # Arguments
///
/// - `num_breakpoints` is the total number of possible break points.
/// - `weight` should be a function that for a given pair of break points computes the total
///   demerits of the line between these break points.
///
/// # Returns
///
/// The return value is a list of break points that has the minimum weight.
pub fn compute_breakpoints(
    num_breakpoints: usize,
    weight: &mut dyn FnMut(usize, usize) -> f64,
) -> Vec<usize> {
    let mut nodes: Vec<Node> = Vec::with_capacity(num_breakpoints);
    let mut active_nodes: Vec<usize> = vec![0];

    nodes.push(Node {
        position: 0,
        weight: 0.0,
        previous: None,
    });

    for i in 1..num_breakpoints {
        let mut j = 0;

        let mut minimal_observed_weight = f64::INFINITY;
        let mut optimal_node = None;

        // TODO: Use drain_filter once it is stable
        while j != active_nodes.len() {
            let node_index = active_nodes[j];
            let node = nodes[node_index];

            let weight_ij = weight(node.position, i);
            let forced_break = weight_ij == f64::NEG_INFINITY;

            // a forced break adds no demerits for the segment before it, but the best way to get
            // to the start of that segment still has to be chosen
            let current_weight = if forced_break {
                node.weight
            } else {
                weight_ij + node.weight
            };
            if current_weight.is_infinite() || forced_break {
                active_nodes.remove(j);
            } else {
                j += 1;
            }

            if current_weight < minimal_observed_weight {
                minimal_observed_weight = current_weight;
                optimal_node = Some(Node {
                    position: i,
                    weight: current_weight,
                    previous: Some(node_index),
                });
            }
        }

        if let Some(best_node) = optimal_node {
            nodes.push(best_node);
            active_nodes.push(nodes.len() - 1);
        }
    }

    // find all the ancestors of the last breakpoint
    let mut node = nodes[active_nodes.last().cloned().unwrap_or(0)];
    let mut result = vec![];
    loop {
        result.push(node.position);
        if let Some(index) = node.previous {
            node = nodes[index];
        } else {
            break;
        }
    }
    result.reverse();
    result
}

#[cfg(test)]
mod test {
    use super::*;

    /// Calculates the demerits for a line between the breakpoints `from` and `to`.
    fn line_demerits(from: usize, to: usize, offsets: &[f64], line_width: f64) -> f64 {
        let natural_width = offsets[to] - offsets[from];
        (natural_width - line_width).powi(2)
    }

    /// Words of equal width (10) with a space between: a line of width 40 holds 3 words.
    fn uniform_words(count: usize) -> (Vec<BreakOpportunity>, Vec<f64>) {
        let opportunities = vec![BreakOpportunity::default(); count + 1];
        let advance = (0..=count).map(|index| index as f64 * 10.0).collect();
        (opportunities, advance)
    }

    #[test]
    fn the_first_lines_can_have_a_width_of_their_own() {
        let (opportunities, advance) = uniform_words(12);
        let metrics = |from: usize, to: usize| {
            Some((advance[to] - advance[from], GlueTotals { stretch: 0.0, shrink: 0.0 }))
        };
        // two lines of 2 words, then lines of 4 words
        let widths = |line: usize, _from: usize| if line < 2 { 20.0 } else { 40.0 };
        let lines = compute_paragraph_lines_shaped(&opportunities, metrics, widths, 2, LineStart::PARAGRAPH, true);
        assert_eq!(lines.breaks, vec![0, 2, 4, 8, 12]);
        // (without a shape the lines are the ones of the width after it)
        let plain = compute_paragraph_lines_shaped(&opportunities, metrics, |_, _| 40.0, 0, LineStart::PARAGRAPH, true);
        assert_eq!(plain.breaks, vec![0, 4, 8, 12]);
        // (and the lines that count their number agree)
        let variants = compute_paragraph_variants_shaped(&opportunities, metrics, widths, 2, LineStart::PARAGRAPH, true, 1);
        assert_eq!(variants[0].breaks, lines.breaks);
    }

    #[test]
    fn test_1() {
        let mut widths = vec![];
        for i in 1..=12 {
            match i % 2 {
                0 => widths.push(4.0),
                _ => widths.push(3.0),
            }
        }
        let mut offsets: Vec<_> = widths
            .into_iter()
            .scan(0.0, |a, b| {
                *a += b;
                Some(*a)
            })
            .collect();
        offsets.insert(0, 0.0);

        let sequence = compute_breakpoints(offsets.len(), &mut |from, to| {
            line_demerits(from, to, &offsets, 15.0)
        });

        println!("{:?}", sequence);
    }

    /// A paragraph for the tests of the start of the breaking: the widths of the words, which
    /// opportunities are hyphens, and the width of the lines. The spaces stretch and shrink.
    struct Paragraph {
        opportunities: Vec<BreakOpportunity>,
        advance: Vec<f64>,
        width: f64,
    }

    impl Paragraph {
        fn metrics(&self, from: usize, to: usize) -> Option<(f64, GlueTotals)> {
            let spaces = (to - from - 1) as f64;
            Some((
                self.advance[to] - self.advance[from],
                GlueTotals { stretch: 1.5 * spaces + 1.0, shrink: 0.5 * spaces },
            ))
        }

        fn lines(&self, start: LineStart) -> ParagraphLines {
            self.lines_of(start, true)
        }

        fn lines_of(&self, start: LineStart, ends_paragraph: bool) -> ParagraphLines {
            compute_paragraph_lines(&self.opportunities, |from, to| self.metrics(from, to), |_| self.width, start, ends_paragraph)
        }

        /// The demerits of a sequence of breaks, as the breaker counts them, or `None` if the
        /// breaker does not allow it.
        fn demerits(&self, breaks: &[usize], start: LineStart) -> Option<(f64, Vec<u8>)> {
            self.demerits_of(breaks, start, true)
        }

        fn demerits_of(&self, breaks: &[usize], start: LineStart, ends_paragraph: bool) -> Option<(f64, Vec<u8>)> {
            let last = self.opportunities.len() - 1;
            let (mut fitness, mut hyphen) = (usize::from(start.fitness), start.hyphen);
            let mut total = 0.0;
            let mut classes = vec![];
            for window in breaks.windows(2) {
                let (from, to) = (window[0], window[1]);
                if self.opportunities[from + 1..to].iter().any(|opportunity| opportunity.forced) {
                    return None;
                }
                let (natural, glue) = self.metrics(from, to)?;
                let delta = self.width - natural;
                let overfull = delta < 0.0 && (glue.shrink <= 0.0 || -delta > glue.shrink);
                if overfull && to != from + 1 {
                    return None;
                }
                let ragged_final_line = ends_paragraph && to == last && delta >= 0.0;
                let ratio = if ragged_final_line { 0.0 } else { adjustment_ratio(delta, glue) };
                let badness = if ragged_final_line { 0.0 } else { line_badness(ratio) };
                let class = fitness_class(ratio);
                let opportunity = self.opportunities[to];
                let penalty = if opportunity.penalty > 0 && opportunity.penalty < 10_000 {
                    f64::from(opportunity.penalty)
                } else {
                    0.0
                };
                total += (LINE_PENALTY + badness + penalty).powi(2);
                if opportunity.penalty >= 10_000 {
                    total += f64::from(opportunity.penalty).powi(2);
                } else if opportunity.penalty < 0 && opportunity.penalty > -10_000 {
                    total -= f64::from(opportunity.penalty).powi(2);
                }
                if class.abs_diff(fitness) > 1 {
                    total += ADJACENT_FITNESS_DEMERITS;
                }
                if hyphen && opportunity.hyphen {
                    total += DOUBLE_HYPHEN_DEMERITS;
                }
                if ends_paragraph && to == last && hyphen {
                    total += opportunity.final_hyphen_demerits.unwrap_or(FINAL_HYPHEN_DEMERITS);
                }
                fitness = class;
                hyphen = opportunity.hyphen;
                classes.push(class as u8);
            }
            Some((total, classes))
        }

        /// The badness of every line of a sequence of breaks that the breaker allows.
        fn badness_of(&self, breaks: &[usize], ends_paragraph: bool) -> Vec<f64> {
            let last = self.opportunities.len() - 1;
            breaks
                .windows(2)
                .map(|window| {
                    let (natural, glue) = self.metrics(window[0], window[1]).unwrap();
                    let delta = self.width - natural;
                    if ends_paragraph && window[1] == last && delta >= 0.0 {
                        0.0
                    } else {
                        line_badness(adjustment_ratio(delta, glue))
                    }
                })
                .collect()
        }

        /// The least demerits of all the ways to break the paragraph.
        fn least_demerits(&self, start: LineStart) -> Option<f64> {
            self.least_demerits_of(start, true)
        }

        fn least_demerits_of(&self, start: LineStart, ends_paragraph: bool) -> Option<f64> {
            let last = self.opportunities.len() - 1;
            let inner = last - 1;
            (0u32..1 << inner)
                .filter_map(|mask| {
                    let mut breaks = vec![0];
                    breaks.extend((1..=inner).filter(|&bit| mask & (1 << (bit - 1)) != 0));
                    breaks.push(last);
                    self.demerits_of(&breaks, start, ends_paragraph).map(|(total, _)| total)
                })
                .min_by(f64::total_cmp)
        }
    }

    struct Random(u64);

    impl Random {
        fn below(&mut self, below: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % below
        }
    }

    fn random_paragraph(random: &mut Random) -> Paragraph {
        let words = 6 + random.below(6) as usize;
        let mut advance = vec![0.0];
        for _ in 0..words {
            advance.push(advance.last().unwrap() + 2.0 + random.below(8) as f64);
        }
        let opportunities = (0..=words)
            .map(|index| {
                let hyphen = index != 0 && index != words && random.below(4) == 0;
                BreakOpportunity { forced: false, hyphen, penalty: if hyphen { 50 } else { 0 }, ..Default::default() }
            })
            .collect();
        Paragraph { opportunities, advance, width: 14.0 + random.below(14) as f64 }
    }

    const STARTS: [LineStart; 8] = {
        let mut starts = [LineStart::PARAGRAPH; 8];
        let mut index = 0;
        while index < 8 {
            starts[index] = LineStart { fitness: (index / 2) as u8, hyphen: index % 2 == 1 };
            index += 1;
        }
        starts
    };

    #[test]
    fn the_breaks_of_every_start_have_the_least_demerits_of_all_breaks() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        let mut differing = 0;
        let mut checked = 0;
        for _ in 0..400 {
            let paragraph = random_paragraph(&mut random);
            let mut previous: Option<Vec<usize>> = None;
            for start in STARTS {
                let found = paragraph.lines(start);
                let Some(least) = paragraph.least_demerits(start) else {
                    continue;
                };
                let (demerits, classes) = paragraph
                    .demerits(&found.breaks, start)
                    .expect("the breaker finds breaks that it allows");
                assert!(
                    (demerits - least).abs() < 1e-6,
                    "{start:?}: the breaks {:?} have {demerits} demerits, the least is {least}",
                    found.breaks
                );
                // the fitness of every line is that of the line as the breaker counted it
                assert_eq!(found.fitness, classes, "{start:?} {:?}", found.breaks);
                if previous.as_ref().is_some_and(|breaks| *breaks != found.breaks) {
                    differing += 1;
                }
                previous = Some(found.breaks);
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked} paragraphs checked");
        // the line in front can change the breaks: the test is only worth something if it does
        assert!(differing > 20, "{differing} paragraphs break differently after another line");
    }

    #[test]
    fn the_part_of_a_paragraph_that_ends_at_a_cut_has_the_least_demerits_of_all_breaks() {
        // the last line of a part that ends at a cut is a line like the others: it is not ragged
        // and it is not counted as the last line of a paragraph
        let mut random = Random(0x0DDB_1A5E_5BAD_5EED);
        let (mut checked, mut differing) = (0, 0);
        for _ in 0..400 {
            let paragraph = random_paragraph(&mut random);
            for start in STARTS {
                let Some(least) = paragraph.least_demerits_of(start, false) else {
                    continue;
                };
                let part = paragraph.lines_of(start, false);
                let (demerits, classes) = paragraph
                    .demerits_of(&part.breaks, start, false)
                    .expect("the breaker finds breaks that it allows");
                assert!(
                    (demerits - least).abs() < 1e-6,
                    "{start:?}: the breaks {:?} have {demerits} demerits, the least is {least}",
                    part.breaks
                );
                assert_eq!(part.fitness, classes);
                // the part ends where the paragraph is cut
                assert_eq!(part.breaks.last(), Some(&(paragraph.opportunities.len() - 1)));
                if part.breaks != paragraph.lines_of(start, true).breaks {
                    differing += 1;
                }
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked} parts checked");
        assert!(differing > 20, "{differing} parts break differently from a whole paragraph");
    }

    #[test]
    fn the_variants_have_the_least_demerits_for_every_number_of_lines_around_the_optimum() {
        use std::collections::BTreeMap;
        let mut random = Random(0xFEED_FACE_CAFE_BEEF);
        let (mut checked, mut others) = (0, 0);
        for round in 0..200 {
            let mut paragraph = random_paragraph(&mut random);
            // some of the paragraphs have a forced break, which needs lines of its own
            if round % 3 == 0 {
                let middle = paragraph.opportunities.len() / 2;
                paragraph.opportunities[middle].forced = true;
            }
            let last = paragraph.opportunities.len() - 1;
            for start in STARTS {
                for ends_paragraph in [true, false] {
                    // the least demerits of all breakings, by their number of lines
                    let mut least: BTreeMap<usize, f64> = BTreeMap::new();
                    for mask in 0u32..1 << (last - 1) {
                        let mut breaks = vec![0];
                        breaks.extend((1..last).filter(|&bit| mask & (1 << (bit - 1)) != 0));
                        breaks.push(last);
                        if let Some((demerits, _)) = paragraph.demerits_of(&breaks, start, ends_paragraph) {
                            let entry = least.entry(breaks.len() - 1).or_insert(f64::INFINITY);
                            *entry = entry.min(demerits);
                        }
                    }
                    if least.is_empty() {
                        continue;
                    }
                    for extra in [0, 1, 2] {
                        let variants = compute_paragraph_variants(
                            &paragraph.opportunities,
                            |from, to| paragraph.metrics(from, to),
                            |_| paragraph.width,
                            start,
                            ends_paragraph,
                            extra,
                        );
                        let optimum = compute_paragraph_lines(
                            &paragraph.opportunities,
                            |from, to| paragraph.metrics(from, to),
                            |_| paragraph.width,
                            start,
                            ends_paragraph,
                        );
                        // the first is the optimum
                        assert_eq!(variants[0], optimum);
                        let around = variants[0].lines();
                        // there is one for every number of lines in the band that a breaking has
                        let expected: Vec<usize> = least
                            .keys()
                            .copied()
                            .filter(|&lines| lines != around && lines.abs_diff(around) <= extra as usize)
                            .collect();
                        let found: Vec<usize> = variants[1..].iter().map(ParagraphLines::lines).collect();
                        assert_eq!(found, expected, "{start:?} {ends_paragraph} extra {extra} around {around}");
                        for variant in &variants {
                            let lines = variant.lines();
                            let (total, classes) = paragraph
                                .demerits_of(&variant.breaks, start, ends_paragraph)
                                .expect("the breaker finds breaks that it allows");
                            assert!(
                                (total - least[&lines]).abs() < 1e-6,
                                "{lines} lines: {total} demerits, the least is {}",
                                least[&lines]
                            );
                            assert_eq!(variant.fitness, classes);
                            assert_eq!(variant.badness.len(), variant.lines());
                            assert_eq!(variant.badness, paragraph.badness_of(&variant.breaks, ends_paragraph));
                            assert!((variant.total_demerits() - total).abs() < 1e-6);
                            // the demerits of every line are what it adds to the lines in front
                            let mut before = 0.0;
                            for (line, demerits) in variant.demerits.iter().enumerate() {
                                let (so_far, _) = paragraph
                                    .demerits_of(&variant.breaks[..line + 2], start, ends_paragraph)
                                    .unwrap();
                                assert!((so_far - before - demerits).abs() < 1e-6, "line {line} of {lines}");
                                before = so_far;
                            }
                            checked += 1;
                        }
                        others += variants.len() - 1;
                    }
                }
            }
        }
        assert!(checked > 2000, "{checked} breakings checked");
        assert!(others > 500, "{others} breakings with another number of lines");
    }

    #[test]
    fn the_demerits_of_the_lines_of_a_breaking_are_those_of_the_lines() {
        let mut random = Random(0x1357_9BDF_2468_ACE0);
        for _ in 0..100 {
            let paragraph = random_paragraph(&mut random);
            for start in STARTS {
                let found = paragraph.lines(start);
                if found.demerits.iter().any(|d| d.is_infinite()) {
                    continue;
                }
                let (total, _) = paragraph.demerits(&found.breaks, start).unwrap();
                assert!((found.total_demerits() - total).abs() < 1e-6);
                assert_eq!(found.demerits.len(), found.lines());
            }
        }
    }

    #[test]
    fn a_hyphen_in_front_keeps_the_first_line_from_ending_in_one() {
        // two hyphens in a row cost the demerits of a double hyphen, so there are paragraphs whose
        // first line ends in a hyphen if it follows nothing, and does not if it follows a hyphen
        let mut random = Random(0x1234_5678_9ABC_DEF1);
        let hyphen_start = LineStart { fitness: LineStart::DECENT, hyphen: true };
        let (mut avoided, mut never_added) = (0, true);
        for _ in 0..400 {
            let paragraph = random_paragraph(&mut random);
            let ends_in_hyphen = |breaks: &[usize]| paragraph.opportunities[breaks[1]].hyphen;
            let plain = paragraph.lines(LineStart::PARAGRAPH).breaks;
            let after = paragraph.lines(hyphen_start).breaks;
            if ends_in_hyphen(&plain) && !ends_in_hyphen(&after) {
                avoided += 1;
            }
            // a hyphen in front never makes a hyphen at the end of the first line the choice if
            // it was not one without
            if !ends_in_hyphen(&plain) && ends_in_hyphen(&after) {
                never_added = false;
            }
        }
        assert!(avoided > 0, "no paragraph was found where a hyphen in front changes the first line");
        assert!(never_added);
    }

    fn pieces(widths: &[f64], forced: &[usize]) -> (Vec<BreakOpportunity>, Vec<f64>) {
        // widths of the words, with an opportunity in front of each, one at the end
        let mut advance = vec![0.0];
        for w in widths {
            advance.push(advance.last().unwrap() + w);
        }
        let opportunities = (0..=widths.len())
            .map(|i| BreakOpportunity { forced: forced.contains(&i), hyphen: false, penalty: 0, ..Default::default() })
            .collect();
        (opportunities, advance)
    }

    /// Rigid spaces with a finite stretch at the end of every line, as for ragged text.
    fn ragged(opportunities: &[BreakOpportunity], advance: &[f64], width: f64) -> Vec<usize> {
        compute_paragraph_breakpoints(
            opportunities,
            |from, to| {
                Some((advance[to] - advance[from], GlueTotals { stretch: 8.0, shrink: 0.0 }))
            },
            |_| width,
        )
    }

    #[test]
    fn ragged_lines_never_exceed_the_width() {
        let (opportunities, advance) = pieces(&[4.0; 9], &[]);
        let sequence = ragged(&opportunities, &advance, 10.0);
        for window in sequence.windows(2) {
            assert!(advance[window[1]] - advance[window[0]] <= 10.0, "{sequence:?}");
        }
    }

    #[test]
    fn ragged_overfull_word_gets_its_own_line() {
        let (opportunities, advance) = pieces(&[4.0, 4.0, 12.0, 4.0], &[]);
        let sequence = ragged(&opportunities, &advance, 10.0);
        assert_eq!(sequence, [0, 2, 3, 4]);
    }

    #[test]
    fn ragged_breaking_respects_mandatory_breaks() {
        let (opportunities, advance) = pieces(&[4.0, 4.0, 4.0, 4.0], &[1]);
        let sequence = ragged(&opportunities, &advance, 10.0);
        assert_eq!(sequence[..2], [0, 1]);
    }

    #[test]
    fn glue_stretch_and_shrink_set_the_adjustment_ratio() {
        let glue = GlueTotals {
            stretch: 4.0,
            shrink: 2.0,
        };

        assert_eq!(adjustment_ratio(2.0, glue), 0.5);
        assert_eq!(adjustment_ratio(-1.0, glue), -0.5);
        assert_eq!(line_badness(0.5), 12.5);
        assert_eq!(line_badness(10.0), 10_000.0);
    }

    #[test]
    fn breakpoint_penalty_prefers_an_equivalent_non_hyphen_break() {
        let opportunities = [
            BreakOpportunity {
                forced: false,
                hyphen: false,
                penalty: 0, ..Default::default() },
            BreakOpportunity {
                forced: false,
                hyphen: true,
                penalty: 50, ..Default::default() },
            BreakOpportunity {
                forced: false,
                hyphen: false,
                penalty: 0, ..Default::default() },
            BreakOpportunity {
                forced: false,
                hyphen: false,
                penalty: 0, ..Default::default() },
        ];
        let sequence = compute_paragraph_breakpoints(
            &opportunities,
            |from, to| match (from, to) {
                (0, 3) | (1, 2) => None,
                _ => Some((10.0, GlueTotals::default())),
            },
            |_| 10.0,
        );

        assert_eq!(sequence, vec![0, 2, 3]);
    }

    #[test]
    fn the_penalty_of_a_hyphen_is_added_to_the_badness_of_its_line() {
        // Without the hyphen the first line is a bit loose (badness 45); with the hyphen it is
        // perfect. As in TeX the hyphen penalty (50) is added to the badness before the sum is
        // squared, which makes the loose line the cheaper one. Added after squaring it would
        // be cheaper to hyphenate.
        let opportunities = [
            BreakOpportunity { forced: false, hyphen: false, penalty: 0, ..Default::default() },
            BreakOpportunity { forced: false, hyphen: true, penalty: 50, ..Default::default() },
            BreakOpportunity { forced: false, hyphen: false, penalty: 0, ..Default::default() },
            BreakOpportunity { forced: false, hyphen: false, penalty: 0, ..Default::default() },
        ];
        let sequence = compute_paragraph_breakpoints(
            &opportunities,
            |from, to| match (from, to) {
                (0, 1) | (1, 3) => Some((10.0, GlueTotals::default())),
                (0, 2) => Some((2.34, GlueTotals { stretch: 10.0, shrink: 0.0 })),
                (2, 3) => Some((5.0, GlueTotals::default())),
                _ => None,
            },
            |_| 10.0,
        );

        assert_eq!(sequence, vec![0, 2, 3]);
    }
}
