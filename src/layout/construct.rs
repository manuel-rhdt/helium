use super::block::BlockFlow;
use super::flow::{Flow, FlowRef};
use super::fragment::{Fragment, SpecificFragment};
use super::inline::InlineFlow;
use crate::parser::{ElementData, ParsedData};
use crate::style::parse::Styles;
use crate::style::properties::computed::{ComputedValues, Display, StyleBuilder};
use crate::traverse::Traversal;
use crate::tree::{DomRef, TreeLinked};

use html5ever::{local_name, namespace_url, ns, Attribute, QualName};

/// tracks state while traversing the html document and builds a flow tree.
#[derive(Debug, Default)]
pub struct FlowConstructor {
    styles: Styles,
    inside_style_tag: bool,
}

impl Traversal<DomRef> for FlowConstructor {
    fn preorder_traverse(&mut self, node: DomRef) {
        if let ParsedData::Element(ElementData {
            name, attributes, ..
        }) = &node.html_data
        {
            if name.ns == ns!(html) && name.local == local_name!("style") {
                for child in node.children() {
                    if let ParsedData::Text(css) = &child.html_data {
                        self.styles.parse_css(&css.borrow())
                    }
                }
            }
            if name.ns == ns!(html) && name.local == local_name!("link") {
                if let Some(Attribute { value, .. }) =
                    attributes
                        .borrow()
                        .get(&QualName::new(None, ns!(), local_name!("rel")))
                {
                    if value.as_ref() == "stylesheet" {
                        if let Some(Attribute { value, .. }) = attributes
                            .borrow()
                            .get(&QualName::new(None, ns!(), local_name!("href")))
                        {
                            self.styles.add_stylesheet(value.as_ref())
                        }
                    }
                }
            }
        }

        // TODO: Selector matching, Cascade
        if let Some(element_ref) = &node.clone().into_element_ref() {
            let parent_style = element_ref
                .as_node()
                .parent()
                .and_then(|parent| parent.style());
            let mut style_builder = StyleBuilder::new(parent_style.unwrap_or_default());
            for matched_rule in self
                .styles
                .rules_iter()
                .filter(|rule| rule.matches(element_ref))
            {
                let properties = &matched_rule.properties;
                for declaration in properties.declarations() {
                    style_builder.add_declaration(declaration.clone());
                }
            }
            element_ref.as_node().set_style(style_builder.build());
        } else {
            node.set_style(Default::default());
        }
    }

    fn postorder_traverse(&mut self, node: DomRef) {
        let style = node.style().expect("No style was assigned");
        let flow = match style.box_.display {
            Display::Inline => self.build_fragments_for_inline(node.clone(), style),
            Display::Block => self.build_block_flow(node.clone(), style),
        };
        node.set_contributed_flows(flow);
    }
}

impl FlowConstructor {
    pub fn new() -> Self {
        FlowConstructor::default()
    }

    fn build_block_flow(&self, node: DomRef, style: ComputedValues) -> Option<FlowRef> {
        let fragment = Fragment::default();
        let block_flow = BlockFlow::new(Some(fragment), style).into_ref();

        for child_flow in node.children().flat_map(|child| child.contributed_flows()) {
            block_flow.append(child_flow);
        }

        Some(block_flow)
    }

    fn build_fragments_for_inline(&self, node: DomRef, style: ComputedValues) -> Option<FlowRef> {
        let fragments = match &node.html_data {
            ParsedData::Text(string) => vec![Fragment {
                class: SpecificFragment::Text(string.borrow().clone()),
                ..Default::default()
            }],
            _ => vec![],
        };
        let inline_flow = InlineFlow::with_fragments(fragments, style).into_ref();

        for child_flow in node.children().flat_map(|child| child.contributed_flows()) {
            inline_flow.append(child_flow);
        }

        Some(inline_flow)
    }
}
