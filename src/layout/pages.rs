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
    /// The number (from 1) of the page in the page groups that it is part of, by the name of the
    /// group, for the names that `:nth(... of name)` selectors mention (see [`PageKeys`]). A page
    /// group is made of the pages of an element with a `page` name and a forced break; if there
    /// are groups of one name inside each other, it is the innermost one.
    pub groups: Vec<(Arc<str>, usize)>,
}

impl PageContext {
    pub fn is_left(&self) -> bool {
        (self.index % 2 == 1) != self.rtl
    }

    pub fn is_first(&self) -> bool {
        self.index == 0
    }

    /// The number of the page in the group with the name, if it is part of one.
    pub fn position_in(&self, group: &str) -> Option<usize> {
        self.groups.iter().find(|(name, _)| &**name == group).map(|&(_, position)| position)
    }
}

/// Which pages a page context can be told apart by.
///
/// The style of a page does not depend on every page number, but on the kinds that the selectors
/// of the style sheet tell apart: the first page, left and right pages, the names that they
/// select, and which of the `:nth()` selectors match (see [`normalize`]).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PageKeys {
    /// The page names that the `@page` rules select. Pages with another name are styled like
    /// pages without a name.
    pub names: HashSet<Arc<str>>,
    /// The `:nth()` selectors by the page group that they select in (`None`: the pages of the
    /// document), as `(a, b)`.
    pub nth: std::collections::BTreeMap<Option<Arc<str>>, Vec<(i32, i32)>>,
}

/// Which pages a context is told apart by: all that the selectors can see, or what the size and
/// the margins of a page can depend on. The kernel that chooses the pages knows only the kind of
/// the page (first, left or right) and whether a page starts a page group, so a rule for
/// `:nth()` that is not for the first page of a group (`:nth(1 of name)`) has no `size` and
/// `margin`, but works for everything that is decided after the pages are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Geometry,
    Full,
}

fn nth_matches_of(selectors: &[(i32, i32)], position: usize) -> Vec<bool> {
    selectors.iter().map(|&(a, b)| crate::style::parse::PageNth { a, b, of: None }.matches(position)).collect()
}

/// The positions that a list of selectors has to be told apart at: the smallest of every set
/// that the selectors do not tell apart.
fn scan_limit(selectors: &[(i32, i32)]) -> usize {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let mut period = 2usize;
    let mut offset = 0usize;
    for &(a, b) in selectors {
        let a = a.unsigned_abs() as usize;
        if a > 0 {
            period = (period / gcd(period, a)).saturating_mul(a).min(1_000);
        }
        offset = offset.max(b.unsigned_abs() as usize);
    }
    offset + 2 * period + 2
}

impl PageKeys {
    /// The representative position (from 1) of a position in the group `group` (`None`: the
    /// document), which the selectors for it do not tell apart from the position.
    fn representative(&self, group: Option<&Arc<str>>, position: usize) -> usize {
        let selectors = self.nth.get(&group.cloned()).map_or(&[][..], Vec::as_slice);
        let key = |position: usize| {
            let mut key = nth_matches_of(selectors, position);
            if group.is_none() {
                // the kinds of pages that `:first`, `:left` and `:right` tell apart
                key.push(position == 1);
                key.push(position % 2 == 0);
            }
            key
        };
        let wanted = key(position);
        (1..=position).find(|&earlier| key(earlier) == wanted).unwrap_or(position)
    }

    /// Every representative position of a group (or of the document).
    fn representatives(&self, group: Option<&Arc<str>>) -> Vec<usize> {
        let selectors = self.nth.get(&group.cloned()).map_or(&[][..], Vec::as_slice);
        let limit = if selectors.is_empty() && group.is_some() { 1 } else { scan_limit(selectors).max(3) };
        let mut found: Vec<usize> = vec![];
        for position in 1..=limit {
            let representative = self.representative(group, position);
            if !found.contains(&representative) {
                found.push(representative);
            }
        }
        found
    }
}

/// Whether the `:nth()` selectors of the sheet pick the page: in the document, or in the group.
fn nth_selects(nth: &crate::style::parse::PageNth, page: &PageContext) -> bool {
    match &nth.of {
        None => nth.matches(page.index + 1),
        Some(group) => page.position_in(group).is_some_and(|position| nth.matches(position)),
    }
}

