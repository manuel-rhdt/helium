//! Tests of the layout of a subtree at a width (see [`super::subtree`] and the methods of
//! `LayoutService` for a subtree). They ask the service in the middle of a real layout, through
//! the hooks of [`super::probe`], and compare with the layout of the same boxes in a document
//! where the page gives them the width.

use super::service::TreeService;
use super::*;
use crate::layout::construct::AssignLayoutEntity;
use crate::style::device::Device;
use crate::tree::{DomRef, TreeLinked};
use helium_paginate::{BoxRef, Item, LayoutService, ParId};
use std::cell::RefCell;
use std::rc::Rc;

type Probe = Box<dyn FnOnce(&mut TreeService<'_, '_, '_>, &DomRef)>;

/// Lays a document out. The probes are called in front of the pagination and behind it.
fn lay_out(html: &str, before: Option<Probe>, after: Option<Probe>) -> (World, DomRef) {
    let dom = crate::parser::generate_render_tree(html);
    let device = Device { width: PixelLength(793.7), height: PixelLength(1123.0) };
    let hook = |probe: Option<Probe>| {
        probe.map(|probe| {
            let dom = dom.clone();
            Box::new(move |service: &mut TreeService<'_, '_, '_>| probe(service, &dom)) as probe::Hook
        })
    };
    probe::BEFORE.with(|slot| *slot.borrow_mut() = hook(before));
    probe::AFTER.with(|slot| *slot.borrow_mut() = hook(after));
    let world = crate::layout::layout(dom.clone(), device, false, std::path::Path::new(".")).unwrap();
    (world, dom)
}

fn element(dom: &DomRef, id: &str) -> Option<DomRef> {
    if let Some(element) = dom.clone().into_element_ref() {
        let attributes = element.attributes.borrow();
        if attributes.iter().any(|(name, attribute)| &*name.local == "id" && &*attribute.value == id) {
            return Some(dom.clone());
        }
    }
    TreeLinked::children(dom).find_map(|child| element(&child, id))
}

fn entity(dom: &DomRef, id: &str) -> specs::Entity {
    element(dom, id).and_then(|node| node.layout_entity()).expect("an element with a box").0
}

/// The box of the element `id` and the paragraphs of the flow inside it, in order.
fn root_of(service: &TreeService<'_, '_, '_>, dom: &DomRef, id: &str) -> (BoxRef, Vec<ParId>) {
    let root = service.box_of(entity(dom, id)).expect("the element is a box of the flow");
    let span = service.tree.nodes[root.0 as usize].span.as_ref().unwrap();
    let pars = service.flow.items[span.start as usize..span.end as usize]
        .iter()
        .filter_map(|item| match item {
            Item::Paragraph { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    (root, pars)
}

fn height_of(world: &World, dom: &DomRef, id: &str) -> f32 {
    world.read_storage::<Position>().get(entity(dom, id)).expect("a position").height.0
}

fn width_of(world: &World, dom: &DomRef, id: &str) -> f32 {
    world.read_storage::<Position>().get(entity(dom, id)).expect("a position").width.0
}

/// What the service says of a subtree: the extent of the content, and the lines of every
/// paragraph in it, as the sum of their heights and their number.
#[derive(Debug, Clone, PartialEq)]
struct Answer {
    content: f64,
    lines: Vec<(f64, usize)>,
}

fn ask(service: &mut TreeService<'_, '_, '_>, dom: &DomRef, id: &str, inline: f32) -> Answer {
    let (root, pars) = root_of(service, dom, id);
    let content = service.content_extent(root, inline);
    let lines = pars
        .iter()
        .map(|&par| {
            let lines = service.lines_in(root, par, None, inline);
            (lines.iter().map(|line| line.extent).sum(), lines.len())
        })
        .collect();
    Answer { content, lines }
}

/// The answer of the service for `id` at the widths, in the middle of the layout of the document
/// (in front of the pagination).
fn answers(html: &str, id: &'static str, widths: &'static [f32]) -> Vec<Answer> {
    let found = Rc::new(RefCell::new(vec![]));
    let sink = found.clone();
    lay_out(
        html,
        Some(Box::new(move |service, dom| {
            for &width in widths {
                sink.borrow_mut().push(ask(service, dom, id, width));
            }
        })),
        None,
    );
    found.take()
}

const TEXT: &str = "the quick brown fox jumps over the lazy dog and then it rests in the shade of an old tree ";

fn page(body: &str) -> String {
    format!(
        "<style>@page {{ size: 1000px 4000px; margin: 0 }} body {{ margin: 0; font: 16px/20px serif }}
         p {{ margin: 8px 0; orphans: 1; widows: 1 }} {body}"
    )
}

/// A document with a box `x` and its content. `style` is the style of the box.
fn document(style: &str, content: &str) -> String {
    page(&format!("</style><div id=x style=\"{style}\">{content}</div>"))
}

/// The box is as wide as the page in the document that is asked, and `width` wide in the one
/// that gives the reference: the width is the width of its content box.
fn reference(style: &str, content: &str, width: f32) -> (f32, f32) {
    let html = document(&format!("{style}; width: {width}px; box-sizing: content-box"), content);
    let (world, dom) = lay_out(&html, None, None);
    (width_of(&world, &dom, "x"), height_of(&world, &dom, "x"))
}

/// The extent of the content of the box at some widths is the height that the layout of the page
/// gives it when its content box has that width.
fn assert_same_as_the_page(style: &str, content: &str) {
    const WIDTHS: &[f32] = &[137.0, 300.0, 611.0, 2.0];
    let asked = answers(&document(style, content), "x", WIDTHS);
    for (&width, answer) in WIDTHS.iter().zip(&asked) {
        let (reference_width, height) = reference(style, content, width);
        assert!((reference_width - width).abs() < 0.01, "{reference_width} for {width}");
        assert!(
            (answer.content - f64::from(height)).abs() < 0.01,
            "at {width} the service says {} and the page has {height} ({style}; {content})",
            answer.content
        );
    }
}

#[test]
fn the_content_of_a_plain_block_is_as_high_as_the_page_makes_it() {
    // (the margins of the paragraphs are inside the box: it is a formatting context of its own)
    assert_same_as_the_page(
        "align-content: start",
        &format!("<p>{TEXT}{TEXT}</p><p>{TEXT}</p><p>short</p>"),
    );
}

#[test]
fn the_content_of_a_box_with_padding_and_borders_is_as_high_as_the_page_makes_it() {
    assert_same_as_the_page(
        "padding: 7px 11px 13px 17px; border: 3px solid black; border-width: 3px 5px 9px 1px",
        &format!("<p>{TEXT}{TEXT}</p><p>{TEXT}</p>"),
    );
    // percentages in the padding of the box refer to the width that was asked for
    assert_same_as_the_page("padding: 2% 3%", &format!("<p>{TEXT}</p>"));
}

#[test]
fn the_content_of_nested_boxes_is_as_high_as_the_page_makes_it() {
    assert_same_as_the_page(
        "padding: 5px",
        &format!(
            "<p>{TEXT}</p>
             <div style=\"padding: 4px 9px; border: 2px solid; margin: 6px 3px\">
               <p>{TEXT}{TEXT}</p>
               <div style=\"margin-left: 20px; border-left: 5px solid\"><p>{TEXT}</p><p>{TEXT}</p></div>
             </div>
             <p>{TEXT}</p>"
        ),
    );
}

#[test]
fn the_content_of_a_box_with_percentages_is_as_high_as_the_page_makes_it() {
    // the width of the box itself does not matter: the width that was asked for is the one of
    // its content box
    assert_same_as_the_page(
        "width: 50%; padding: 3px",
        &format!(
            "<div style=\"width: 50%\"><p>{TEXT}{TEXT}</p></div>
             <div style=\"width: 33%; margin-left: 10%; padding: 0 2%\"><p>{TEXT}</p></div>
             <div style=\"width: 100%; max-width: 90px\"><p>{TEXT}</p></div>"
        ),
    );
}

#[test]
fn the_content_of_a_box_with_inline_blocks_is_as_high_as_the_page_makes_it() {
    assert_same_as_the_page(
        "padding: 4px",
        &format!(
            "<p>{TEXT} <span style=\"display: inline-block; width: 80px; height: 33px\"></span> {TEXT}</p>
             <p>{TEXT} <span style=\"display: inline-block; width: 40%; height: 25px; background: red\">x y</span> {TEXT}</p>
             <p><span style=\"display: inline-block; padding: 2px\">{TEXT}</span> {TEXT}</p>"
        ),
    );
}

#[test]
fn the_lines_of_a_paragraph_in_a_subtree_are_the_lines_of_its_box() {
    const WIDTHS: &[f32] = &[150.0, 420.0];
    let content = format!("<p id=a>{TEXT}{TEXT}{TEXT}</p><div style=\"padding: 0 30px\"><p id=b>{TEXT}{TEXT}</p></div>");
    let asked = answers(&document("padding: 5px", &content), "x", WIDTHS);
    for (&width, answer) in WIDTHS.iter().zip(&asked) {
        let html = document(&format!("padding: 5px; width: {width}px; box-sizing: content-box"), &content);
        let (world, dom) = lay_out(&html, None, None);
        let heights = [height_of(&world, &dom, "a"), height_of(&world, &dom, "b")];
        assert_eq!(answer.lines.len(), 2);
        for ((extent, count), height) in answer.lines.iter().zip(heights) {
            assert!(*count > 1);
            assert!((*extent - f64::from(height)).abs() < 0.01, "{extent} and {height} at {width}");
        }
    }
    // the lines are broken for the width of the box that holds the paragraph, not for the page
    assert!(asked[0].lines[0].1 > asked[1].lines[0].1);
}

/// The extents of the boxes and the gaps inside the box `x`, by the item of the flow.
fn extents_inside(html: &str, width: Option<f32>) -> Vec<f64> {
    let found = Rc::new(RefCell::new(vec![]));
    let sink = found.clone();
    let probe: Probe = Box::new(move |service, dom| {
        let (root, _) = root_of(service, dom, "x");
        let span = service.tree.nodes[root.0 as usize].span.clone().unwrap();
        for item in service.flow.items[span.start as usize..span.end as usize].to_vec() {
            if let Item::Box { node } | Item::Gap { node } = item {
                let extent = match width {
                    Some(width) => service.extent_in(root, node, width),
                    None => service.extent(node, 1000.0),
                };
                sink.borrow_mut().push(extent);
            }
        }
    });
    if width.is_some() {
        lay_out(html, Some(probe), None);
    } else {
        lay_out(html, None, Some(probe));
    }
    found.take()
}

#[test]
fn the_extent_of_a_box_inside_a_subtree_is_the_extent_in_the_page_of_that_width() {
    const WIDTH: f32 = 260.0;
    let content = format!(
        "<p>{TEXT}</p><div style=\"padding: 6px 0; margin: 12px 0\"><p>{TEXT}{TEXT}</p></div><p>{TEXT} {TEXT}</p>"
    );
    let style = "padding: 3px; align-content: start";
    let asked = extents_inside(&document(style, &content), Some(WIDTH));
    // the document in which the page gives the box the width, and the service asks for the
    // extents on the page
    let page = extents_inside(&document(&format!("{style}; width: {WIDTH}px; box-sizing: content-box"), &content), None);
    assert!(asked.len() >= 8, "{asked:?}");
    assert_eq!(asked.len(), page.len());
    for (asked, page) in asked.iter().zip(&page) {
        assert!((asked - page).abs() < 0.01, "{asked:?} and {page:?}");
    }
}

#[test]
fn asking_twice_gives_the_same_answer() {
    const WIDTHS: &[f32] = &[200.0, 333.0, 200.0, 333.0, 200.0];
    let content = format!(
        "<p>{TEXT}{TEXT}</p><p>{TEXT} <span style=\"display: inline-block; width: 30%; height: 20px\"></span> {TEXT}</p>"
    );
    let asked = answers(&document("padding: 3px", &content), "x", WIDTHS);
    assert_eq!(asked[0], asked[2]);
    assert_eq!(asked[0], asked[4]);
    assert_eq!(asked[1], asked[3]);
    assert_ne!(asked[0], asked[1]);
}

#[test]
fn the_answer_does_not_depend_on_what_was_laid_out_before() {
    // in front of the pagination, and behind it, when the galleys and the lines of the page are
    // there
    // (994 is the width that the page gives the box: the cache of Taffy has entries for it)
    const WIDTHS: &[f32] = &[170.0, 480.0, 994.0];
    let content = format!(
        "<p>{TEXT}{TEXT}</p><div style=\"padding: 2px 8px\"><p>{TEXT} <span style=\"display: inline-block; width: 45%; height: 20px\">a b c</span> {TEXT}</p></div>"
    );
    let html = document("padding: 3px", &content);
    let before = answers(&html, "x", WIDTHS);
    let found = Rc::new(RefCell::new(vec![]));
    let sink = found.clone();
    lay_out(
        &html,
        None,
        Some(Box::new(move |service, dom| {
            for &width in WIDTHS {
                sink.borrow_mut().push(ask(service, dom, "x", width));
            }
        })),
    );
    assert_eq!(before, found.take());
}

/// Lays a document out with a service that is asked about subtrees, in front of the pagination,
/// behind it, or both, and gives the layout of the document. It must be the layout of the
/// document that nobody asked about.
fn assert_not_changed_by_asking(html: &str, id: &'static str, widths: &'static [f32]) {
    let (world, dom) = lay_out(html, None, None);
    let expected = crate::layout::dump::dump_layout(&dom, &world);
    let ask_all = move || -> Probe {
        Box::new(move |service, dom| {
            for &width in widths {
                ask(service, dom, id, width);
            }
        })
    };
    for (before, after) in [(true, false), (false, true), (true, true)] {
        let (world, dom) = lay_out(html, before.then(ask_all), after.then(ask_all));
        assert_eq!(
            crate::layout::dump::dump_layout(&dom, &world),
            expected,
            "asking in front of the pagination: {before}, behind it: {after}"
        );
    }
}

#[test]
fn asking_for_a_subtree_does_not_change_the_layout_of_the_page() {
    let content = format!(
        "<p>{TEXT}{TEXT}</p>
         <div style=\"width: 60%; padding: 3px; border: 1px solid\"><p>{TEXT}{TEXT}</p></div>
         <p>{TEXT} <span style=\"display: inline-block; width: 50%; height: 20px; background: blue\">a b c d e f g</span> {TEXT}</p>
         <p>{TEXT}</p>"
    );
    // a box that is the whole width of the page and one that is narrower; widths that are
    // narrower and wider than the page
    assert_not_changed_by_asking(&document("padding: 5px 9px", &content), "x", &[90.0, 333.0, 1500.0]);
    assert_not_changed_by_asking(&document("width: 400px; margin: 0 auto", &content), "x", &[90.0, 400.0, 1500.0]);
}

#[test]
fn asking_for_a_subtree_does_not_change_the_pages_of_a_document_that_has_several() {
    // the content is on several pages of different widths: the boxes are cut by the windows
    let paragraphs = format!("<p>{TEXT}{TEXT}{TEXT}</p>").repeat(14);
    let html = format!(
        "<style>@page {{ size: 500px 300px; margin: 20px }} @page :first {{ margin: 20px 90px }}
         body {{ margin: 0; font: 16px/20px serif }} p {{ margin: 6px 0; orphans: 1; widows: 1 }}</style>
         <div id=x style=\"padding: 4px 8px\">{paragraphs}</div><div id=y><p>{TEXT}</p></div>"
    );
    assert_not_changed_by_asking(&html, "x", &[70.0, 210.0, 444.0]);
    assert_not_changed_by_asking(&html, "y", &[70.0, 444.0]);
}

#[test]
fn the_boxes_around_a_subtree_are_laid_out_again_behind_a_question_about_it() {
    // the box around `x` is neither at the start nor at the end of the page: the window does not
    // change it, so its layout would come from the cache of Taffy if the question left an entry
    // that says that its children are as the galley of the page had them
    // (and the width of the box is the width that the galley gave it, so that the entries of its
    // cache are the ones that the question is asked with)
    let content = format!(
        "<p>{TEXT}{TEXT}</p><div style=\"padding: 2px\"><p>{TEXT}</p></div>
         <p>{TEXT} <span style=\"display: inline-block; width: 35%; height: 20px\">a b c</span> {TEXT}</p>"
    );
    let html = page(&format!(
        "</style><div><p>in front</p></div><div id=wrapper style=\"padding: 1px\"><div id=x>{content}</div></div><div><p>behind</p></div>"
    ));
    assert_not_changed_by_asking(&html, "x", &[75.0, 998.0, 1500.0]);
}

// A part of a paragraph that ends at any break of the paragraph.

#[test]
fn a_part_of_a_paragraph_can_end_at_any_break() {
    use super::tree::LineCut;
    use helium_paginate::ParLine;
    let html = document("", &format!("<p id=a>{TEXT}{TEXT}{TEXT}{TEXT}</p>"));
    type Found = (Vec<ParLine>, Vec<ParLine>, Vec<ParLine>, Vec<ParLine>, Vec<ParLine>, usize);
    let found: Rc<RefCell<Option<Found>>> = Rc::new(RefCell::new(None));
    let sink = found.clone();
    lay_out(
        &html,
        Some(Box::new(move |service, dom| {
            let (root, pars) = root_of(service, dom, "x");
            let par = pars[0];
            let whole = service.lines_in(root, par, None, 250.0);
            // a break inside of the third line, which is not where a line of the paragraph ends
            let at = LineCut::of(whole[2].end).at - 2;
            let to = LineCut { at, fitness: 0 }.cut();
            let part = service.lines_to_in(root, par, None, to, 250.0);
            let again = service.lines_to_in(root, par, None, to, 250.0);
            // where a line of the paragraph ends, the part is the lines up to it
            let at_a_line = service.lines_to_in(root, par, None, whole[2].end, 250.0);
            // the next page goes on at the cut as the service made it
            let tail = service.lines_in(root, par, Some(part.last().unwrap().end), 250.0);
            *sink.borrow_mut() = Some((whole.to_vec(), part.to_vec(), again.to_vec(), at_a_line.to_vec(), tail.to_vec(), at));
        })),
        None,
    );
    let (whole, part, again, at_a_line, tail, at) = found.take().expect("the probe was called");
    assert!(whole.len() >= 6, "{} lines", whole.len());
    // the part ends at the break that was asked for, with a fitness class that the next page uses
    assert_eq!(LineCut::of(part.last().unwrap().end).at, at);
    assert!(LineCut::of(part.last().unwrap().end).fitness < 4);
    // it has about as many lines as the part of the paragraph in front of that break
    assert!((2..=4).contains(&part.len()), "{} lines", part.len());
    assert_eq!(part, again);
    assert_eq!(at_a_line.as_slice(), &whole[..=2]);
    // the rest of the paragraph is what is left of it
    assert!(!tail.is_empty());
    let (before, after) = (part.len(), tail.len());
    assert!((before + after).abs_diff(whole.len()) <= 1, "{before} + {after} lines of {}", whole.len());
}

/// Pages of 100px with lines of 20px: five lines to a page.
fn narrow_pages(paragraph: &str) -> String {
    format!(
        "<style>@page {{ size: 300px 100px; margin: 0 }} body {{ margin: 0; font: 16px/20px serif }}
         p {{ margin: 0; orphans: 1; widows: 1 }}</style><p id=a>{paragraph}</p>"
    )
}

#[test]
fn a_solution_with_pages_that_end_at_any_break_of_a_paragraph_is_laid_out() {
    use super::tree::LineCut;
    use helium_paginate::{Position as FlowPosition, Solution};
    use specs::Join;
    // a solver that fills every page as the paragraph is broken for it, and then ends it one break
    // earlier, wherever that is
    let arbitrary = Rc::new(std::cell::Cell::new(0usize));
    let count = arbitrary.clone();
    let solve: probe::Solve = Box::new(move |kernel| {
        let first = kernel.first_page();
        let mut pages = vec![first];
        let mut start = first;
        loop {
            let fills = kernel.candidates(&start);
            let Some(last) = fills.last() else { break };
            if kernel.is_end(last.end) {
                break;
            }
            let Some(full) = fills.iter().rev().find(|fill| matches!(kernel.position(fill.end), FlowPosition::InParagraph { .. }))
            else {
                break;
            };
            let FlowPosition::InParagraph { item, cut } = kernel.position(full.end) else { unreachable!() };
            let earlier = LineCut { at: LineCut::of(cut).at - 1, fitness: 0 }.cut();
            let fill = kernel.fill_at(&start, item, earlier).expect("a page can end one break earlier");
            // (the page does not end where the paragraph is broken for the page)
            if !fills.iter().any(|other| other.end == fill.end) {
                count.set(count.get() + 1);
            }
            start = kernel.next_page(&start, &fill).expect("the paragraph goes on");
            pages.push(start);
        }
        Solution { pages }
    });
    probe::SOLVE.with(|slot| *slot.borrow_mut() = Some(solve));
    let text = TEXT.repeat(12);
    let (world, dom) = lay_out(&narrow_pages(&text), None, None);
    let pages = world.read_resource::<crate::layout::pages::Pages>().0.len();
    assert!(pages >= 3, "{pages} pages");
    // all of the pages but the last end at a break that the paragraph is not broken at
    assert_eq!(arbitrary.get(), pages - 1);
    // every page has lines that fit it: the paragraph is cut into the same lines as the pages hold
    let positions = world.read_storage::<Position>();
    let lines = world.read_storage::<crate::layout::components::Paragraph>();
    let mut per_page = vec![0usize; pages];
    for (position, _) in (&positions, &lines).join() {
        assert!(position.y.0 + position.height.0 <= 100.5, "a line ends at {}, below the page", position.y.0 + position.height.0);
        per_page[position.page] += 1;
    }
    assert!(per_page.iter().all(|&count| count > 0), "{per_page:?}");
    let _ = dom;
}

// The ways to break a paragraph.

fn variants_of(extra: u32, justified: bool) -> (Vec<Rc<[helium_paginate::ParLine]>>, Rc<[helium_paginate::ParLine]>, Vec<Rc<[helium_paginate::ParLine]>>, Rc<[helium_paginate::ParLine]>) {
    let style = if justified { "text-align: justify" } else { "" };
    let html = document(style, &format!("<p id=a>{TEXT}{TEXT}{TEXT}{TEXT}</p>"));
    let found = Rc::new(RefCell::new(None));
    let sink = found.clone();
    lay_out(
        &html,
        Some(Box::new(move |service, dom| {
            let (root, pars) = root_of(service, dom, "x");
            let par = pars[0];
            let variants = service.variants_in(root, par, None, 250.0, extra);
            let whole = service.lines_in(root, par, None, 250.0);
            // the rest of the paragraph after the end of its third line
            let cut = Some(whole[2].end);
            let tail_variants = service.variants_in(root, par, cut, 250.0, extra);
            let tail = service.lines_in(root, par, cut, 250.0);
            *sink.borrow_mut() = Some((variants, whole, tail_variants, tail));
        })),
        None,
    );
    found.take().expect("the probe was called")
}

fn total(lines: &[helium_paginate::ParLine]) -> f64 {
    lines.iter().map(|line| line.demerits).sum()
}

#[test]
fn a_paragraph_without_extra_lines_has_one_way_to_be_broken() {
    let (variants, whole, tail_variants, tail) = variants_of(0, false);
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0], whole);
    assert_eq!(tail_variants.len(), 1);
    assert_eq!(tail_variants[0], tail);
}

#[test]
fn a_paragraph_can_be_broken_into_a_line_more_or_fewer() {
    for justified in [false, true] {
        let (variants, whole, _, _) = variants_of(1, justified);
        // the first is what the paragraph has, and the others have other numbers of lines, in order
        assert_eq!(variants[0], whole);
        assert!(variants.len() >= 2, "justified {justified}: {} ways to break the paragraph", variants.len());
        let counts: Vec<usize> = variants[1..].iter().map(|lines| lines.len()).collect();
        assert!(counts.windows(2).all(|pair| pair[0] < pair[1]), "{counts:?}");
        for lines in &variants[1..] {
            assert_ne!(lines.len(), whole.len());
            assert!(lines.len().abs_diff(whole.len()) <= 1);
            // the optimum has the least demerits, and every line has the height of a line of text
            assert!(total(lines) >= total(&whole) - 1e-6, "{} and {}", total(lines), total(&whole));
            assert!(lines.iter().all(|line| (line.extent - 20.0).abs() < 0.01));
        }
        // every line has demerits that count: they are not zero
        assert!(variants[1..].iter().all(|lines| lines.iter().all(|line| line.demerits > 0.0)));
    }
}

#[test]
fn the_rest_of_a_paragraph_after_a_cut_has_ways_to_be_broken_of_its_own() {
    use super::tree::LineCut;
    let (_, whole, variants, tail) = variants_of(1, false);
    // the first is the rest of the paragraph, as the lines after the cut are
    assert_eq!(variants[0], tail);
    assert!(variants.len() >= 2);
    // the others are broken from the cut, with the state that the cut carries: the rest of the
    // paragraph with another number of lines ends where the paragraph ends
    for lines in &variants[1..] {
        assert_ne!(lines.len(), tail.len());
        assert_eq!(lines.last().map(|line| LineCut::of(line.end).at), tail.last().map(|line| LineCut::of(line.end).at));
    }
    assert!(whole.len() > 3);
}

#[test]
fn the_ways_to_break_a_paragraph_are_kept_by_the_service() {
    let html = document("", &format!("<p id=a>{TEXT}{TEXT}{TEXT}</p>"));
    let same = Rc::new(RefCell::new(false));
    let sink = same.clone();
    lay_out(
        &html,
        Some(Box::new(move |service, dom| {
            let (root, pars) = root_of(service, dom, "x");
            let first = service.variants_in(root, pars[0], None, 250.0, 1);
            let second = service.variants_in(root, pars[0], None, 250.0, 1);
            let other = service.variants_in(root, pars[0], None, 250.0, 2);
            *sink.borrow_mut() = first.len() == second.len()
                && first.iter().zip(&second).all(|(a, b)| Rc::ptr_eq(a, b))
                && other.len() >= first.len();
        })),
        None,
    );
    assert!(same.take());
}
