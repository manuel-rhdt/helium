use kuchiki::iter::NodeEdge;
use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;

use crate::flow::{Flow, FlowType, FormattingContext};
use crate::style::{Display, Style};

use std::cell::RefCell;

pub fn generate_render_tree(html: &str) -> crate::FlowTree {
    let parser = parse_html();

    // this is the dom tree
    let document = parser.one(html);

    let mut state = ParserState::new();

    for edge in document.traverse() {
        match edge {
            NodeEdge::Start(start) => {
                let style = crate::style::style_for_element(start.clone());
                if let Some(elem_data) = start.as_element() {
                    state.element_start(elem_data, style);
                }
                if let Some(text) = start.as_text() {
                    state.text(text, style);
                }
            }
            NodeEdge::End(end) => {
                if let Some(elem_data) = end.as_element() {
                    state.element_end(elem_data);
                }
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
    render_tree: crate::FlowTree,
    // a cursor into the `render_tree`.
    cursor: crate::FlowTree,
}

impl ParserState {
    fn new() -> Self {
        let render_tree = crate::FlowTree::new(Default::default());
        ParserState {
            render_tree: render_tree.clone(),
            cursor: render_tree,
        }
    }

    fn element_start(&mut self, _: &kuchiki::ElementData, style: Style) {
        let formatting_context = self.cursor.borrow().formatting_context();
        match style.display {
            Display::Block => {
                match formatting_context {
                    FormattingContext::Block => {
                        let new_node = Flow::with_content(None, style).into_ref();
                        self.cursor.append(new_node.clone());
                        self.cursor = new_node;
                    }
                    FormattingContext::Inline => {
                        // create an anonymous box around the inline content
                        unimplemented!()
                    }
                }
            }
            Display::Inline => {}
        }
    }

    fn element_end(&mut self, _: &kuchiki::ElementData) {
        // put the cursor back to the parent
        if let Some(parent) = self.cursor.parent() {
            self.cursor = parent;
        }
    }

    fn text(&mut self, text: &RefCell<String>, style: Style) {
        let new_node =
            Flow::with_content(Some(FlowType::Text(text.borrow().clone())), style).into_ref();
        self.cursor.append(new_node)
    }
}