/// Whether an `@page` rule with the selector applies to the page.
pub fn selector_matches(selector: &PageSelector, page: &PageContext) -> bool {
    selector_matches_in(selector, page, Scope::Full)
}

/// Whether an `@page` rule with the selector applies to the page, as far as the scope can see.
pub fn selector_matches_in(selector: &PageSelector, page: &PageContext, scope: Scope) -> bool {
    if let Some(name) = &selector.name {
        if page.name.as_ref() != Some(name) {
            return false;
        }
    }
    match &selector.pseudo {
        None => true,
        // CSS Paged Media 3, 4.2.2: `:first` is the first page of the document, whatever else
        // the selector says (`chapter:first` is the first page, if it is a `chapter` page)
        Some(PagePseudo::First) => page.is_first(),
        Some(PagePseudo::Left) => page.is_left(),
        Some(PagePseudo::Right) => !page.is_left(),
        // CSS GCPM 3, 3.1: the page number in the document, or in a page group
        Some(PagePseudo::Nth(nth)) => match scope {
            Scope::Full => nth_selects(nth, page),
            Scope::Geometry => nth.is_first_of_group() && nth_selects(nth, page),
        },
    }
}

/// The `@page` rules that apply to the page, from the least to the most specific; rules of the
/// same specificity are in the order of the style sheet.
pub fn matching_rules<'a>(styles: &'a Styles, page: &PageContext) -> Vec<&'a QualifiedRule> {
    matching_rules_in(styles, page, Scope::Full)
}

/// Like [`matching_rules`], for what the scope can see.
pub fn matching_rules_in<'a>(styles: &'a Styles, page: &PageContext, scope: Scope) -> Vec<&'a QualifiedRule> {
    let mut rules: Vec<&QualifiedRule> = styles
        .rules_iter()
        .filter(|rule| {
            rule.page_selector()
                .is_some_and(|selector| selector_matches_in(selector, page, scope))
        })
        .collect();
    rules.sort_by_key(|rule| rule.page_selector().map_or(0, PageSelector::specificity));
    rules
}

/// What the `@page` rules of the style sheet select: the page names, and the `:nth()` selectors.
pub fn selected_names(styles: &Styles) -> PageKeys {
    let mut keys = PageKeys::default();
    for selector in styles.rules_iter().filter_map(|rule| rule.page_selector()) {
        if let Some(name) = &selector.name {
            keys.names.insert(name.clone());
        }
        if let Some(PagePseudo::Nth(nth)) = &selector.pseudo {
            let entry = keys.nth.entry(nth.of.clone()).or_default();
            if !entry.contains(&(nth.a, nth.b)) {
                entry.push((nth.a, nth.b));
            }
        }
    }
    keys
}

/// One page for every combination that the rules can tell apart.
pub fn representative_pages(keys: &PageKeys, rtl: bool, scope: Scope) -> Vec<PageContext> {
    let indexes: Vec<usize> = match scope {
        Scope::Geometry => vec![0, 1, 2],
        Scope::Full => keys.representatives(None).into_iter().map(|position| position - 1).collect(),
    };
    // the groups that a page can be part of: not in it, or at a position that matters
    let mut groups: Vec<Vec<(Arc<str>, usize)>> = vec![vec![]];
    for group in keys.nth.keys().flatten() {
        let positions = match scope {
            Scope::Geometry => vec![1],
            Scope::Full => keys.representatives(Some(group)),
        };
        let mut next = groups.clone();
        for existing in &groups {
            for &position in &positions {
                let mut combined = existing.clone();
                combined.push((group.clone(), position));
                next.push(combined);
            }
        }
        groups = next;
    }
    let mut pages = vec![];
    let names = std::iter::once(None).chain(keys.names.iter().cloned().map(Some));
    for name in names {
        for &index in &indexes {
            for groups in &groups {
                pages.push(PageContext { index, name: name.clone(), rtl, groups: groups.clone() });
            }
        }
    }
    pages
}

