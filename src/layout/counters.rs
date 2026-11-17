//! Counters: the numbers of `counter-reset`, `counter-increment` and `counter-set`, and the text
//! that `counter()` and `counters()` make of them.
//!
//! https://www.w3.org/TR/css-lists-3/#auto-numbering

use crate::style::parse::CounterStyle;
use crate::style::properties::computed::ComputedValues;
use crate::style::properties::generated::{CounterList, Quotes};
use crate::style::properties::specified::Display;
use std::collections::HashMap;
use std::rc::Rc;

fn roman(mut n: usize) -> String {
    const NUMERALS: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    for &(value, numeral) in &NUMERALS {
        while n >= value {
            result.push_str(numeral);
            n -= value;
        }
    }
    result
}

/// `a`, `b`, … `z`, `aa`, `ab`, … — the `lower-alpha` counter style.
fn alphabetic(mut n: usize) -> String {
    const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
    let mut result = Vec::new();
    while n > 0 {
        n -= 1;
        result.push(LETTERS[n % 26]);
        n /= 26;
    }
    result.reverse();
    String::from_utf8(result).expect("the alphabet is valid UTF-8")
}

/// `α`, `β`, … `ω`, `αα`, `αβ`, … — the `lower-greek` counter style (24 letters, without the
/// final sigma).
fn greek(mut n: usize) -> String {
    const LETTERS: [char; 24] = [
        'α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ', 'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ', 'υ',
        'φ', 'χ', 'ψ', 'ω',
    ];
    let mut result = Vec::new();
    while n > 0 {
        n -= 1;
        result.push(LETTERS[n % 24]);
        n /= 24;
    }
    result.reverse();
    result.into_iter().collect()
}

/// The decimal digits of `value` in a script whose zero is `zero`.
fn digits(value: i64, zero: char) -> String {
    value
        .to_string()
        .chars()
        .map(|digit| match digit {
            '0'..='9' => char::from_u32(zero as u32 + digit as u32 - '0' as u32).unwrap_or(digit),
            other => other,
        })
        .collect()
}

/// The text of a counter value in the given style.
pub fn format_counter(value: i64, style: CounterStyle) -> String {
    // the numeral styles cannot write 0 and the negative numbers: they use decimal (CSS Counter
    // Styles 3, § 2.5 `fallback`)
    let positive = usize::try_from(value).ok().filter(|&value| value > 0);
    match (style, positive) {
        (CounterStyle::None, _) => String::new(),
        (CounterStyle::Disc, _) => "\u{2022}".to_string(),
        (CounterStyle::Circle, _) => "\u{25E6}".to_string(),
        (CounterStyle::Square, _) => "\u{25AA}".to_string(),
        // `decimal-leading-zero` pads to at least two digits: 01, 02, … 10
        (CounterStyle::DecimalLeadingZero, _) if (0..10).contains(&value) => format!("0{value}"),
        (CounterStyle::DecimalLeadingZero, _) if (-9..0).contains(&value) => format!("-0{}", -value),
        (CounterStyle::LowerRoman, Some(n)) if n < 4000 => roman(n),
        (CounterStyle::UpperRoman, Some(n)) if n < 4000 => roman(n).to_uppercase(),
        (CounterStyle::LowerAlpha, Some(n)) => alphabetic(n),
        (CounterStyle::UpperAlpha, Some(n)) => alphabetic(n).to_uppercase(),
        (CounterStyle::LowerGreek, Some(n)) => greek(n),
        (CounterStyle::ArabicIndic, _) => digits(value, '\u{660}'),
        (CounterStyle::Persian, _) => digits(value, '\u{6f0}'),
        _ => value.to_string(),
    }
}

/// One counter: its identity, which distinguishes counters that have the same name.
#[derive(Debug, Clone)]
struct Counter {
    id: usize,
    name: String,
    /// The parent of the element that created the counter. A `counter-reset` of a sibling
    /// replaces the counter, one of a child nests a new one inside of it.
    creator_parent: usize,
}

