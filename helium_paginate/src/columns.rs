//! The columns of one fragment of a container that is split over pages.
//!
//! A multi-column container is a fragmentation context inside the one of the page. The fragment
//! of it on a page has its columns side by side, as many as the container has, each as high as the
//! page lets it be. This is the inner problem of the container: the columns are the pages of a
//! [`Problem`] over the content of the container, and the kernel and the exact solver choose them
//! as they choose pages. [`plan_fragment`] does it for the fragment that starts at a place of the
//! content and has some height available (see `docs/pagination.md`, "Multi-column layout").
//!
//! - The columns of a fragment that does not end the container are as full as they can be. They are
//!   chosen by the exact solver with [`CssObjective`], so that `avoid`, orphans and widows are
//!   kept as far as possible. The fragment ends where its last column does, or at a forced break
//!   of the page level.
//! - The last fragment is balanced with `column-fill: balance` and `balance-all` (only it is
//!   balanced with `balance`), and the columns are filled in turn with `auto`. A fragment that ends
//!   at a forced break of the page level is balanced with `balance-all` only.

use crate::flow::{BreakKind, BreakLevel, Item, PageSpec, PageTable, Problem};
use crate::kernel::Kernel;
use crate::service::LayoutService;
use crate::solver::{CssObjective, ExactSolver, Solver, balance};
use crate::token::{ContainerCut, Position};

/// The value of `column-fill`.
/// https://www.w3.org/TR/css-multicol-1/#cf
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum ColumnFill {
    /// Only the last fragment is balanced.
    #[default]
    Balance,
    /// Every fragment that is not full is balanced: the last one, and one that ends at a forced
    /// break.
    BalanceAll,
    /// The columns are filled in turn.
    Auto,
}

/// The columns of a fragment.
#[derive(Debug, Clone, PartialEq)]
pub struct FragmentPlan {
    /// The start and the end of every column, in the flow of the content. There are fewer columns
    /// than the container has if the content ends sooner. The end of a column is the place where
    /// the next one starts, or the item behind the break that ends the fragment.
    pub columns: Vec<(Position, Position)>,
    /// Where the next fragment starts, or `None` if the content ends in this fragment.
    pub end: Option<ContainerCut>,
    /// The kind of the break that ends the fragment.
    pub end_kind: BreakKind,
    /// The height of the tallest column.
    pub height: f32,
    /// Some column is taller than the height that is available: it has an item that cannot be
    /// split.
    pub overflow: bool,
    /// A column of the fragment ends where it should not, or has too few lines of a paragraph.
    pub avoid_violation: bool,
    pub orphan_violation: bool,
    pub widow_violation: bool,
}