/// Maps a page to the representative page that is styled the same.
pub fn normalize(page: &PageContext, keys: &PageKeys, scope: Scope) -> PageContext {
    let name = page.name.clone().filter(|name| keys.names.contains(name));
    let index = match scope {
        Scope::Geometry => match page.index {
            0 => 0,
            index if index % 2 == 1 => 1,
            _ => 2,
        },
        Scope::Full => keys.representative(None, page.index + 1) - 1,
    };
    let groups = page
        .groups
        .iter()
        .filter(|(group, _)| keys.nth.contains_key(&Some(group.clone())))
        .filter_map(|(group, position)| match scope {
            // only the first page of a group is known where the pages are chosen
            Scope::Geometry => (*position == 1).then(|| (group.clone(), 1)),
            Scope::Full => Some((group.clone(), keys.representative(Some(group), *position))),
        })
        .collect();
    PageContext { index, name, rtl: page.rtl, groups }
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
    pub keys: PageKeys,
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
            .get(&normalize(page, &self.keys, Scope::Geometry))
            .copied()
            .unwrap_or(plain)
    }

    pub fn edges(&self, page: &PageContext) -> EdgeValues<PixelLength> {
        self.pages
            .get(&normalize(page, &self.keys, Scope::Geometry))
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
    pub keys: PageKeys,
    pub pages: HashMap<PageContext, Arc<crate::style::properties::computed::ComputedValues>>,
}

impl PageStyles {
    pub fn style(
        &self,
        page: &PageContext,
    ) -> Option<&Arc<crate::style::properties::computed::ComputedValues>> {
        self.pages.get(&normalize(page, &self.keys, Scope::Full))
    }
}

/// The pages of the document, in order. Written by the paginator.
#[derive(Debug, Default, Clone)]
pub struct Pages(pub Vec<PageContext>);

#[cfg(test)]
mod tests {
    use super::*;

