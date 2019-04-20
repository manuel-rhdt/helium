mod assign_heights;
// pub mod block_size;
pub mod construct;
pub mod entity;
// mod global_transform;
pub mod inline;
pub mod inline_size;
pub mod text;
pub mod normal_formatting;

use crate::layout::entity::LayoutContext;
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{Builder, Dispatcher, DispatcherBuilder, World};

pub fn layout(dom: DomRef, device: Device) -> LayoutContext {
    let inline_size_context = inline_size::InlineSizeContext::new(device);

    // here we set the layout passes
    let mut dispatcher: Dispatcher = DispatcherBuilder::new()
        .with(normal_formatting::BlockLevelLayout { page_width: device.width, page_height: device.height }, "layout", &[])
        // Assign widths.
        // .with(inline_size_context, "inline-size", &[])
        // Assign heights to all entities where it does not depend on their children.
        // .with(assign_heights::AssignHeights, "assign-heights", &["inline-size"])
        // block formatting
        // .with(block_formatting_context::BlockFormattingContextBuilder::new(), "block-formatting", &["assign-heights"])
        // Assign heights to the remaining entities.
        // .with(block_size::BlockSizeContext {}, "block-size", &["block-formatting"])
        // Calculate positions relative to the parent stacking context
        // .with(global_transform::GlobalTransform, "global-transform", &["block-size"])
        // Calculate positions relative to the parent sline
        .with(inline::InlinePositionAssign, "inline-position-assign", &["layout"])
        .build();

    let mut world = World::new();
    dispatcher.setup(&mut world.res);
    let mut flow_constructor = FlowConstructor::new(world);
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
    // layout_context
    //     .write_storage()
    //     .insert(
    //         root_entity.0,
    //         entity::ContainingBlock {
    //             entity: page_container.into(),
    //         },
    //     )
    //     .unwrap();

    // perform the layout passes
    dispatcher.dispatch(&layout_context.res);

    layout_context
}
