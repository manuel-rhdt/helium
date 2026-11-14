pub mod absolute;
pub mod bidi;
pub mod check;
pub mod components;
pub mod construct;
pub mod dump;
pub mod inline;
pub mod inline_size;
mod margin_boxes;
mod mathml;
pub mod normal_formatting;
mod paginate;
pub mod pages;
pub(crate) mod paragraph;
mod relative_positioning;
mod taffy_layout;
pub mod text;

use crate::PixelLength;
use crate::layout::components::{
    EntityHrefs, GoToDestination, Hyperlinks, LayoutContext, Position, RootNode,
};
use crate::style::{
    device::Device,
    properties::computed::{GenericSize, PageSize, StyleBuilder},
};
use crate::traverse::Traverse;
use crate::tree::DomRef;
use construct::FlowConstructor;

use components::{EdgeValues, TextList};
use relative_positioning::RelativePositioningSystem;
use specs::{RunNow, World, WorldExt};
use std::path::Path;
use std::time::Instant;

use log::info;

pub type FontMap = std::collections::HashMap<String, std::sync::Arc<text::Font>>;

/// Collects every layout entity that is inside an `<a href>` element and remembers its href.
///
/// Only elements and text nodes with a layout entity are recorded; the entity of the `<a>`
/// element itself is not recorded because its content is drawn by its descendants (text runs,
/// line boxes, ...).
fn collect_entity_hrefs(dom: &DomRef) -> EntityHrefs {
    use crate::parser::ElementData;
    use html5ever::{QualName, local_name, ns};

    fn walk(
        node: &DomRef,
        current_href: Option<&str>,
        map: &mut std::collections::HashMap<u32, String>,
    ) {
        // If this node is an element with an href, it becomes the new current link for its
        // descendants. Other elements pass the current link through. An `<a>` without an href
        // ends the link chain.
        let owned_href;
        let mut node_href = current_href;
        if let Some(element) = node.clone().into_element_ref() {
            let ElementData {
                name, attributes, ..
            } = &*element;
            if name.ns == ns!(html) && name.local == local_name!("a") {
                if let Some(href) =
                    attributes
                        .borrow()
                        .get(&QualName::new(None, ns!(), local_name!("href")))
                {
                    owned_href = Some(href.value.clone());
                    node_href = owned_href.as_deref();
                } else {
                    node_href = None;
                }
            }
        }

        // Record the node's layout entity, if it has one and is inside a link.
        if let (Some(href), Some(entity)) = (node_href, node.layout_entity.get()) {
            let id = entity.0.id();
            map.entry(id).or_insert_with(|| href.to_owned());
        }

        for child in crate::tree::TreeLinked::children(node) {
            walk(&child, node_href, map);
        }
    }

    let mut map = std::collections::HashMap::new();
    walk(dom, None, &mut map);
    EntityHrefs(map)
}

/// Resolves cross-reference hyperlinks (`<a href="#fragment">`) to destinations.
///
/// Returns a map `fragment -> entity`: the layout entity of the element that has this `id`, to be
/// looked up in the world's `Position` storage once the positions are final.
fn collect_id_targets(dom: &DomRef) -> std::collections::HashMap<String, specs::Entity> {
    use crate::parser::ElementData;
    use html5ever::{QualName, local_name, ns};

    fn walk(node: &DomRef, targets: &mut std::collections::HashMap<String, specs::Entity>) {
        if let Some(element) = node.clone().into_element_ref() {
            let ElementData {
                name, attributes, ..
            } = &*element;
            if name.ns == ns!(html) {
                if let Some(id) =
                    attributes
                        .borrow()
                        .get(&QualName::new(None, ns!(), local_name!("id")))
                {
                    if let Some(entity) = node.layout_entity.get() {
                        targets
                            .entry(id.value.as_ref().to_owned())
                            .or_insert(entity.0);
                    }
                }
            }
        }
        for child in crate::tree::TreeLinked::children(node) {
            walk(&child, targets);
        }
    }

    let mut targets = std::collections::HashMap::new();
    walk(dom, &mut targets);
    targets
}

