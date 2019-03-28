mod assign_heights;
pub mod block;
mod block_formatting_context;
pub mod block_size;
pub mod construct;
pub mod entity;
mod global_transform;
pub mod inline;
pub mod inline_size;
pub mod text;

use crate::layout::entity::LayoutContext;
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{Builder, RunNow};

pub fn layout(dom: DomRef, device: Device) -> LayoutContext {
    let mut flow_constructor = FlowConstructor::new();
    dom.traverse(&mut flow_constructor);

    let mut layout_context = flow_constructor.layout_context();

    // create the page container entity (which defines the width of the root
    // element)
    let page_container = layout_context
        .create_entity()
        .with(entity::Position {
            x: PixelLength(0.0),
            y: PixelLength(0.0),
            width: device.width,
            height: device.height,
        })
        .build();

    let root_entity = dom.layout_entity.get().expect("No root entity");
    layout_context
        .write_storage()
        .insert(
            root_entity.0,
            entity::ContainingBlock {
                entity: page_container.into(),
            },
        )
        .unwrap();

    // Assign widths.
    let mut inline_size_context = inline_size::InlineSizeContext::new(device);
    inline_size_context.run_now(&layout_context.res);

    // Assign heights to all entities where it does not depend on their children.
    assign_heights::AssignHeights.run_now(&layout_context.res);

    // inline context layout
    inline::InlineLayout.run_now(&layout_context.res);

    layout_context.maintain();

    // create block formatting contexts
    block_formatting_context::BlockFormattingContextBuilder::new().run_now(&layout_context.res);

    layout_context.maintain();

    // Assign heights to the remaining entities.
    let mut block_size_context = block_size::BlockSizeContext {};
    block_size_context.run_now(&layout_context.res);

    // Calculate positions relative to the parent stacking context
    global_transform::GlobalTransform.run_now(&layout_context.res);

    layout_context
}
