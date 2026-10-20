pub mod absolute;
pub mod check;
pub mod components;
pub mod construct;
pub mod dump;
pub mod inline;
pub mod inline_size;
pub mod normal_formatting;
mod paginate;
pub(crate) mod paragraph;
mod relative_positioning;
pub mod text;
pub(crate) mod lscheme;

use crate::layout::components::{LayoutContext, RootNode};
use crate::style::{device::Device, properties::computed::StyleBuilder};
use crate::traverse::Traverse;
use crate::tree::DomRef;
use crate::PixelLength;
use construct::FlowConstructor;

use components::{EdgeValues, TextList};
use relative_positioning::RelativePositioningSystem;
use specs::{RunNow, World, WorldExt};
use std::time::Instant;

use log::info;

pub type FontMap = std::collections::HashMap<String, text::Font>;

pub fn layout(dom: DomRef, device: Device, crop: bool) -> Result<LayoutContext, lscheme::RuntimeError> {
    // here we set the layout passes
    let world = World::new();

    let mut time_before = Instant::now();
    let mut flow_constructor = FlowConstructor::new(world);
    dom.traverse(&mut flow_constructor);
    info!("TIMED: construct {:?}", time_before.elapsed());

    // dom.serialize_dot_graph(&mut std::fs::File::create("graph.dot").unwrap())
    //     .unwrap();

    // get page style
    let mut style_builder = StyleBuilder::new(Default::default());
    for rule in flow_constructor.styles.rules_iter() {
        if rule.is_page_declaration() {
            for declaration in rule.properties.declarations() {
                style_builder.add_declaration(declaration);
            }
        }
    }
    let style = style_builder.build();

    let margin;
    let page_width;
    let page_height;
    if crop {
        margin = Default::default();
        page_width = PixelLength(0.0);
        page_height = PixelLength(0.0);
    } else {
        margin = EdgeValues {
            top: style
                .position
                .margin_top
                .absolute(Default::default(), device.height),
            left: style
                .position
                .margin_left
                .absolute(Default::default(), device.width),
            bottom: style
                .position
                .margin_bottom
                .absolute(Default::default(), device.height),
            right: style
                .position
                .margin_right
                .absolute(Default::default(), device.width),
        };
        page_width = style.position.width.absolute(device.width, device.width);
        page_height = style.position.height.absolute(device.width, device.width);
    }

    let mut layout_context = flow_constructor.layout_context();

    layout_context.register::<components::IntrinsicSize>();
    layout_context.register::<components::ContainingBlock>();

    let mut break_points_finder = inline::FindPossibleLineBreaks;
    time_before = Instant::now();
    break_points_finder.run_now(&layout_context);
    info!("TIMED: line breaks {:?}", time_before.elapsed());

    // perform the layout routines
    layout_context.insert(TextList::default());
    layout_context.insert(components::AbsoluteGroups::default());
    layout_context.insert(components::PageGeometry {
        width: page_width,
        height: page_height,
        margin,
    });
    let RootNode(root_entity) = *layout_context.read_resource();
    let block_layout = lscheme::scheme_layout::SchemeBlockLayout {
        root_entity: root_entity.expect("Could not find root entity."),
        page_width,
        page_height,
        margin,
    };
    time_before = Instant::now();
    block_layout.run(&mut layout_context)?;
    info!("TIMED: block layout {:?}", time_before.elapsed());

    let mut paginator = paginate::Paginator {
        page_height,
        margin,
    };

    time_before = Instant::now();
    paginator.run_now(&layout_context);
    info!("TIMED: paginate {:?}", time_before.elapsed());

    let mut line_box_creator = inline::LineBoxCreator;
    line_box_creator.setup(&mut layout_context);
    time_before = Instant::now();
    line_box_creator.run_now(&layout_context);
    info!("TIMED: line boxes {:?}", time_before.elapsed());

    let mut relative_positioning_system = RelativePositioningSystem;
    relative_positioning_system.run_now(&mut layout_context);

    // at this point the positions of all elements are fixed and should not be changed anymore

    let mut pages_create = normal_formatting::PagesCreate {
        page_width,
        page_height,
    };
    pages_create.setup(&mut layout_context);
    pages_create.run_now(&layout_context);

    Ok(layout_context)
}
