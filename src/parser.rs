use kuchiki::iter::NodeEdge;
use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;

use crate::flow::{Flow, FlowType, FormattingContext};
use crate::style::{Display, Style};

use std::cell::RefCell;

pub fn generate_render_tree(html: &str) -> crate::FlowRef {
    let parser = parse_html();

    // this is the dom tree
    let document = parser.one(html);

    let mut flow_constructor = FlowConstructor::new();

    document.traverse(&mut flow_constructor);

    println!("{:?}", flow_constructor.root);
    flow_constructor.root
}

pub trait Traversal<NodeRef> {
    type Pre;

    // called on top-down traversal
    fn preorder_traverse(&mut self, node: NodeRef) -> Self::Pre;

    // called on bottom-up traversal
    fn postorder_traverse(&mut self, node: NodeRef, pre_result: Self::Pre);
}

pub trait Traverse<T> {
    fn traverse(self, traversal: &mut T);
}

impl<T> Traverse<T> for kuchiki::NodeRef
where
    T: Traversal<kuchiki::NodeRef>,
{
    fn traverse(self, traversal: &mut T) {
        let pre_result = traversal.preorder_traverse(self.clone());
        self.children().for_each(|child| child.traverse(traversal));
        traversal.postorder_traverse(self, pre_result);
    }
}

// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
struct FlowConstructor {
    // the root node
    root: crate::FlowRef,
    // a cursor into the flow tree.
    cursor: crate::FlowRef,
}

impl Traversal<kuchiki::NodeRef> for FlowConstructor {
    type Pre = Style;

    fn preorder_traverse(&mut self, node: kuchiki::NodeRef) -> Style {
        crate::style::style_for_element(node)
    }

    fn postorder_traverse(&mut self, node: kuchiki::NodeRef, pre_result: Style) {
        unimplemented!()
    }
}

impl FlowConstructor {
    fn new() -> Self {
        let render_tree = crate::FlowRef::new(Default::default());
        FlowConstructor {
            root: render_tree.clone(),
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
