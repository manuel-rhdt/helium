use super::flow::FlowRef;
use crate::traverse::Traversal;
use crate::tree::DomRef;

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug)]
pub struct FlowConstructor {}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, _node: DomRef) {
        // crate::style::style_for_element(node);
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        unimplemented!()
    }
}

impl FlowConstructor {
    pub fn new() -> Self {
        FlowConstructor {}
    }

    fn build_fragments_for_inline(&self) {}

    //     fn element_start(&mut self, _: &ElementData, style: Style) {
    //         let formatting_context = self.cursor.borrow().formatting_context();
    //         match style.display {
    //             Display::Block => {
    //                 match formatting_context {
    //                     FormattingContext::Block => {
    //                         let new_node = Flow::with_content(None, style).into_ref();
    //                         self.cursor.append(new_node.clone());
    //                         self.cursor = new_node;
    //                     }
    //                     FormattingContext::Inline => {
    //                         // create an anonymous box around the inline content
    //                         unimplemented!()
    //                     }
    //                 }
    //             }
    //             Display::Inline => {}
    //         }
    //     }

    //     fn element_end(&mut self, _: &ElementData) {
    //         // put the cursor back to the parent
    //         if let Some(parent) = self.cursor.parent() {
    //             self.cursor = parent;
    //         }
    //     }

    //     fn text(&mut self, text: &RefCell<String>, style: Style) {
    //         let new_node =
    //             Flow::with_content(Some(FlowType::Text(text.borrow().clone())), style).into_ref();
    //         self.cursor.append(new_node)
    //     }
}
