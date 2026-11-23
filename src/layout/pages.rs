//! The kinds of pages a style sheet can tell apart: first, left and right pages (`:first`,
//! `:left`, `:right`) and pages with a name (the `page` property).
//!
//! A page is printed on a left page if its index is odd: the first page is a right page.
//! The name of a page is the name of the first box on it. A change of the name forces a page
//! break. `:first` is the first page of the document, also in `name:first` (CSS Paged Media 3,
//! 4.2.2): there are no page groups.
//! https://www.w3.org/TR/css-page-3/#page-selectors

use super::components::EdgeValues;
use crate::style::parse::{PagePseudo, PageSelector, QualifiedRule, Styles};
use crate::style::properties::computed::PixelLength;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What decides which `@page` rules apply to a page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PageContext {
    /// The index of the page in the document, starting at 0.
    pub index: usize,
    pub name: Option<Arc<str>>,
    /// Whether the document is set from right to left. The pages of such a document are bound
    /// on the right, and the first page is a left page.
    pub rtl: bool,
}

impl PageContext {
    pub fn is_left(&self) -> bool {
        (self.index % 2 == 1) != self.rtl
    }

    pub fn is_first(&self) -> bool {
        self.index == 0
    }
}

/// Whether an `@page` rule with the selector applies to the page.
pub fn selector_matches(selector: &PageSelector, page: &PageContext) -> bool {
    if let Some(name) = &selector.name {
        if page.name.as_ref() != Some(name) {
            return false;
        }
    }
    match selector.pseudo {
        None => true,
        // CSS Paged Media 3, 4.2.2: `:first` is the first page of the document, whatever else
        // the selector says (`chapter:first` is the first page, if it is a `chapter` page)
        Some(PagePseudo::First) => page.is_first(),
        Some(PagePseudo::Left) => page.is_left(),
        Some(PagePseudo::Right) => !page.is_left(),
    }
}

/// The `@page` rules that apply to the page, from the least to the most specific; rules of the
/// same specificity are in the order of the style sheet.
pub fn matching_rules<'a>(styles: &'a Styles, page: &PageContext) -> Vec<&'a QualifiedRule> {
    let mut rules: Vec<&QualifiedRule> = styles
        .rules_iter()
        .filter(|rule| {
            rule.page_selector()
                .is_some_and(|selector| selector_matches(selector, page))
        })
        .collect();
    rules.sort_by_key(|rule| rule.page_selector().map_or(0, PageSelector::specificity));
    rules
}

/// The page names that the `@page` rules of the style sheet select. Pages with another name
/// are styled like pages without a name.
pub fn selected_names(styles: &Styles) -> HashSet<Arc<str>> {
    styles
        .rules_iter()
        .filter_map(|rule| rule.page_selector()?.name.clone())
        .collect()
}

/// One page for every combination that the rules can tell apart.
pub fn representative_pages(names: &HashSet<Arc<str>>, rtl: bool) -> Vec<PageContext> {
    let mut pages = vec![];
    let names = std::iter::once(None).chain(names.iter().cloned().map(Some));
    for name in names {
        for index in 0..3 {
            pages.push(PageContext { index, name: name.clone(), rtl });
        }
    }
    pages
}

/// Maps a page to the representative page that is styled the same.
pub fn normalize(page: &PageContext, names: &HashSet<Arc<str>>) -> PageContext {
    let name = page.name.clone().filter(|name| names.contains(name));
    PageContext {
        index: match page.index {
            0 => 0,
            index if index % 2 == 1 => 1,
            _ => 2,
        },
        name,
        rtl: page.rtl,
    }
}

/// The margins and sizes of every kind of page.
///
/// Every page is laid out for its own content area (see `taffy_layout::paginate`), so the margins
/// and the size of a page can be anything its style sheet says.
#[derive(Debug, Default, Clone)]
pub struct PageMargins {
    /// Whether the pages are bound on the right (the document is set from right to left).
    pub rtl: bool,
    /// The margins of the layout.
    pub base: EdgeValues<PixelLength>,
    pub names: HashSet<Arc<str>>,
    pub pages: HashMap<PageContext, EdgeValues<PixelLength>>,
    /// The size of every kind of page that has a `size` of its own.
    pub sizes: HashMap<PageContext, (PixelLength, PixelLength)>,
}

impl PageMargins {
    /// The width and the height of the page, which are the ones of `plain` (the plain page)
    /// unless the style of the page says otherwise.
    pub fn size(
        &self,
        page: &PageContext,
        plain: (PixelLength, PixelLength),
    ) -> (PixelLength, PixelLength) {
        self.sizes
            .get(&normalize(page, &self.names))
            .copied()
            .unwrap_or(plain)
    }

