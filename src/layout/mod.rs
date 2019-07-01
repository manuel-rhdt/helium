pub mod construct;
pub mod entity;
pub mod inline;
pub mod inline_size;
pub mod normal_formatting;
pub mod text;

use crate::layout::entity::LayoutContext;
use crate::style::device::Device;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{Dispatcher, DispatcherBuilder, World, WorldExt};

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
        .with(
            inline::InlinePositionAssign,
            "inline-position-assign",
            &["layout"],
        )
        .build();

    let mut world = World::new();
    dispatcher.setup(&mut world);
    let mut flow_constructor = FlowConstructor::new(world);
    dom.traverse(&mut flow_constructor);

    let layout_context = flow_constructor.layout_context();

    // perform the layout passes
    dispatcher.dispatch(&layout_context);

    layout_context
}
