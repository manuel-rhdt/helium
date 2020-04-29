//! Contains an algorithm for Knuth-style optimal paragraph formatting

use std::cmp::Ordering;

/// Returns the weight of the subsequence with the lowest penalty
fn lowest_weight_subsequence(
    to: usize,
    weights: &[f64],
    weight: &mut dyn FnMut(usize, usize) -> f64,
) -> Option<(usize, f64)> {
    (0..to)
        .filter(|&from| weights[from] < std::f64::INFINITY)
        .map(|from| (from, weight(from, to) + weights[from]))
        .min_by(|(_, p1), (_, p2)| p1.partial_cmp(p2).unwrap_or(Ordering::Equal))
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
    // a vec of weights for every break point
    let mut weights = vec![0.0f64; num_breakpoints];
    // a list that stores the optimal "parent" break point's index for every break point
    let mut parents = vec![0; num_breakpoints];
    for i in 1..num_breakpoints {
        let (j, weight) =
            lowest_weight_subsequence(i, &weights, weight).unwrap_or((0, std::f64::INFINITY));
        weights[i] = weight;
        debug_assert!(j < i);
        parents[i] = j;
    }

    // find all the ancestors of the last breakpoint
    let mut i = parents.last().cloned().unwrap_or(0);
    let mut result = vec![];
    while i != 0 {
        result.push(i);
        i = parents[i];
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

        dbg!(&offsets);
        let sequence = compute_breakpoints(offsets.len(), &mut |from, to| {
            line_demerits(from, to, &offsets, 15.0)
        });

        println!("{:?}", sequence);
    }
}
