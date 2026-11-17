//! Counters: the numbers of `counter-reset`, `counter-increment` and `counter-set`, and the text
//! that `counter()` and `counters()` make of them.
//!
//! https://www.w3.org/TR/css-lists-3/#auto-numbering

use crate::style::parse::CounterStyle;
use crate::style::properties::computed::ComputedValues;
use crate::style::properties::generated::{CounterEntry, Quotes};
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

/// The starting value of a reversed counter that is reset without one: the counter goes down to
/// a value that makes the last of the increments in its scope come out at its own step. `steps`
/// holds, in tree order, what the elements and pseudo-elements of the scope do to the counter:
/// the increment and the value that `counter-set` gives it, if either.
/// https://www.w3.org/TR/css-lists-3/#instantiating-counters
pub fn reversed_initial(steps: impl IntoIterator<Item = (Option<i64>, Option<i64>)>) -> i64 {
    let mut num = 0;
    let mut last_non_zero_negated = 0;
    for (increment, set) in steps {
        let negated = -increment.unwrap_or(0);
        if negated != 0 {
            last_non_zero_negated = negated;
        }
        if let Some(value) = set {
            num += value;
            break;
        }
        num += negated;
    }
    num + last_non_zero_negated
}

/// One counter: its identity, which distinguishes counters that have the same name.
#[derive(Debug, Clone)]
struct Counter {
    id: usize,
    name: String,
    /// The parent of the element that created the counter. A `counter-reset` of a sibling
    /// replaces the counter, one of a child nests a new one inside of it.
    creator_parent: usize,
    /// Made with `reversed()`: it counts down.
    reversed: bool,
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
    /// `<ol>`, `<ul>` and `<menu>`: they start a new `list-item` counter.
    pub resets_list_item: bool,
    /// The number the list starts at, if `<ol start>` gives one.
    pub start: Option<i64>,
    /// `<ol reversed>`: the items are numbered downwards.
    pub reversed: bool,
    /// The `value` of a `<li>`.
    pub value: Option<i64>,
}

impl CounterState {
    /// Instantiates a counter on the element whose parent is `parent`: a counter of the same
    /// name that the element or one of its previous siblings made goes out of scope; one of an
    /// ancestor stays, and the new counter nests inside of it.
    fn instantiate(&mut self, set: &mut Vec<Counter>, name: &str, value: i64, reversed: bool, parent: usize) {
        if let Some(position) = set.iter().rposition(|counter| counter.name == name) {
            if set[position].creator_parent == parent {
                set.remove(position);
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.values.insert(id, value);
        set.push(Counter { id, name: name.to_owned(), creator_parent: parent, reversed });
    }

    /// The counter of that name that is innermost, if any.
    fn innermost(set: &[Counter], name: &str) -> Option<usize> {
        set.iter().rposition(|counter| counter.name == name)
    }

    /// The index of the innermost counter of that name in `set`, instantiating a counter that
    /// starts at 0 if there is none.
    fn innermost_or_new(&mut self, set: &mut Vec<Counter>, name: &str, parent: usize) -> usize {
        match Self::innermost(set, name) {
            Some(index) => index,
            None => {
                self.instantiate(set, name, 0, false, parent);
                set.len() - 1
            }
        }
    }

    /// Counts for an element that is reached: the counters it inherits, then `counter-reset`,
    /// `counter-increment` and `counter-set`. `parent` identifies the parent element and
    /// `style` is the style of the element. `initials` holds the starting values of the
    /// reversed counters that the element resets without one (see [`reversed_initial`]).
    pub fn enter(
        &mut self,
        parent: usize,
        style: &ComputedValues,
        hints: ListHints,
        initials: &HashMap<String, i64>,
    ) {
        let parent_frame = self.frames.last().cloned().unwrap_or_default();
        if parent_frame.inert || style.box_.display == Display::None {
            // an element without a box does not count, and neither do its descendants
            self.frames.push(Frame { inert: true, ..parent_frame });
            return;
        }
        // the counters of the previous sibling (which has the ones of the parent as well), or
        // else the ones of the parent: https://www.w3.org/TR/css-lists-3/#inheriting-counters
        let mut set: Vec<Counter> = match &parent_frame.last_child_set {
            Some(sibling) => (**sibling).clone(),
            None => (*parent_frame.set).clone(),
        };

        let is_item = style.box_.display == Display::ListItem;
        let clamp = |value: i64| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32;

        // `counter-reset`; a list starts a new `list-item` counter at one before its first item
        // (a reversed one at the number of its items, or after its `start`)
        let mut resets = style.box_.counter_reset.0.clone();
        if hints.resets_list_item && style.box_.counter_reset.get("list-item").is_none() {
            resets.push(CounterEntry {
                name: "list-item".to_owned(),
                value: match (hints.reversed, hints.start) {
                    (true, start) => start.map(|start| clamp(start + 1)),
                    (false, start) => Some(start.map_or(0, |start| clamp(start - 1))),
                },
                reversed: hints.reversed,
            });
        }
        for entry in &resets {
            let value = entry.value.map(i64::from).or_else(|| initials.get(&entry.name).copied()).unwrap_or(0);
            self.instantiate(&mut set, &entry.name, value, entry.reversed, parent);
        }

        // `counter-increment`; a list item counts in `list-item`, down if the counter is
        // reversed
        // (the flag marks the increment that the list item makes by itself)
        let mut increments: Vec<(CounterEntry, bool)> =
            style.box_.counter_increment.0.iter().map(|entry| (entry.clone(), false)).collect();
        if is_item && style.box_.counter_increment.get("list-item").is_none() {
            increments.push((CounterEntry { name: "list-item".to_owned(), value: None, reversed: false }, true));
        }
        for (entry, implicit) in &increments {
            let index = self.innermost_or_new(&mut set, &entry.name, parent);
            let step = match entry.value {
                Some(value) => i64::from(value),
                None if *implicit && set[index].reversed => -1,
                None => 1,
            };
            *self.values.entry(set[index].id).or_insert(0) += step;
        }

        // `counter-set`; `<li value>` sets the `list-item` counter
        let mut sets = style.box_.counter_set.0.clone();
        if let Some(value) = hints.value.filter(|_| is_item && style.box_.counter_set.get("list-item").is_none()) {
            sets.push(CounterEntry { name: "list-item".to_owned(), value: i32::try_from(value).ok(), reversed: false });
        }
        for entry in &sets {
            let index = self.innermost_or_new(&mut set, &entry.name, parent);
            self.values.insert(set[index].id, i64::from(entry.value.unwrap_or(0)));
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
            .and_then(|frame| Self::innermost(&frame.set, name).map(|index| frame.set[index].id))
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