/// The counters in scope at the children of an element.
#[derive(Debug, Clone, Default)]
struct Frame {
    set: Rc<Vec<Counter>>,
    /// The counters of the last child that was left: they are in scope for the next sibling.
    last_child_set: Option<Rc<Vec<Counter>>>,
    /// The element or an ancestor has no box, so that counters do not change.
    inert: bool,
}

/// The state of the counters and quotation marks while the document is walked in tree order.
///
/// Call [`enter`](Self::enter) when an element is reached and [`leave`](Self::leave) after its
/// descendants.
/// https://www.w3.org/TR/css-lists-3/#counters-model
#[derive(Debug, Default)]
pub struct CounterState {
    frames: Vec<Frame>,
    values: HashMap<usize, i64>,
    next_id: usize,
    quote_depth: usize,
}

/// What the HTML specification adds to the style sheet of the user agent for lists
/// (https://html.spec.whatwg.org/multipage/rendering.html#lists): list items count in the
/// `list-item` counter, `<ol start>` and `<li value>` set it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListHints {
    /// `<li>`: the element is a list item (this renderer has no `display: list-item`).
    pub is_item: bool,
    /// `<ol>`, `<ul>` and `<menu>`: they start a new `list-item` counter.
    pub resets_list_item: bool,
    /// The number the list starts at, if `<ol start>` gives one.
    pub start: Option<i64>,
    /// The `value` of a `<li>`.
    pub value: Option<i64>,
}

impl CounterState {
    fn instantiate(&mut self, set: &mut Vec<Counter>, name: &str, value: i64, parent: usize) {
        // a counter of a sibling with the same name goes out of scope
        set.retain(|counter| !(counter.name == name && counter.creator_parent == parent));
        let id = self.next_id;
        self.next_id += 1;
        self.values.insert(id, value);
        set.push(Counter { id, name: name.to_owned(), creator_parent: parent });
    }

    /// The counter of that name that is innermost, if any.
    fn innermost(set: &[Counter], name: &str) -> Option<usize> {
        set.iter().rev().find(|counter| counter.name == name).map(|counter| counter.id)
    }

    /// Counts for an element that is reached: the counters it inherits, then `counter-reset`,
    /// `counter-increment` and `counter-set`. `parent` identifies the parent element, `style`
    /// is the style of the element.
    pub fn enter(&mut self, parent: usize, style: &ComputedValues, hints: ListHints) {
        let parent_frame = self.frames.last().cloned().unwrap_or_default();
        if parent_frame.inert || style.box_.display == Display::None {
            // an element without a box does not count, and neither do its descendants
            self.frames.push(Frame { inert: true, ..parent_frame });
            return;
        }
        let mut set: Vec<Counter> = (*parent_frame.set).clone();
        if let Some(sibling) = &parent_frame.last_child_set {
            for counter in sibling.iter() {
                if !set.iter().any(|existing| existing.id == counter.id) {
                    set.push(counter.clone());
                }
            }
        }

        let mentions = |list: &CounterList, name: &str| list.0.iter().any(|(entry, _)| entry == name);
        let is_item = hints.is_item;

        // `counter-reset`; a list starts a new `list-item` counter at one before its first item
        let mut resets = style.box_.counter_reset.0.clone();
        if hints.resets_list_item && !mentions(&style.box_.counter_reset, "list-item") {
            resets.push(("list-item".to_owned(), Some(hints.start.map_or(0, |start| (start - 1).clamp(i32::MIN as i64, i32::MAX as i64) as i32))));
        }
        for (name, value) in &resets {
            self.instantiate(&mut set, name, i64::from(value.unwrap_or(0)), parent);
        }

        // `counter-increment`; a list item counts in `list-item`
        let mut increments = style.box_.counter_increment.0.clone();
        if is_item && !mentions(&style.box_.counter_increment, "list-item") {
            increments.push(("list-item".to_owned(), Some(1)));
        }
        for (name, by) in &increments {
            let id = match Self::innermost(&set, name) {
                Some(id) => id,
                None => {
                    self.instantiate(&mut set, name, 0, parent);
                    set.last().expect("the counter was just added").id
                }
            };
            *self.values.entry(id).or_insert(0) += i64::from(by.unwrap_or(1));
        }

        // `counter-set`; `<li value>` sets the `list-item` counter
        let mut sets = style.box_.counter_set.0.clone();
        if let Some(value) = hints.value.filter(|_| is_item && !mentions(&style.box_.counter_set, "list-item")) {
            sets.push(("list-item".to_owned(), i32::try_from(value).ok()));
        }
        for (name, to) in &sets {
            let id = match Self::innermost(&set, name) {
                Some(id) => id,
                None => {
                    self.instantiate(&mut set, name, 0, parent);
                    set.last().expect("the counter was just added").id
                }
            };
            self.values.insert(id, i64::from(to.unwrap_or(0)));
        }

        self.frames.push(Frame { set: Rc::new(set), last_child_set: None, inert: false });
    }

