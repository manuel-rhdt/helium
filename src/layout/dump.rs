//! Serializes the computed layout as JSON so it can be inspected or compared against the layout
//! of other engines.
//!
//! The output is an object with two members. `document` is a tree that mirrors the DOM. Every node that has a layout box carries its
//! `border` box (like `getBoundingClientRect()`), its `content` box and the `padding`, `border`
//! and `margin` widths. All lengths are in CSS pixels, with the origin at the top left corner of
//! the page the box was placed on.
//!
//! `paint` lists what ends up on the page independent of the document structure, which is what a
//! reftest compares: the filled `rects` (backgrounds and borders) and the `text` paragraphs with
//! their glyph runs.

use std::fmt::Write;

use specs::{Join, World, WorldExt};

use crate::layout::components::{
    Decoration, EdgeValues, KernedGlyphs, LayoutEntity, Paragraph, Position, TextOperator, TextRun,
};
use crate::layout::construct::AssignLayoutEntity;
use crate::parser::ParsedData;
use crate::style::properties::computed::{PixelLength, RGBA};
use crate::tree::{DomRef, TreeLinked};

pub fn dump_layout(dom: &DomRef, world: &World) -> String {
    let mut out = String::from("{\n\"document\": ");
    dump_node(dom, world, 0, &mut out);
    out.push_str(",\n\"paint\": {\n\"rects\": [");
    let mut rects = Vec::new();
    collect_rects(dom, world, &mut rects);
    out.push_str(&rects.join(",\n"));
    out.push_str("],\n\"text\": [");
    out.push_str(&dump_paragraphs(world).join(",\n"));
    out.push_str("]\n}\n}\n");
    out
}

fn dump_node(node: &DomRef, world: &World, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let entity = node.layout_entity().map(|LayoutEntity(e)| e);

    out.push_str(&indent);
    out.push('{');
    match &node.html_data {
        ParsedData::Element(element) => {
            let attributes = element.attributes.borrow();
            let attr = |name: &str| {
                attributes
                    .iter()
                    .find(|(qname, _)| &*qname.local == name)
                    .map(|(_, attr)| attr.value.to_string())
            };
            write!(out, "\"type\": \"element\", \"tag\": {}", json_string(&element.name.local)).unwrap();
            if let Some(id) = attr("id") {
                write!(out, ", \"id\": {}", json_string(&id)).unwrap();
            }
            if let Some(class) = attr("class") {
                write!(out, ", \"class\": {}", json_string(&class)).unwrap();
            }
        }
        ParsedData::Text(_) => out.push_str("\"type\": \"text\""),
        ParsedData::Document(_) => out.push_str("\"type\": \"document\""),
        _ => {
            out.push_str("\"type\": \"other\"}");
            return;
        }
    }

    if let Some(entity) = entity {
        if let Some(text_run) = world.read_storage::<TextRun>().get(entity) {
            write!(out, ", \"text\": {}", json_string(&text_run.text)).unwrap();
        }
        if let Some(pos) = world.read_storage::<Position>().get(entity).copied() {
            let deco = world
                .read_storage::<Decoration>()
                .get(entity)
                .copied()
                .unwrap_or_default();
            write_boxes(out, pos, deco);
        }
    }

    let children: Vec<DomRef> = TreeLinked::children(node).collect();
    if children.is_empty() {
        out.push('}');
    } else {
        out.push_str(", \"children\": [\n");
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                out.push_str(",\n");
            }
            dump_node(child, world, depth + 1, out);
        }
        write!(out, "\n{}]}}", indent).unwrap();
    }
}

fn write_boxes(out: &mut String, pos: Position, deco: Decoration) {
    let (pad, bor) = (deco.padding, deco.border);
    let border_box = Rect {
        x: pos.x.0 - pad.left.0 - bor.left.0,
        y: pos.y.0 - pad.top.0 - bor.top.0,
        width: pos.width.0 + pad.left.0 + pad.right.0 + bor.left.0 + bor.right.0,
        height: pos.height.0 + pad.top.0 + pad.bottom.0 + bor.top.0 + bor.bottom.0,
    };
    let content = Rect { x: pos.x.0, y: pos.y.0, width: pos.width.0, height: pos.height.0 };
    write!(out, ", \"page\": {}", pos.page).unwrap();
    write!(out, ", \"border_box\": {}", border_box).unwrap();
    write!(out, ", \"content_box\": {}", content).unwrap();
    write!(out, ", \"padding\": {}", Edges(pad)).unwrap();
    write!(out, ", \"border\": {}", Edges(bor)).unwrap();
    write!(out, ", \"margin\": {}", Edges(deco.margin)).unwrap();
    if deco.background_color.alpha > 0 {
        write!(out, ", \"background\": {}", color(deco.background_color)).unwrap();
    }
}

fn color(c: RGBA) -> String {
    format!("[{}, {}, {}, {}]", c.red, c.green, c.blue, c.alpha)
}

