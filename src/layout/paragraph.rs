//! Contains an algorithm for Knuth-style optimal paragraph formatting

#[derive(Default, Copy, Clone)]
struct Node {
    position: usize,
    weight: f64,
    previous: Option<usize>,
}

#[derive(Debug, Copy, Clone)]
pub struct BreakOpportunity {
    pub forced: bool,
    pub hyphen: bool,
    pub penalty: i32,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct GlueTotals {
    pub stretch: f64,
    pub shrink: f64,
}

#[derive(Debug, Copy, Clone)]
struct ParagraphNode {
    demerits: f64,
    previous: Option<(usize, usize)>,
}

const FITNESS_CLASSES: usize = 4;
const HYPHEN_STATES: usize = 2;
const STATE_COUNT: usize = FITNESS_CLASSES * HYPHEN_STATES;
const LINE_PENALTY: f64 = 10.0;
const ADJACENT_FITNESS_DEMERITS: f64 = 3_000.0;
const DOUBLE_HYPHEN_DEMERITS: f64 = 10_000.0;
const FINAL_HYPHEN_DEMERITS: f64 = 5_000.0;

/// Fills every line as far as it fits, like browsers do for text that is not justified: a line
/// ends at the last break opportunity where the content still fits (or can shrink to fit),
/// at a mandatory break, or after the first piece if even that does not fit. Optimal breaking
/// is wrong for ragged text, where there is no glue to stretch and it prefers short first lines.
///
/// The arguments are those of [`compute_paragraph_breakpoints`].
pub fn compute_greedy_breakpoints(
    opportunities: &[BreakOpportunity],
    mut line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize) -> f64,
) -> Vec<usize> {
    if opportunities.len() < 2 {
        return opportunities.first().map_or_else(Vec::new, |_| vec![0]);
    }
    let last = opportunities.len() - 1;
    let mut sequence = vec![0];
    let mut from = 0;
    while from < last {
        let width = line_width(from);
        let mut best = None;
        for to in from + 1..=last {
            let fits = line_metrics(from, to).is_some_and(|(natural, glue)| {
                let delta = width - natural;
                delta >= 0.0 || (glue.shrink > 0.0 && -delta <= glue.shrink)
            });
            if fits {
                best = Some(to);
            } else if best.is_some() {
                break;
            }
            // a mandatory break ends the line, whether or not the content fits
            if opportunities[to].forced {
                best = best.or(Some(to));
                break;
            }
        }
        let to = best.unwrap_or(from + 1);
        sequence.push(to);
        from = to;
    }
    sequence
}

/// Choose paragraph breaks using adjustment-ratio badness and TeX-style demerits.
///
/// `line_metrics` returns the natural width and the total stretch and shrink of glue strictly
/// inside the line. Glue at a break is discarded by the caller and must not be included.
pub fn compute_paragraph_breakpoints(
    opportunities: &[BreakOpportunity],
    mut line_metrics: impl FnMut(usize, usize) -> Option<(f64, GlueTotals)>,
    mut line_width: impl FnMut(usize) -> f64,
) -> Vec<usize> {
    if opportunities.len() < 2 {
        return opportunities.first().map_or_else(Vec::new, |_| vec![0]);
    }

    let mut paths = vec![[None; STATE_COUNT]; opportunities.len()];
    let mut forced_prefix = vec![0usize; opportunities.len() + 1];
    for (index, opportunity) in opportunities.iter().enumerate() {
        forced_prefix[index + 1] = forced_prefix[index] + usize::from(opportunity.forced);
    }
    let line_widths: Vec<_> = (0..opportunities.len() - 1).map(&mut line_width).collect();
    let initial_state = state_index(1, false);
    paths[0][initial_state] = Some(ParagraphNode {
        demerits: 0.0,
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
            let delta = line_widths[from] - natural_width;
            if delta < 0.0 && (glue.shrink <= 0.0 || -delta > glue.shrink) {
                continue;
            }
            let ragged_final_line = to + 1 == opportunities.len() && delta >= 0.0;
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

            for previous_state in 0..STATE_COUNT {
                let Some(previous) = paths[from][previous_state] else {
                    continue;
                };
                let (previous_fitness, previous_hyphen) = decode_state(previous_state);
                let mut demerits = previous.demerits + (LINE_PENALTY + badness).powi(2);

                if opportunity.penalty > 0 {
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
                if to + 1 == opportunities.len() && previous_hyphen {
                    demerits += FINAL_HYPHEN_DEMERITS;
                }

                let next_state = state_index(fitness, opportunity.hyphen);
                let replace =
                    paths[to][next_state].map_or(true, |current| demerits < current.demerits);
                if replace {
                    paths[to][next_state] = Some(ParagraphNode {
                        demerits,
                        previous: Some((from, previous_state)),
                    });
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
        return vec![0, last];
    };

    let mut index = last;
    let mut sequence = vec![last];
    while let Some((previous_index, previous_state)) =
        paths[index][state].and_then(|node| node.previous)
    {
        index = previous_index;
        state = previous_state;
        sequence.push(index);
    }
    sequence.reverse();
    sequence
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

    fn pieces(widths: &[f64], forced: &[usize]) -> (Vec<BreakOpportunity>, Vec<f64>) {
        // widths of the words, with an opportunity in front of each, one at the end
        let mut advance = vec![0.0];
        for w in widths {
            advance.push(advance.last().unwrap() + w);
        }
        let opportunities = (0..=widths.len())
            .map(|i| BreakOpportunity { forced: forced.contains(&i), hyphen: false, penalty: 0 })
            .collect();
        (opportunities, advance)
    }

    #[test]
    fn greedy_breaking_fills_the_first_lines() {
        // words of width 4 (no spaces) in lines of width 10: two words fit per line
        let (opportunities, advance) = pieces(&[4.0; 5], &[]);
        let sequence = compute_greedy_breakpoints(
            &opportunities,
            |from, to| Some((advance[to] - advance[from], GlueTotals::default())),
            |_| 10.0,
        );
        assert_eq!(sequence, [0, 2, 4, 5]);
    }

    #[test]
    fn greedy_breaking_respects_mandatory_breaks_and_overfull_words() {
        let (opportunities, advance) = pieces(&[4.0, 4.0, 4.0, 12.0, 4.0], &[2]);
        let sequence = compute_greedy_breakpoints(
            &opportunities,
            |from, to| Some((advance[to] - advance[from], GlueTotals::default())),
            |_| 10.0,
        );
        // the forced break after the second word, then one word, the overfull one, the rest
        assert_eq!(sequence, [0, 2, 3, 4, 5]);
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
                penalty: 0,
            },
            BreakOpportunity {
                forced: false,
                hyphen: true,
                penalty: 50,
            },
            BreakOpportunity {
                forced: false,
                hyphen: false,
                penalty: 0,
            },
            BreakOpportunity {
                forced: false,
                hyphen: false,
                penalty: 0,
            },
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
}