    /// Leaves the element that was entered last: its counters stay in scope for its following
    /// siblings.
    pub fn leave(&mut self) {
        let Some(frame) = self.frames.pop() else { return };
        if frame.inert {
            return;
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.last_child_set = Some(frame.set);
        }
    }

    /// The value of `counter(name)` for the element that was entered last. A name without a
    /// counter in scope counts as 0.
    pub fn value(&self, name: &str) -> i64 {
        self.frames
            .last()
            .and_then(|frame| Self::innermost(&frame.set, name))
            .and_then(|id| self.values.get(&id))
            .copied()
            .unwrap_or(0)
    }

    /// The values of `counters(name, ...)`: the counters of that name in scope, outermost first.
    pub fn values(&self, name: &str) -> Vec<i64> {
        let Some(frame) = self.frames.last() else { return vec![0] };
        let values: Vec<i64> = frame
            .set
            .iter()
            .filter(|counter| counter.name == name)
            .map(|counter| self.values.get(&counter.id).copied().unwrap_or(0))
            .collect();
        if values.is_empty() { vec![0] } else { values }
    }

    /// The text of `open-quote` (with `mark`) or `no-open-quote` (without): the nesting of
    /// quotations gets one level deeper.
    pub fn open_quote(&mut self, quotes: &Quotes, lang: &str, mark: bool) -> String {
        let text = quotes.marks(self.quote_depth, lang).map(|(open, _)| open).filter(|_| mark);
        self.quote_depth += 1;
        text.unwrap_or_default()
    }

    /// The text of `close-quote` (with `mark`) or `no-close-quote`; the nesting gets one level
    /// shallower. Without an open quotation there is nothing to close and no mark.
    pub fn close_quote(&mut self, quotes: &Quotes, lang: &str, mark: bool) -> String {
        if self.quote_depth == 0 {
            return String::new();
        }
        self.quote_depth -= 1;
        quotes.marks(self.quote_depth, lang).map(|(_, close)| close).filter(|_| mark).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_styles() {
        assert_eq!(format_counter(7, CounterStyle::Decimal), "7");
        assert_eq!(format_counter(-3, CounterStyle::Decimal), "-3");
        assert_eq!(format_counter(7, CounterStyle::DecimalLeadingZero), "07");
        assert_eq!(format_counter(12, CounterStyle::DecimalLeadingZero), "12");
        assert_eq!(format_counter(14, CounterStyle::LowerRoman), "xiv");
        assert_eq!(format_counter(14, CounterStyle::UpperRoman), "XIV");
        assert_eq!(format_counter(0, CounterStyle::LowerRoman), "0");
        assert_eq!(format_counter(28, CounterStyle::LowerAlpha), "ab");
        assert_eq!(format_counter(2, CounterStyle::UpperAlpha), "B");
        assert_eq!(format_counter(2, CounterStyle::LowerGreek), "β");
        assert_eq!(format_counter(5, CounterStyle::None), "");
        assert_eq!(format_counter(12, CounterStyle::Persian), "۱۲");
    }
}
