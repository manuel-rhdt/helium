//! A port of the subset of web-platform-tests' `check-layout-th.js` that helium can evaluate.
//!
//! WPT layout tests annotate elements with `data-expected-*` attributes. After layout, every
//! annotated element is compared against the box helium computed for it.

use specs::{World, WorldExt};

use crate::layout::components::{Decoration, LayoutEntity, PageGeometry, Position};
use crate::style::properties::computed::PositionSpec;
use crate::layout::construct::AssignLayoutEntity;
use crate::parser::ParsedData;
use crate::tree::{DomRef, TreeLinked};

/// Tolerance used by `assert_tolerance` in check-layout-th.js.
const TOLERANCE: f32 = 1.0;

/// Attributes of check-layout-th.js that we know about but cannot evaluate yet. A test that uses
/// one of them is reported as unsupported instead of silently passing.
const UNSUPPORTED: &[&str] = &[
    "data-total-x",
    "data-total-y",
    "data-expected-client-width",
    "data-expected-client-height",
    "data-expected-scroll-width",
    "data-expected-scroll-height",
    "data-expected-display",
];

#[derive(Debug, Default)]
pub struct CheckReport {
    /// Number of individual expectations that were evaluated.
    pub checked: usize,
    pub failures: Vec<String>,
    pub unsupported: Vec<String>,
}

pub fn check_layout(dom: &DomRef, world: &World) -> CheckReport {
    let mut report = CheckReport::default();
    visit(dom, world, &mut report, &mut Vec::new());
    report
}

/// The border box of the node's layout box as `(x, y, width, height)`.
fn border_box(entity: specs::Entity, world: &World) -> Option<(f32, f32, f32, f32)> {
    let pos = world.read_storage::<Position>().get(entity).copied()?;
    let deco = world.read_storage::<Decoration>().get(entity).copied().unwrap_or_default();
    Some((
        pos.x.0 - deco.padding.left.0 - deco.border.left.0,
        pos.y.0 - deco.padding.top.0 - deco.border.top.0,
        pos.width.0 + deco.padding.left.0 + deco.padding.right.0 + deco.border.left.0 + deco.border.right.0,
        pos.height.0 + deco.padding.top.0 + deco.padding.bottom.0 + deco.border.top.0 + deco.border.bottom.0,
    ))
}

/// The distance of the border box of `node` from its `offsetParent` like `offsetLeft` and
/// `offsetTop` report it.
///
/// The offset parent is the nearest ancestor that is positioned. Without one the offset is
/// relative to the origin of the initial containing block, the content area of the first page.
fn offset(node: &DomRef, world: &World) -> Option<(f32, f32)> {
    let entity = node.layout_entity().map(|LayoutEntity(e)| e)?;
    let (x, y, _, _) = border_box(entity, world)?;

    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        let positioned = crate::tree::NodeData::style(&parent)
            .map_or(false, |style| style.box_.position != PositionSpec::Static);
        if positioned {
            if let Some(parent_entity) = parent.layout_entity().map(|LayoutEntity(e)| e) {
                let pos = world.read_storage::<Position>().get(parent_entity).copied()?;
                let deco = world.read_storage::<Decoration>().get(parent_entity).copied().unwrap_or_default();
                // the offset is relative to the padding edge of the offset parent
                let padding_edge_x = pos.x.0 - deco.padding.left.0;
                let padding_edge_y = pos.y.0 - deco.padding.top.0;
                return Some((x - padding_edge_x, y - padding_edge_y));
            }
        }
        ancestor = parent.parent();
    }

    let geometry = world.try_fetch::<PageGeometry>().map(|g| *g).unwrap_or_default();
    Some((x - geometry.margin.left.0, y - geometry.margin.top.0))
}

fn visit(node: &DomRef, world: &World, report: &mut CheckReport, path: &mut Vec<String>) {
    if let ParsedData::Element(element) = &node.html_data {
        path.push(element.name.local.to_string());
        check_element(node, element, world, report, &path.join(" > "));
        for child in TreeLinked::children(node) {
            visit(&child, world, report, path);
        }
        path.pop();
    } else {
        for child in TreeLinked::children(node) {
            visit(&child, world, report, path);
        }
    }
}

fn check_element(
    node: &DomRef,
    element: &crate::parser::ElementData,
    world: &World,
    report: &mut CheckReport,
    path: &str,
) {
    let attrs: Vec<(String, String)> = element
        .attributes
        .borrow()
        .iter()
        .map(|(name, attr)| (name.local.to_string(), attr.value.to_string()))
        .filter(|(name, _)| name.starts_with("data-expected-") || name.starts_with("data-offset-") || name.starts_with("data-total-"))
        .collect();
    if attrs.is_empty() {
        return;
    }

    let entity = node.layout_entity().map(|LayoutEntity(e)| e);
    let boxes = entity.and_then(|e| {
        let pos = world.read_storage::<Position>().get(e).copied()?;
        let deco = world
            .read_storage::<Decoration>()
            .get(e)
            .copied()
            .unwrap_or_default();
        Some((pos, deco))
    });

    for (name, value) in attrs {
        if UNSUPPORTED.contains(&name.as_str()) {
            report.unsupported.push(format!("{}: {}", path, name));
            continue;
        }
        let expected: f32 = match value.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                report.failures.push(format!("{}: {}=\"{}\" is not a number", path, name, value));
                continue;
            }
        };
        let (pos, deco) = match &boxes {
            Some(b) => *b,
            None => {
                report.failures.push(format!("{}: {} but element has no layout box", path, name));
                continue;
            }
        };
        let actual = match name.as_str() {
            "data-offset-x" | "data-offset-y" => match offset(node, world) {
                Some((x, y)) => if name == "data-offset-x" { x } else { y },
                None => {
                    report.failures.push(format!("{}: {} but element has no layout box", path, name));
                    continue;
                }
            },
            // offsetWidth/offsetHeight are border-box sizes
            "data-expected-width" | "data-expected-bounding-client-rect-width" => {
                pos.width.0 + deco.padding.left.0 + deco.padding.right.0 + deco.border.left.0 + deco.border.right.0
            }
            "data-expected-height" | "data-expected-bounding-client-rect-height" => {
                pos.height.0 + deco.padding.top.0 + deco.padding.bottom.0 + deco.border.top.0 + deco.border.bottom.0
            }
            "data-expected-padding-top" => deco.padding.top.0,
            "data-expected-padding-bottom" => deco.padding.bottom.0,
            "data-expected-padding-left" => deco.padding.left.0,
            "data-expected-padding-right" => deco.padding.right.0,
            "data-expected-margin-top" => deco.margin.top.0,
            "data-expected-margin-bottom" => deco.margin.bottom.0,
            "data-expected-margin-left" => deco.margin.left.0,
            "data-expected-margin-right" => deco.margin.right.0,
            other => {
                report.unsupported.push(format!("{}: {}", path, other));
                continue;
            }
        };
        report.checked += 1;
        if (actual - expected).abs() >= TOLERANCE {
            report
                .failures
                .push(format!("{}: {} expected {} but got {}", path, name, expected, actual));
        }
    }
}
