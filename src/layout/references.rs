//! Cross-references of generated content: `target-counter()`, `target-counters()` and
//! `target-text()` (CSS GCPM 3, "Cross-references").
//!
//! What they read is known only once the document is laid out: the counters and the text of the
//! target are those of a place that may come after the reference (a table of contents), and the
//! page it is on is known after pagination. So a document that uses them is laid out again with
//! what the previous layout found, until the page numbers stop changing
//! ([`layout_resolved`](super::layout_resolved)).

use std::collections::HashMap;

/// What is known of an element with an `id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Target {
    /// The counters in scope at the element, by name, outermost first.
    pub counters: HashMap<String, Vec<i64>>,
    /// The text content of the element, without generated content and hidden elements.
    pub text: String,
    /// The value of the `page` counter on the page where the element starts, if it is known.
    pub page: Option<i64>,
}

/// The targets of a document by `id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct References(pub HashMap<String, Target>);

impl References {
    /// The id that a link refers to: `#id`, or `document.html#id` (only the fragment is followed,
    /// since a link to another document cannot be resolved).
    pub fn fragment(url: &str) -> Option<&str> {
        let (before, fragment) = url.trim().split_once('#')?;
        before.is_empty().then_some(fragment).filter(|fragment| !fragment.is_empty())
    }

    /// The values of `target-counter()` and `target-counters()`: the counter `name` at the
    /// target, outermost first. `None` if there is no such target or counter.
    pub fn counter_values(&self, id: &str, name: &str) -> Option<Vec<i64>> {
        let target = self.0.get(id)?;
        if name == "page" {
            return target.page.map(|page| vec![page]);
        }
        // a name without a counter in scope counts as 0, as it does for `counter()`
        Some(target.counters.get(name).cloned().unwrap_or_else(|| vec![0]))
    }

    pub fn text(&self, id: &str) -> Option<&str> {
        self.0.get(id).map(|target| target.text.as_str())
    }
}
