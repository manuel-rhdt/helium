use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;
use kuchiki::iter::NodeEdge;

use crate::render_tree::Content;

use std::cell::RefCell;

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
                if let Some(text) = start.as_text() {
                    state.text(text);
                }
            },
            NodeEdge::End(end) => {

            }
        }
    }

    println!("{:?}", state.render_tree);
    state.render_tree
}

// tracks state while traversing the html document and builds a render tree.
#[derive(Debug)]
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
        if &elem.name.local == "div" {
            // this is appropriate for all block level elements
            self.append_child(crate::RenderTree::new());
        }
    }   

    fn element_end(&mut self, elem: &kuchiki::ElementData) {
       // put the cursor back to the parent 
    }


    fn text(&mut self, text: &RefCell<String>) {
        println!("found text node: {:?}", text);
        let current = &self.cursor.content;

        let mut replace_content = false;
        match &mut *current.borrow_mut() {
            None => { replace_content = true},
            Some(Content::Text(previous_text)) => previous_text.push_str(&text.borrow()),
            Some(Content::Children(children)) => unimplemented!()
        }

        if replace_content {
            self.replace_content(Some(Content::Text(text.borrow().clone())));
        }
    }

    fn append_child(&mut self, child: crate::RenderTree) {
        let current = &self.cursor.content;

        let mut replace_content = false;

        match &mut *current.borrow_mut() {
            None => { replace_content = true; },
            Some(Content::Text(_)) => unimplemented!(),
            Some(Content::Children(children)) => children.push(child.clone()),
        }

        if replace_content {
            self.replace_content(Some(Content::Children(vec![child.clone()])));
        }

        self.cursor = child;
    }

    fn replace_content(&self, content: Option<Content>) {
	let current = &self.cursor.content;
        *current.borrow_mut() = content;
    }
}
