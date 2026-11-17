//! Pagination: the choice of page breaks.
//!
//! The crate does not know the ECS, fonts or Taffy. The content that is split across pages is
//! given as a flat flow of [`Item`]s. The [`Kernel`] knows the rules of fragmentation (which breaks
//! are legal, what is discarded at the edge of a page, how orphans and widows are counted) and
//! enumerates the pages that can start at a [`BreakToken`]. A [`Solver`] chooses among these
//! pages with the help of an [`Objective`]. A [`LayoutService`] answers the kernel's questions
//! about sizes. See `docs/pagination.md` for the design.
//!
//! [`paragraph`] holds the line breaker.

mod columns;
mod flow;
mod graph;
mod kernel;
pub mod paragraph;
mod service;
mod solver;
mod token;

pub use flow::{BoxRef, BreakKind, BreakLevel, FloatSide, Item, ItemId, NodeRef, PageKind, PageSpec, PageTable, ParId, Problem, Variants};
pub use columns::{plan_fragment, ColumnFill, FragmentPlan};
pub use kernel::{Choice, ChoiceId, FloatList, Invalid, Kernel, PageFill, PageFragment, PageStart, Solution};
pub use service::{ContainerFloat, Fragment, GalleyFlow, GalleyService, LayoutService};
pub use graph::{shortest_path, Graph, Path, PageEdge, PageGraph, PageKey};
pub use solver::{balance, ColumnCost, CssCost, CssObjective, ExactSolver, FullestFirst, Objective, Solver, TallestColumn};
pub use service::ParLine;
pub use token::{BreakToken, ContainerCut, Cut, Position};

#[cfg(test)]
mod tests;
