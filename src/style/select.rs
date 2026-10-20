use super::parse::{self, CssIdent, CssNamespace, CssString};
use crate::tree::{DomRef, ElementRef, TreeLinked};

use html5ever::{local_name, ns, LocalName, Namespace, QualName};
use selectors::{
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint, SELECTOR_WHITESPACE},
    bloom::BloomFilter,
    matching::{ElementSelectorFlags, MatchingContext},
};

impl selectors::Element for ElementRef {
    type Impl = parse::Selectors;

    #[inline]
    fn opaque(&self) -> selectors::OpaqueElement {
        let node: &crate::tree::NodeData = &self.as_node();
        selectors::OpaqueElement::new(node)
    }

    #[inline]
    fn is_html_slot_element(&self) -> bool {
        false
    }
    #[inline]
    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }
    #[inline]
    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    #[inline]
    fn parent_element(&self) -> Option<Self> {
        self.as_node().parent().and_then(DomRef::into_element_ref)
    }
    #[inline]
    fn prev_sibling_element(&self) -> Option<Self> {
        self.as_node()
            .preceding_siblings()
            .filter_map(DomRef::into_element_ref)
            .next()
    }
    #[inline]
    fn next_sibling_element(&self) -> Option<Self> {
        self.as_node()
            .following_siblings()
            .filter_map(DomRef::into_element_ref)
            .next()
    }
    #[inline]
    fn is_empty(&self) -> bool {
        self.as_node()
            .children()
            .all(|child| match child.html_data {
                crate::parser::ParsedData::Element(_) => false,
                crate::parser::ParsedData::Text(ref text) => text.borrow().is_empty(),
                _ => true,
            })
    }
    #[inline]
    fn is_root(&self) -> bool {
        match self.as_node().parent() {
            None => false,
            Some(parent) => {
                if let crate::parser::ParsedData::Document(_) = parent.html_data {
                    true
                } else {
                    false
                }
            }
        }
    }

    #[inline]
    fn is_html_element_in_html_document(&self) -> bool {
        // FIXME: Have a notion of HTML document v.s. XML document?
        self.name.ns == ns!(html)
    }

    #[inline]
    fn is_link(&self) -> bool {
        self.name.ns == ns!(html)
            && (self.name.local == local_name!("a")
                || self.name.local == local_name!("area")
                || self.name.local == local_name!("link"))
            && self.attributes.borrow().contains_key(&QualName::new(
                None,
                ns!(),
                local_name!("href"),
            ))
    }

    #[inline]
    fn has_id(&self, id: &CssIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.attributes
            .borrow()
            .get(&QualName::new(None, ns!(), local_name!("id")))
            .map_or(false, |id_attr| {
                case_sensitivity.eq(id.0.as_bytes(), id_attr.value.as_bytes())
            })
    }

    #[inline]
    fn has_class(&self, name: &CssIdent, case_sensitivity: CaseSensitivity) -> bool {
        let name = name.0.as_bytes();
        !name.is_empty()
            && match self.attributes
                    .borrow()
                    .get(&QualName::new(None, ns!(), local_name!("class")))
            { Some(class_attr) => {
                class_attr
                    .value
                    .split(SELECTOR_WHITESPACE)
                    .any(|class| case_sensitivity.eq(class.as_bytes(), name))
            } _ => {
                false
            }}
    }

    #[inline]
    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&CssNamespace>,
        local_name: &CssIdent,
        operation: &AttrSelectorOperation<&CssString>,
    ) -> bool {
        let attrs = self.attributes.borrow();
        match *ns {
            NamespaceConstraint::Any => attrs
                .iter()
                .any(|(name, attr)| name.local == local_name.0 && operation.eval_str(&attr.value)),
            NamespaceConstraint::Specific(ns_url) => attrs
                .get(&QualName::new(None, ns_url.0.clone(), local_name.0.clone()))
                .map_or(false, |attr| operation.eval_str(&attr.value)),
        }
    }

    fn match_pseudo_element(
        &self,
        pseudo: &parse::PseudoElement,
        _context: &mut MatchingContext<parse::Selectors>,
    ) -> bool {
        match *pseudo {}
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo: &parse::PseudoClass,
        _context: &mut MatchingContext<parse::Selectors>,
    ) -> bool {
        use parse::PseudoClass::*;
        match *pseudo {
            Active | Focus | Hover | Enabled | Disabled | Checked | Indeterminate | Visited => {
                false
            }
            AnyLink | Link => {
                self.name.ns == ns!(html)
                    && (self.name.local == local_name!("a")
                        || self.name.local == local_name!("area")
                        || self.name.local == local_name!("link"))
                    && self.attributes.borrow().contains_key(&QualName::new(
                        None,
                        ns!(),
                        local_name!("href"),
                    ))
            }
        }
    }
    fn is_pseudo_element(&self) -> bool {
        false
    }
    fn has_local_name(&self, local_name: &LocalName) -> bool {
        &self.name.local == local_name
    }
    fn has_namespace(&self, ns: &Namespace) -> bool {
        &self.name.ns == ns
    }
    fn is_same_type(&self, other: &Self) -> bool {
        self.name.ns == other.name.ns && self.name.local == other.name.local
    }
    fn first_element_child(&self) -> Option<Self> {
        self.as_node().children().find_map(DomRef::into_element_ref)
    }
    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}
    fn has_custom_state(&self, _name: &CssIdent) -> bool {
        false
    }
    fn imported_part(&self, _name: &CssIdent) -> Option<CssIdent> {
        None
    }
    fn is_part(&self, _name: &CssIdent) -> bool {
        false
    }
    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}
