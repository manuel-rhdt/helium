mod assign_heights;
pub mod block;
pub mod block_size;
pub mod construct;
pub mod entity;
pub mod inline_size;

use crate::layout::entity::LayoutContext;
use crate::style::device::Device;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{Builder, RunNow};

pub fn layout(dom: DomRef, device: Device) -> LayoutContext {
    let mut flow_constructor = FlowConstructor::new();
    dom.traverse(&mut flow_constructor);

    let mut layout_context = flow_constructor.layout_context();
    let page_container = layout_context
        .create_entity()
        .with(entity::Position {
            x: 0.0,
            y: 0.0,
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

    let mut inline_size_context = inline_size::InlineSizeContext::new(device);
    inline_size_context.run_now(&layout_context.res);

    // assign_heights
    assign_heights::AssignHeights.run_now(&layout_context.res);

    let mut block_size_context = block_size::BlockSizeContext {};
    block_size_context.run_now(&layout_context.res);

    layout_context
}
