use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;
use kuchiki::iter::NodeEdge;

use crate::render_tree::Content;

pub fn generate_render_tree(html: &str) -> crate::RenderTree {
    let parser = parse_html();
    let document = parser.one(html);

    let mut state = ParserState::new();

    for edge in document.traverse() {
	match edge {
            NodeEdge::Start(start) => {
                if let Some(elem_data) = start.as_element() {
                    state.element_start(elem_data);
                }
            },
            NodeEdge::End(end) => {

            }
        }
    }

    state.render_tree
}

// tracks state while traversing the html document and builds a render tree.
struct ParserState {
    // the root tree
    render_tree: crate::RenderTree,
    // a cursor into the `render_tree`.
    cursor: crate::RenderTree,
}

impl ParserState {
    fn new() -> Self {
        let render_tree = crate::RenderTree::new();
        ParserState {
            render_tree: render_tree.clone(),
            cursor: render_tree
        }
    }

    fn element_start(&mut self, elem: &kuchiki::ElementData) {
        println!("Element Start: {:?}", elem.name.local);


    }   

    fn append_child(&mut self, child: crate::RenderTree) {
        let current = &self.cursor.node().content;

        match *current.borrow_mut() {
            None => unimplemented!(),
            Some(Content::Text(_)) => unimplemented!(),
            Some(Content::Children(mut children)) => childrend.push(child),
        }
    }

    fn replace_content(&self, content: Option<Content>) {
	let current = &self.cursor.node().content;
        *current.borrow_mut() = content;
    }
}
