mod assign_heights;
// pub mod block_size;
pub mod construct;
pub mod entity;
// mod global_transform;
pub mod inline;
pub mod inline_size;
pub mod normal_formatting;
pub mod text;

use crate::layout::entity::LayoutContext;
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{Builder, Dispatcher, DispatcherBuilder, World};

pub fn layout(dom: DomRef, device: Device) -> LayoutContext {
    // here we set the layout passes
    let mut dispatcher: Dispatcher = DispatcherBuilder::new()
        .with(
            normal_formatting::BlockLevelLayout {
                page_width: device.width,
                page_height: device.height,
            },
            "layout",
            &[],
        )
        // Assign widths.
        // .with(inline_size_context, "inline-size", &[])
        // Assign heights to all entities where it does not depend on their children.
        // .with(assign_heights::AssignHeights, "assign-heights", &["inline-size"])
        // block formatting
        // .with(block_formatting_context::BlockFormattingContextBuilder::new(), "block-formatting", &["assign-heights"])
        // Assign heights to the remaining entities.
        // .with(block_size::BlockSizeContext {}, "block-size", &["block-formatting"])
        // Calculate positions relative to the parent line
        .with(
            inline::InlinePositionAssign,
            "inline-position-assign",
            &["layout"],
        )
        .build();

    let mut world = World::new();
    dispatcher.setup(&mut world.res);
    let mut flow_constructor = FlowConstructor::new(world);
    dom.traverse(&mut flow_constructor);

    let mut layout_context = flow_constructor.layout_context();

    // perform the layout passes
    dispatcher.dispatch(&layout_context.res);

    layout_context
}