/// Collects the rectangles that are filled with a solid color for the element tree below `node`.
fn collect_rects(node: &DomRef, world: &World, rects: &mut Vec<String>) {
    let entity = node.layout_entity().map(|LayoutEntity(e)| e);
    if let (ParsedData::Element(_), Some(entity)) = (&node.html_data, entity) {
        if let Some(pos) = world.read_storage::<Position>().get(entity).copied() {
            let deco = world.read_storage::<Decoration>().get(entity).copied().unwrap_or_default();
            let (pad, bor) = (deco.padding, deco.border);
            let (x, y) = (pos.x.0 - pad.left.0 - bor.left.0, pos.y.0 - pad.top.0 - bor.top.0);
            let width = pos.width.0 + pad.left.0 + pad.right.0 + bor.left.0 + bor.right.0;
            let height = pos.height.0 + pad.top.0 + pad.bottom.0 + bor.top.0 + bor.bottom.0;
            let mut push = |c: RGBA, x: f32, y: f32, width: f32, height: f32| {
                if c.alpha > 0 && width > 0.0 && height > 0.0 {
                    rects.push(format!(
                        "{{\"page\": {}, \"x\": {}, \"y\": {}, \"width\": {}, \"height\": {}, \"color\": {}}}",
                        pos.page, number(x), number(y), number(width), number(height), color(c)
                    ));
                }
            };
            // the background is painted below the borders
            push(deco.background_color, x, y, width, height);
            let bc = deco.border_color;
            push(bc.top, x, y, width, bor.top.0);
            push(bc.bottom, x, y + height - bor.bottom.0, width, bor.bottom.0);
            push(bc.left, x, y, bor.left.0, height);
            push(bc.right, x + width - bor.right.0, y, bor.right.0, height);
        }
    }
    for child in TreeLinked::children(node) {
        collect_rects(&child, world, rects);
    }
}

fn dump_paragraphs(world: &World) -> Vec<String> {
    let positions = world.read_storage::<Position>();
    let paragraphs = world.read_storage::<Paragraph>();
    let mut out = Vec::new();
    for (pos, paragraph) in (&positions, &paragraphs).join() {
        let ops: Vec<String> = paragraph.ops.iter().map(op_json).collect();
        out.push(format!(
            "{{\"page\": {}, \"x\": {}, \"y\": {}, \"ops\": [{}]}}",
            pos.page,
            number(paragraph.start_pos.0 .0),
            number(paragraph.start_pos.1 .0),
            ops.join(", ")
        ));
    }
    out
}

fn glyph_ids(glyphs: &crate::layout::components::Glyphs) -> String {
    let ids: Vec<String> = glyphs.0.iter().map(|g| g.0.to_string()).collect();
    format!("[{}]", ids.join(", "))
}

fn op_json(op: &TextOperator) -> String {
    match op {
        TextOperator::SetFont(name, size) => {
            format!("{{\"op\": \"font\", \"name\": {}, \"size\": {}}}", json_string(name), number(size.0))
        }
        TextOperator::Glyphs(glyphs) => format!("{{\"op\": \"glyphs\", \"glyphs\": {}}}", glyph_ids(glyphs)),
        TextOperator::KernedGlyphs(items) => {
            let items: Vec<String> = items
                .iter()
                .map(|item| match item {
                    KernedGlyphs::Glyphs(glyphs) => glyph_ids(glyphs),
                    KernedGlyphs::Kern(kern) => number(kern.0 as f32),
                })
                .collect();
            format!("{{\"op\": \"kerned\", \"items\": [{}]}}", items.join(", "))
        }
        TextOperator::SetWordSpacing(spacing) => {
            format!("{{\"op\": \"word_spacing\", \"value\": {}}}", number(spacing.0))
        }
        TextOperator::SetTransform(m) => format!(
            "{{\"op\": \"transform\", \"matrix\": [{}, {}, {}, {}, {}, {}]}}",
            number(m.a), number(m.b), number(m.c), number(m.d), number(m.e.0), number(m.f.0)
        ),
        TextOperator::NewLine { x_offset, leading } => format!(
            "{{\"op\": \"newline\", \"x_offset\": {}, \"leading\": {}}}",
            number(x_offset.0), number(leading.0)
        ),
    }
}

struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{{\"x\": {}, \"y\": {}, \"width\": {}, \"height\": {}}}",
            number(self.x),
            number(self.y),
            number(self.width),
            number(self.height)
        )
    }
}

struct Edges(EdgeValues<PixelLength>);

impl std::fmt::Display for Edges {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{{\"top\": {}, \"right\": {}, \"bottom\": {}, \"left\": {}}}",
            number(self.0.top.0),
            number(self.0.right.0),
            number(self.0.bottom.0),
            number(self.0.left.0)
        )
    }
}

/// Formats a length rounded to 1/1000 px; JSON has no NaN or infinity so those become `null`.
fn number(value: f32) -> String {
    if !value.is_finite() {
        return "null".into();
    }
    let rounded = (value as f64 * 1000.0).round() / 1000.0;
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json_strings() {
        assert_eq!(json_string("a\"b\\c\n\u{1}"), "\"a\\\"b\\\\c\\n\\u0001\"");
    }

    #[test]
    fn numbers_are_rounded_and_finite() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(0.30000001), "0.3");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(f32::NAN), "null");
    }
}