/// Rewrites the destinations of cross-reference hyperlinks.
///
/// A link with `href="#fragment"` becomes a `GoTo` link to the position of the element with
/// `id="fragment"`. Links pointing at unknown fragments keep `destination: None` (no jump).
/// Must run after pagination, when every position has its final page and page-relative
/// coordinates.
pub fn resolve_hyperlink_destinations(world: &mut World, dom: &DomRef) {
    let targets = collect_id_targets(dom);
    let positions = world.read_storage::<Position>();
    let mut hyperlinks = world.write_resource::<Hyperlinks>();

    for link in &mut hyperlinks.0 {
        let Some(fragment) = link.href.strip_prefix('#') else {
            continue;
        };
        if fragment.is_empty() {
            // `href="#"` jumps to the top of the document (page 0, y = 0).
            link.destination = Some(GoToDestination {
                page: 0,
                x: PixelLength(0.0),
                y: PixelLength(0.0),
            });
            continue;
        }
        let Some(&entity) = targets.get(fragment) else {
            continue;
        };
        let Some(&Position { x, y, page, .. }) = positions.get(entity) else {
            continue;
        };
        link.destination = Some(GoToDestination { page, x, y });
    }
}

pub fn layout(
    dom: DomRef,
    device: Device,
    crop: bool,
    stylesheet_base_dir: &Path,
) -> Result<LayoutContext, String> {
    // here we set the layout passes
    let world = World::new();

    let mut time_before = Instant::now();
    let mut flow_constructor = FlowConstructor::new(world, stylesheet_base_dir.to_path_buf());
    dom.traverse(&mut construct::StylesheetReader(&mut flow_constructor));
    dom.traverse(&mut flow_constructor);
    info!("TIMED: construct {:?}", time_before.elapsed());

    // dom.serialize_dot_graph(&mut std::fs::File::create("graph.dot").unwrap())
    //     .unwrap();

    // get page style
    let mut style_builder = StyleBuilder::new(Default::default());
    for rule in flow_constructor.styles.rules_iter() {
        if rule.page_selector() == Some(&crate::style::parse::PageSelector::any()) {
            for declaration in rule.properties.declarations() {
                style_builder.add_declaration(declaration);
            }
        }
    }
    let style = style_builder.build();

    // Page margins cannot be negative: a negative margin would move the content, and with it
    // the text, off the page where it is silently lost. Clamp instead of rendering a blank
    // document.
    let page_margin = |value: GenericSize, container_size: PixelLength| {
        value
            .absolute(Default::default(), container_size)
            .max(PixelLength(0.0))
    };
    let margin;
    let page_width;
    let page_height;
    if crop {
        margin = Default::default();
        page_width = PixelLength(0.0);
        page_height = PixelLength(0.0);
    } else {
        // `size` replaces the default size and the `width` and `height` of the page
        let (mut width, mut height) = (
            style.position.width.absolute(device.width, device.width),
            style.position.height.absolute(device.width, device.width),
        );
        match style.page.size {
            PageSize::Auto => {}
            PageSize::Dimensions { width: w, height: h } => (width, height) = (w, h),
            PageSize::Orientation { landscape } => {
                if (width < height) == landscape {
                    (width, height) = (height, width);
                }
            }
        }
        page_width = width;
        page_height = height;
        margin = EdgeValues {
            top: page_margin(style.position.margin_top, width),
            left: page_margin(style.position.margin_left, width),
            bottom: page_margin(style.position.margin_bottom, width),
            right: page_margin(style.position.margin_right, width),
        };
    }

    // The left and right margins of the kinds of pages (the inner and outer margins of a book)
    let rtl = flow_constructor.is_rtl();
    let mut page_styles = pages::PageStyles::default();
    let mut page_margins = pages::PageMargins {
        rtl,
        base: margin,
        ..Default::default()
    };
    if !crop {
        page_margins.names = pages::selected_names(&flow_constructor.styles);
        page_styles.names = page_margins.names.clone();
        for page in pages::representative_pages(&page_margins.names, rtl) {
            let mut style_builder = StyleBuilder::new(Default::default());
            for rule in pages::matching_rules(&flow_constructor.styles, &page) {
                for declaration in rule.properties.declarations() {
                    style_builder.add_declaration(declaration);
                }
            }
            let style = style_builder.build();
            page_styles
                .pages
                .insert(page.clone(), std::sync::Arc::new(style.clone()));
            page_margins.pages.insert(
                page,
                EdgeValues {
                    top: page_margin(style.position.margin_top, page_width),
                    left: page_margin(style.position.margin_left, page_width),
                    bottom: page_margin(style.position.margin_bottom, page_width),
                    right: page_margin(style.position.margin_right, page_width),
                },
            );
        }
    }

    if page_margins.left_right_conflicts() {
        log::warn!(
            "the top and bottom margins and the width of left and right pages differ: only the \
             left and right margins with the same sum are supported for them"
        );
    }

    let margin_boxes = if crop {
        Default::default()
    } else {
        flow_constructor.resolve_margin_boxes()
    };

    // the style sheets and the `foreignObject` elements of the SVG images, for their content
    let (shared_styles, islands) = flow_constructor.take_shared();
    let mut layout_context = flow_constructor.layout_context();
    layout_context.insert(margin_boxes);
    layout_context.insert(page_margins);
    layout_context.insert(page_styles);

    // Record which layout entities are part of a hyperlink so that the clickable areas can be
    // computed when the line boxes are created.
    let entity_hrefs = collect_entity_hrefs(&dom);
    layout_context.insert(entity_hrefs);
    // The logical structure (tagged PDF) follows the DOM; the content that is painted below is
    // tagged with the nodes of this tree.
    layout_context.insert(crate::tagging::StructureTree::build(&dom));
    layout_context.insert(Hyperlinks::default());

    layout_context.register::<components::IntrinsicSize>();
    layout_context.register::<components::ContainingBlock>();

    let mut math_renderer = mathml::MathFormat {};
    math_renderer.setup(&mut layout_context);

    time_before = Instant::now();
    math_renderer.run_now(&layout_context);
    info!("TIMED: math {:?}", time_before.elapsed());

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
    let root_entity = root_entity.expect("Could not find root entity.");
    time_before = Instant::now();
    taffy_layout::TaffyBlockLayout {
        root_entity,
        page_width,
        page_height,
        margin,
    }
    .run(&mut layout_context)?;
    info!("TIMED: block layout {:?}", time_before.elapsed());

    let mut paginator = paginate::Paginator { page_height };
    paginator.setup(&mut layout_context);

    time_before = Instant::now();
    paginator.run_now(&layout_context);
    info!("TIMED: paginate {:?}", time_before.elapsed());

    let mut place_inline_blocks = inline::PlaceInlineBlocks;
    place_inline_blocks.setup(&mut layout_context);
    place_inline_blocks.run_now(&layout_context);

    let mut line_box_creator = inline::LineBoxCreator;
    line_box_creator.setup(&mut layout_context);
    time_before = Instant::now();
    line_box_creator.run_now(&layout_context);
    info!("TIMED: line boxes {:?}", time_before.elapsed());

    let mut list_marker_creator = inline::ListMarkerCreator;
    list_marker_creator.setup(&mut layout_context);
    time_before = Instant::now();
    list_marker_creator.run_now(&layout_context);
    info!("TIMED: list markers {:?}", time_before.elapsed());

    let mut relative_positioning_system = RelativePositioningSystem;
    relative_positioning_system.run_now(&mut layout_context);

    // at this point the positions of all elements are fixed and should not be changed anymore

    let mut math_layout = mathml::LayoutMathGlyphs {};
    math_layout.setup(&mut layout_context);
    time_before = Instant::now();
    math_layout.run_now(&layout_context);
    info!("TIMED: math paint {:?}", time_before.elapsed());

    // headers and footers need the final number of pages
    margin_boxes::MarginBoxPaint.run_now(&layout_context);

    let mut pages_create = normal_formatting::PagesCreate {
        page_width,
        page_height,
    };
    pages_create.setup(&mut layout_context);
    pages_create.run_now(&layout_context);

    // Resolve cross-reference links (`href="#..."`) to their destinations now that all positions
    // are final. Only possible here because the page of every element is only known after
    // pagination.
    resolve_hyperlink_destinations(&mut layout_context, &dom);

    layout_islands(&mut layout_context, shared_styles, islands, stylesheet_base_dir)?;

    Ok(layout_context)
}

