pub mod components;
pub mod construct;
pub mod inline;
pub mod inline_size;
pub mod normal_formatting;
pub(crate) mod paragraph;
pub mod text;

use crate::layout::components::LayoutContext;
use crate::style::device::Device;
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use specs::{RunNow, World, WorldExt};

pub fn layout(dom: DomRef, device: Device) -> LayoutContext {
    // here we set the layout passes
    let world = World::new();
    let mut flow_constructor = FlowConstructor::new(world);
    dom.traverse(&mut flow_constructor);

    let mut layout_context = flow_constructor.layout_context();

    let mut break_points_finder = inline::FindPossibleLineBreaks;
    break_points_finder.run_now(&layout_context);

    // perform the layout passes
    let mut block_layout = normal_formatting::BlockLevelLayout {
        page_width: device.width,
        page_height: device.height,
        world: &layout_context,
    };
    block_layout.run();

    let mut line_box_creator = inline::LineBoxCreator;
    line_box_creator.setup(&mut layout_context);
    line_box_creator.run_now(&layout_context);

    normal_formatting::PagesCreate.setup(&mut layout_context);
    normal_formatting::PagesCreate.run_now(&layout_context);

    layout_context
}
