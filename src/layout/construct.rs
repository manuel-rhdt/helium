use super::block::BlockFlow;
use super::flow::FlowRef;
use super::fragment::{Fragment, SpecificFragment};
use super::inline::InlineFlow;
use crate::parser::ParsedData;
use crate::style::{style_for_element, Display, Style};
use crate::traverse::Traversal;
use crate::tree::{DomRef, TreeLinked};

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug, Default)]
pub struct FlowConstructor {}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, _node: DomRef) {
        // crate::style::style_for_element(node);
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        let style = style_for_element(node.clone());
        let flow = match style.display {
            Display::Inline => self.build_fragments_for_inline(node.clone(), style),
            Display::Block => self.build_block_flow(node.clone(), style),
        };
        node.set_contributed_flows(flow);
    }
}

impl FlowConstructor {
    pub fn new() -> Self {
        FlowConstructor {}
    }

    fn build_block_flow(&self, node: DomRef, style: Style) -> Option<FlowRef> {
        let fragment = Fragment::default();
        let block_flow = BlockFlow::new(Some(fragment), style).into_ref();

        for child_flow in node.children().flat_map(|child| child.contributed_flows()) {
            block_flow.append(child_flow);
        }

        Some(block_flow)
    }

    fn build_fragments_for_inline(&self, node: DomRef, style: Style) -> Option<FlowRef> {
        let fragment = match &node.html_data {
            ParsedData::Text(string) => Fragment {
                class: SpecificFragment::Text(string.borrow().clone()),
                ..Default::default()
            },
            _ => unimplemented!(),
        };
        Some(InlineFlow::with_fragments(vec![fragment], style).into_ref())
    }
}