/// Lays out the content of the `foreignObject` elements of the SVG images, each as a page of the
/// size of the element, and stores the pages in `main` (`display_list::IslandPages`).
///
/// The content of an element can contain SVG images with `foreignObject` elements of their own.
/// They are added to the list and come after the element that has them.
fn layout_islands(
    main: &mut World,
    mut styles: crate::style::parse::Styles,
    mut islands: Vec<construct::IslandSpec>,
    base_dir: &Path,
) -> Result<(), String> {
    let mut pages = vec![];
    let mut index = 0;
    while index < islands.len() {
        let spec = islands[index].clone();
        index += 1;
        let size = (PixelLength(spec.width), PixelLength(spec.height));
        if spec.width <= 0.0 || spec.height <= 0.0 {
            pages.push(Default::default());
            continue;
        }
        let fonts: FontMap = (*main.read_resource::<FontMap>()).clone();
        let mut constructor = FlowConstructor::for_island(
            World::new(),
            base_dir.to_path_buf(),
            std::mem::take(&mut styles),
            fonts,
            (spec.node.clone(), spec.width, spec.height),
            std::mem::take(&mut islands),
        );
        constructor.construct_island();
        let (back_styles, back_islands) = constructor.take_shared();
        styles = back_styles;
        islands = back_islands;
        let mut world = constructor.layout_context();
        run_island_passes(&mut world, size.0, size.1)?;
        let mut printed = crate::display_list::generate_display_list(&mut world);
        // the fonts that the content uses are embedded with the rest
        {
            let island_fonts = world.read_resource::<FontMap>();
            let mut main_fonts = main.write_resource::<FontMap>();
            for (name, font) in island_fonts.iter() {
                main_fonts.entry(name.clone()).or_insert_with(|| font.clone());
            }
        }
        // the first page is the visible one; what does not fit is cut off, like the content of
        // a `foreignObject`
        let mut page = if printed.pages.is_empty() {
            Default::default()
        } else {
            printed.pages.swap_remove(0)
        };
        page.width = size.0;
        page.height = size.1;
        pages.push(page);
    }
    main.insert(crate::display_list::IslandPages(pages));
    Ok(())
}