/// The columns that the content of a container has in the fragment that starts at `from`
/// (`None`: at the start of the container).
///
/// `items` is the flow of the content of the container, as `inline` wide as a column; the service
/// answers for it. The fragment has `columns` columns of at most `available` height. `bottom` is
/// what is added to the fragment if it ends the container (the padding and the border at the
/// bottom): the content of the last fragment is balanced in `available - bottom`.
#[allow(clippy::too_many_arguments)]
pub fn plan_fragment(
    service: &mut dyn LayoutService,
    items: &[Item],
    columns: usize,
    inline: f32,
    from: Option<ContainerCut>,
    available: f32,
    bottom: f32,
    fill: ColumnFill,
) -> FragmentPlan {
    let start = from.map_or(Position::Item(0), ContainerCut::position);
    if matches!(start, Position::Item(item) if item >= items.len()) {
        return FragmentPlan {
            columns: vec![],
            end: None,
            end_kind: BreakKind::Allowed,
            height: 0.0,
            overflow: false,
            avoid_violation: false,
            orphan_violation: false,
            widow_violation: false,
        };
    }
    let columns = columns.max(1);
    let table = |block_size: f32| {
        PageTable::uniform(PageSpec {
            inline_size: inline,
            block_size: block_size.max(0.0),
        })
    };
    let chosen = |service: &mut dyn LayoutService, block_size: f32| {
        let problem = Problem::new(table(block_size), items.to_vec())
            .with_level(BreakLevel::Column)
            .with_page_limit(columns)
            .starting_at(start)
            .as_fragment();
        let mut kernel = Kernel::new(&problem, service);
        let solution = ExactSolver::new(CssObjective::default()).solve(&mut kernel);
        let fills = kernel.realize(&solution).ok()?;
        let places: Vec<(Position, Position)> = solution
            .pages
            .iter()
            .zip(&fills)
            .map(|(page, fill)| (kernel.position(page.at), kernel.position(fill.end)))
            .collect();
        let ended = fills.last().is_some_and(|fill| kernel.is_end(fill.end));
        Some((places, fills, ended))
    };

    // The columns of a fragment that is as full as it can be. A fragment that ends the container
    // has the padding and the border at the bottom, too, so it has less room.
    let room = (available - bottom).max(0.0);
    let Some(mut found) = chosen(service, room) else {
        return whole(items, start, available);
    };
    if !found.2 && bottom > 0.0 {
        // there is no room for the bottom edge, but there is for the content: the container does
        // not end here, either (the content of the last fragment is in `room`)
        if let Some(full) = chosen(service, available) {
            if !full.2 {
                found = full;
            }
        }
    }
    let (mut places, mut fills, ended) = found;
    // (how the last column ends is how the fragment does, whatever the columns are made into)
    let end_kind = if ended {
        BreakKind::Allowed
    } else {
        fills.last().map_or(BreakKind::Allowed, |fill| fill.end_kind)
    };

    // Where the fragment ends in the content, and whether it ends at a forced break of the page
    // level (which is behind the item that is the end of the last column).
    let last_end = places.last().map(|&(_, end)| end);
    let forced_page_break = match last_end {
        Some(Position::Item(item)) if item > 0 && !ended => {
            matches!(items[item - 1], Item::Break { kind, level: BreakLevel::Page } if kind.is_forced())
        }
        _ => false,
    };
    let balanced = match fill {
        ColumnFill::Auto => false,
        ColumnFill::Balance => ended,
        ColumnFill::BalanceAll => ended || forced_page_break,
    };
    if balanced {
        // the content up to the end of the container or up to the break
        let content = match last_end {
            Some(Position::Item(item)) if forced_page_break => item - 1,
            _ => items.len(),
        };
        let size = if ended { room } else { available };
        let problem = Problem::new(table(size), items[..content].to_vec())
            .with_level(BreakLevel::Column)
            .with_page_limit(columns)
            .starting_at(start)
            .with_page_breaks();
        let mut kernel = Kernel::new(&problem, service);
        let solution = balance(&mut kernel);
        if let Ok(balanced_fills) = kernel.realize(&solution) {
            let last = places.last().map(|&(_, end)| end);
            places = solution
                .pages
                .iter()
                .zip(&balanced_fills)
                .map(|(page, fill)| (kernel.position(page.at), kernel.position(fill.end)))
                .collect();
            // the last column ends where the fragment does
            if let (Some(entry), Some(last)) = (places.last_mut(), last) {
                entry.1 = last;
            }
            fills = balanced_fills;
        }
    }

    let end = match last_end {
        Some(_) if ended => None,
        Some(position) => Some(ContainerCut::of(position)),
        None => None,
    };
    let (mut orphan_violation, mut widow_violation, mut avoid_violation, mut overflow) = (false, false, false, false);
    let count = fills.len();
    for (index, fill) in fills.iter().enumerate() {
        orphan_violation |= fill.orphan_violation;
        widow_violation |= fill.widow_violation;
        // (how the last column ends is how the fragment does)
        avoid_violation |= fill.end_kind == BreakKind::Avoid && index + 1 < count;
        overflow |= fill.overflow;
    }
    FragmentPlan {
        columns: places,
        end,
        end_kind,
        height: fills.iter().map(|fill| fill.used).fold(0.0, f32::max),
        overflow,
        avoid_violation,
        orphan_violation,
        widow_violation,
    }
}

/// All the content in one column: what is left if the columns cannot be chosen.
fn whole(items: &[Item], start: Position, available: f32) -> FragmentPlan {
    FragmentPlan {
        columns: vec![(start, Position::Item(items.len()))],
        end: None,
        end_kind: BreakKind::Allowed,
        height: available.max(0.0),
        overflow: false,
        avoid_violation: false,
        orphan_violation: false,
        widow_violation: false,
    }
}