    pub fn edges(&self, page: &PageContext) -> EdgeValues<PixelLength> {
        self.pages
            .get(&normalize(page, &self.names))
            .copied()
            .unwrap_or(self.base)
    }

    /// The left and right margin of the page.
    pub fn for_page(&self, page: &PageContext) -> (PixelLength, PixelLength) {
        let edges = self.edges(page);
        (edges.left, edges.right)
    }
}

/// The style of every kind of page: the declarations of the `@page` rules that apply to it, of
/// which the background is painted over the whole page.
#[derive(Debug, Default, Clone)]
pub struct PageStyles {
    pub names: HashSet<Arc<str>>,
    pub pages: HashMap<PageContext, Arc<crate::style::properties::computed::ComputedValues>>,
}

impl PageStyles {
    pub fn style(
        &self,
        page: &PageContext,
    ) -> Option<&Arc<crate::style::properties::computed::ComputedValues>> {
        self.pages.get(&normalize(page, &self.names))
    }
}

/// The pages of the document, in order. Written by the paginator.
#[derive(Debug, Default, Clone)]
pub struct Pages(pub Vec<PageContext>);

#[cfg(test)]
mod tests {
    use super::*;

    fn page(index: usize, name: Option<&str>) -> PageContext {
        PageContext { index, name: name.map(Arc::from), rtl: false }
    }

    fn selector(css: &str) -> PageSelector {
        let mut styles = Styles::default();
        styles.parse_css(&format!("{css} {{ }}"));
        styles.rules_iter().last().unwrap().page_selector().unwrap().clone()
    }

    #[test]
    fn selectors_are_parsed() {
        assert_eq!(selector("@page"), PageSelector::any());
        assert_eq!(selector("@page :left"), PageSelector::pseudo(PagePseudo::Left));
        let named = selector("@page chapter");
        assert_eq!(named.name.as_deref(), Some("chapter"));
        assert_eq!(named.pseudo, None);
        let named = selector("@page chapter:first");
        assert_eq!(named.name.as_deref(), Some("chapter"));
        assert_eq!(named.pseudo, Some(PagePseudo::First));
    }

    #[test]
    fn invalid_selectors_are_dropped() {
        let mut styles = Styles::default();
        let rules = styles.rules_iter().count();
        styles.parse_css("@page :bogus { } @page chapter:first:left { } @page a b { }");
        assert_eq!(styles.rules_iter().count(), rules);
    }

    #[test]
    fn pseudo_classes_match_pages() {
        let first = selector("@page :first");
        let left = selector("@page :left");
        let right = selector("@page :right");
        assert!(selector_matches(&first, &page(0, None)));
        assert!(!selector_matches(&first, &page(2, None)));
        assert!(selector_matches(&right, &page(0, None)));
        assert!(selector_matches(&left, &page(1, None)));
        assert!(!selector_matches(&left, &page(2, None)));
        assert!(selector_matches(&right, &page(2, None)));
    }

    #[test]
    fn names_match_pages_with_that_name() {
        let named = selector("@page chapter");
        assert!(selector_matches(&named, &page(3, Some("chapter"))));
        assert!(!selector_matches(&named, &page(3, Some("other"))));
        assert!(!selector_matches(&named, &page(3, None)));
    }

    #[test]
    fn named_first_is_the_first_page_of_the_document() {
        // CSS Paged Media 3, 4.2.2: `:first` matches the first printed page of the document, and
        // a name only adds the condition that the page has it
        let first = selector("@page chapter:first");
        assert!(selector_matches(&first, &page(0, Some("chapter"))));
        assert!(!selector_matches(&first, &page(5, Some("chapter"))));
        assert!(!selector_matches(&first, &page(0, Some("other"))));
        assert!(!selector_matches(&first, &page(0, None)));
    }

    #[test]
    fn rules_are_ordered_by_specificity() {
        let mut styles = Styles::default();
        styles.parse_css(
            "@page chapter:first { margin-left: 1px } @page :right { margin-left: 2px } \
             @page chapter { margin-left: 3px } @page { margin-left: 4px }",
        );
        let order: Vec<_> = matching_rules(&styles, &page(0, Some("chapter")))
            .iter()
            .map(|rule| rule.page_selector().unwrap().specificity())
            .collect();
        // the first rule is the one of the user agent style sheet
        assert_eq!(order, vec![0, 0, 1, 100, 110]);
    }

    #[test]
    fn pages_are_normalized() {
        let names: HashSet<Arc<str>> = [Arc::from("chapter")].into();
        let normalized = normalize(&page(7, Some("chapter")), &names);
        assert_eq!(normalized, page(1, Some("chapter")));
        // a name that no rule selects is the same as no name
        assert_eq!(normalize(&page(4, Some("other")), &names), page(2, None));
        assert!(representative_pages(&names, false).contains(&normalized));
    }
}