/// The layout of a document that is one block of a given size, like a page: no margins, no
/// headers or footers, no links and no structure.
fn run_island_passes(world: &mut World, width: PixelLength, height: PixelLength) -> Result<(), String> {
    world.insert(margin_boxes::MarginBoxes::default());
    world.insert(pages::PageMargins::default());
    world.insert(pages::PageStyles::default());
    world.insert(EntityHrefs::default());
    world.insert(crate::tagging::StructureTree::default());
    world.insert(Hyperlinks::default());
    world.register::<components::IntrinsicSize>();
    world.register::<components::ContainingBlock>();

    let mut math_renderer = mathml::MathFormat {};
    math_renderer.setup(world);
    math_renderer.run_now(world);

    let mut break_points_finder = inline::FindPossibleLineBreaks;
    break_points_finder.run_now(world);

    world.insert(TextList::default());
    world.insert(components::AbsoluteGroups::default());
    let margin = EdgeValues::default();
    world.insert(components::PageGeometry { width, height, margin });
    let RootNode(root_entity) = *world.read_resource();
    let root_entity = root_entity.ok_or("the content of a foreignObject has no root")?;
    taffy_layout::TaffyBlockLayout {
        root_entity,
        page_width: width,
        page_height: height,
        margin,
    }
    .run(world)?;

    let mut paginator = paginate::Paginator { page_height: height };
    paginator.setup(world);
    paginator.run_now(world);

    let mut place_inline_blocks = inline::PlaceInlineBlocks;
    place_inline_blocks.setup(world);
    place_inline_blocks.run_now(world);

    let mut line_box_creator = inline::LineBoxCreator;
    line_box_creator.setup(world);
    line_box_creator.run_now(world);

    let mut list_marker_creator = inline::ListMarkerCreator;
    list_marker_creator.setup(world);
    list_marker_creator.run_now(world);

    RelativePositioningSystem.run_now(world);

    let mut math_layout = mathml::LayoutMathGlyphs {};
    math_layout.setup(world);
    math_layout.run_now(world);

    let mut pages_create = normal_formatting::PagesCreate {
        page_width: width,
        page_height: height,
    };
    pages_create.setup(world);
    pages_create.run_now(world);
    Ok(())
}
