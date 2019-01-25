use kuchiki::iter::NodeEdge;
use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;

use crate::render_tree::{FormattingContext, NodeType, RenderNodeRef};
use crate::style::{Display, Style};

use std::cell::RefCell;

pub fn generate_render_tree(html: &str) -> crate::RenderNodeRef {
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
    render_tree: crate::RenderNodeRef,
    // a cursor into the `render_tree`.
    cursor: crate::RenderNodeRef,
}

impl ParserState {
    fn new() -> Self {
        let render_tree = crate::RenderNodeRef::new();
        ParserState {
            render_tree: render_tree.clone(),
            cursor: render_tree,
        }
    }

    fn element_start(&mut self, elem: &kuchiki::ElementData, style: Style) {
        match style.display {
            Display::Block => {
                match self.cursor.formatting_context() {
                    FormattingContext::Block => {
                        self.cursor = self
                            .cursor
                            .append_child(RenderNodeRef::with_content(None, style));
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

    fn element_end(&mut self, elem: &kuchiki::ElementData) {
        // put the cursor back to the parent
        if let Some(parent) = self.cursor.parent_ref() {
            self.cursor = parent;
        }
    }

    fn text(&mut self, text: &RefCell<String>, style: Style) {
        self.cursor.append_child(RenderNodeRef::with_content(
            Some(NodeType::Text(text.borrow().clone())),
            style,
        ));
    }

    fn replace_content(&self, content: Option<NodeType>) {
        let current = &self.cursor.node_type;
        *current.borrow_mut() = content;
    }
}