    fn page(index: usize, name: Option<&str>) -> PageContext {
        PageContext { index, name: name.map(Arc::from), rtl: false, groups: vec![] }
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

    fn in_groups(mut page: PageContext, groups: &[(&str, usize)]) -> PageContext {
        page.groups = groups.iter().map(|&(name, position)| (Arc::from(name), position)).collect();
        page
    }

    #[test]
    fn nth_selectors_are_parsed() {
        use crate::style::parse::PageNth;
        let nth = |css: &str| match selector(css).pseudo {
            Some(PagePseudo::Nth(nth)) => nth,
            other => panic!("{css}: {other:?}"),
        };
        assert_eq!(nth("@page :nth(3)"), PageNth { a: 0, b: 3, of: None });
        assert_eq!(nth("@page :nth(2n+1)"), PageNth { a: 2, b: 1, of: None });
        assert_eq!(nth("@page :nth(even)"), PageNth { a: 2, b: 0, of: None });
        assert_eq!(nth("@page :nth(-n + 3)"), PageNth { a: -1, b: 3, of: None });
        assert_eq!(nth("@page :nth(1 of chapter)"), PageNth { a: 0, b: 1, of: Some(Arc::from("chapter")) });
        assert_eq!(selector("@page body:nth(2n of chapter)").name.as_deref(), Some("body"));
        // not valid: no argument, junk after the name, another name than an identifier
        let mut styles = Styles::default();
        styles.parse_css("@page :nth() { } @page :nth(1 of) { } @page :nth(1 of a b) { } @page :nth(1 chapter) { }");
        assert_eq!(styles.rules_iter().filter(|rule| rule.page_selector().is_some()).count(), 1);
    }

    #[test]
    fn nth_matches_the_numbers_of_the_formula() {
        use crate::style::parse::PageNth;
        let nth = |a, b| PageNth { a, b, of: None };
        let numbers = |nth: PageNth| (1..=8).filter(|&position| nth.matches(position)).collect::<Vec<_>>();
        assert_eq!(numbers(nth(0, 3)), [3]);
        assert_eq!(numbers(nth(2, 0)), [2, 4, 6, 8]);
        assert_eq!(numbers(nth(2, 1)), [1, 3, 5, 7]);
        assert_eq!(numbers(nth(3, 2)), [2, 5, 8]);
        assert_eq!(numbers(nth(-1, 3)), [1, 2, 3]);
        assert_eq!(numbers(nth(1, -2)), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(numbers(nth(0, 0)), Vec::<usize>::new());
    }

    #[test]
    fn nth_selects_pages_of_the_document_and_of_a_group() {
        let in_document = selector("@page :nth(3)");
        assert!(selector_matches(&in_document, &page(2, None)));
        assert!(!selector_matches(&in_document, &page(3, None)));
        // the page of a group is counted in the group, not in the document
        let in_group = selector("@page :nth(2 of body)");
        assert!(selector_matches(&in_group, &in_groups(page(7, Some("other")), &[("body", 2)])));
        assert!(!selector_matches(&in_group, &in_groups(page(7, None), &[("body", 3)])));
        assert!(!selector_matches(&in_group, &page(1, None)));
        // a page can be in several groups
        assert!(selector_matches(&in_group, &in_groups(page(7, None), &[("body", 2), ("part", 5)])));
    }

    #[test]
    fn the_size_follows_nth_only_for_the_first_page_of_a_group() {
        let first = selector("@page :nth(1 of body)");
        let second = selector("@page :nth(2 of body)");
        let document = selector("@page :nth(3)");
        let start = in_groups(page(4, None), &[("body", 1)]);
        let later = in_groups(page(5, None), &[("body", 2)]);
        for (selector, page, geometry, full) in [
            (&first, &start, true, true),
            (&second, &later, false, true),
            (&document, &page(2, None), false, true),
        ] {
            assert_eq!(selector_matches_in(selector, page, Scope::Geometry), geometry);
            assert_eq!(selector_matches_in(selector, page, Scope::Full), full);
        }
    }

    #[test]
    fn pages_are_normalized_by_what_the_selectors_see() {
        let mut styles = Styles::default();
        styles.parse_css("@page :nth(3) { } @page :nth(2n of body) { } @page :nth(1 of body) { } @page chapter { }");
        let keys = selected_names(&styles);
        // the document: the pages that nothing tells apart are one kind (first, even, odd), and
        // the third page is a kind of its own
        let kind = |index| normalize(&page(index, None), &keys, Scope::Full).index;
        // (indexes count from 0: the third page is index 2, and the odd pages after it are index 4)
        assert_eq!((kind(0), kind(1), kind(2), kind(3), kind(4), kind(5), kind(6)), (0, 1, 2, 1, 4, 1, 4));
        // a group: the first page, the even pages and the other odd pages
        let position = |position| {
            normalize(&in_groups(page(9, None), &[("body", position)]), &keys, Scope::Full).groups[0].1
        };
        assert_eq!((position(1), position(2), position(3), position(4), position(5)), (1, 2, 3, 2, 3));
        // groups that no selector names are not kept, and a name that is not selected is dropped
        let other = normalize(&in_groups(page(1, Some("unknown")), &[("elsewhere", 2)]), &keys, Scope::Full);
        assert_eq!((other.name, other.groups), (None, vec![]));
        // the kernel knows only the first page of a group
        let geometry = |position| normalize(&in_groups(page(9, None), &[("body", position)]), &keys, Scope::Geometry);
        assert_eq!(geometry(1).groups, vec![(Arc::from("body"), 1)]);
        assert_eq!(geometry(2).groups, vec![]);
        assert_eq!(normalize(&page(6, None), &keys, Scope::Geometry).index, 2);
    }

    #[test]
    fn every_kind_of_page_has_a_representative() {
        let mut styles = Styles::default();
        styles.parse_css("@page :nth(3) { } @page :nth(2n of body) { } @page chapter { }");
        let keys = selected_names(&styles);
        for scope in [Scope::Full, Scope::Geometry] {
            let representatives: HashSet<PageContext> = representative_pages(&keys, false, scope).into_iter().collect();
            for index in 0..12 {
                for position in [None, Some(1), Some(2), Some(5)] {
                    for name in [None, Some("chapter"), Some("x")] {
                        let mut page = page(index, name);
                        page.groups = position.map(|position| (Arc::from("body"), position)).into_iter().collect();
                        let normalized = normalize(&page, &keys, scope);
                        assert!(representatives.contains(&normalized), "{scope:?} {page:?} -> {normalized:?}");
                    }
                }
            }
        }
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
        let keys = PageKeys { names: [Arc::from("chapter")].into(), ..Default::default() };
        let normalized = normalize(&page(7, Some("chapter")), &keys, Scope::Full);
        assert_eq!(normalized, page(1, Some("chapter")));
        // a name that no rule selects is the same as no name
        assert_eq!(normalize(&page(4, Some("other")), &keys, Scope::Full), page(2, None));
        assert!(representative_pages(&keys, false, Scope::Full).contains(&normalized));
    }
}
