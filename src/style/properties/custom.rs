//! Custom properties (`--name: value`) and the substitution of `var()`.
//! https://www.w3.org/TR/css-variables-1/
//!
//! A custom property is inherited and holds the tokens of its value. Its computed value is the
//! value with every `var()` in it substituted; that is why the cascade of an element first
//! computes the custom properties and only then parses the declarations that use `var()`.

use std::collections::{BTreeMap, HashMap, HashSet};

use cssparser::{ParseError, Parser, ToCss, Token};

/// The longest value, in bytes, that a substitution may produce. It stops the exponential growth
/// of values that use a property several times, each of which uses the one before it twice.
const MAX_VALUE_LEN: usize = 1 << 16;

/// How deep fallbacks may nest.
const MAX_DEPTH: usize = 32;

/// The computed custom properties of an element: the names (with the leading `--`) and the
/// values, with `var()` substituted and the surrounding white space removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomProperties(BTreeMap<String, String>);

impl CustomProperties {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// Whether `name` is the name of a custom property. Unlike the other properties they are case
/// sensitive.
pub fn is_custom_property(name: &str) -> bool {
    name.len() > 2 && name.starts_with("--")
}

/// Whether the tokens of `text` contain a `var()` function.
pub fn contains_var(text: &str) -> bool {
    fn scan(input: &mut Parser) -> bool {
        let mut found = false;
        while let Ok(token) = input.next().cloned() {
            match token {
                Token::Function(name) if name.eq_ignore_ascii_case("var") => found = true,
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    // the block must be read up to its end, also after a `var()` was found
                    found |= input
                        .parse_nested_block(|input| Ok::<_, ParseError<()>>(scan(input)))
                        .unwrap_or(false);
                }
                _ => {}
            }
        }
        found
    }
    // `var(` may be written in any case; only run the tokenizer if it may be there
    text.to_ascii_lowercase().contains("var(") && scan(&mut Parser::new(text))
}

/// Replaces every `var()` in `text` by the value of its custom property, or by its fallback.
/// `lookup` gives the value of a custom property. Returns `None` if a `var()` refers to a
/// property without value and has no fallback (the declaration is then invalid at computed-value
/// time).
pub fn substitute(text: &str, lookup: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    substitute_into(&mut Parser::new(text), lookup, &mut out, 0)?;
    Some(out)
}

fn substitute_into(
    input: &mut Parser,
    lookup: &mut dyn FnMut(&str) -> Option<String>,
    out: &mut String,
    depth: usize,
) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        if out.len() > MAX_VALUE_LEN {
            return None;
        }
        let (open, close) = match &token {
            Token::Function(name) if name.eq_ignore_ascii_case("var") => {
                let (name, fallback) = input
                    .parse_nested_block(|input| {
                        input.skip_whitespace();
                        let name = input.expect_ident()?.to_string();
                        input.skip_whitespace();
                        let fallback = if input.try_parse(|input| input.expect_comma()).is_ok() {
                            let start = input.position();
                            while input.next_including_whitespace_and_comments().is_ok() {}
                            Some(input.slice_from(start).to_owned())
                        } else {
                            input.expect_exhausted()?;
                            None
                        };
                        Ok::<_, ParseError<()>>((name, fallback))
                    })
                    .ok()?;
                if !is_custom_property(&name) {
                    return None;
                }
                match lookup(&name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        let fallback = fallback?;
                        substitute_into(&mut Parser::new(fallback.trim_start()), lookup, out, depth + 1)?;
                    }
                }
                continue;
            }
            Token::Function(name) => {
                cssparser::serialize_identifier(name, out).ok()?;
                ("(", ")")
            }
            Token::ParenthesisBlock => ("(", ")"),
            Token::SquareBracketBlock => ("[", "]"),
            Token::CurlyBracketBlock => ("{", "}"),
            token => {
                token.to_css(out).ok()?;
                continue;
            }
        };
        out.push_str(open);
        input
            .parse_nested_block(|input| {
                substitute_into(input, lookup, out, depth).ok_or_else(|| ParseError::<()>::custom(()))
            })
            .ok()?;
        out.push_str(close);
    }
    Some(())
}

/// The computed custom properties of an element.
///
/// `inherited` are the ones of the parent and `declared` are the `(name, value)` of the custom
/// properties that the cascade set, the winning declaration last. Properties that refer to each
/// other in a cycle, or to a property that does not exist, have no value.
pub fn compute<'a>(
    inherited: &CustomProperties,
    declared: impl Iterator<Item = (&'a str, &'a str)>,
) -> CustomProperties {
    let mut winners: HashMap<&str, &str> = HashMap::new();
    for (name, value) in declared {
        winners.insert(name, value);
    }
    let mut resolver = Resolver {
        inherited,
        winners,
        done: HashMap::new(),
        in_progress: Vec::new(),
        in_cycle: HashSet::new(),
    };
    let mut result = inherited.clone();
    let names: Vec<&str> = resolver.winners.keys().copied().collect();
    for name in names {
        match resolver.get(name) {
            Some(value) => result.0.insert(name.to_owned(), value),
            None => result.0.remove(name),
        };
    }
    result
}

struct Resolver<'a> {
    inherited: &'a CustomProperties,
    winners: HashMap<&'a str, &'a str>,
    done: HashMap<String, Option<String>>,
    /// The properties whose value is being computed, the one that needs the next one first.
    in_progress: Vec<String>,
    /// The properties that were found to take part in a cycle.
    in_cycle: HashSet<String>,
}

impl Resolver<'_> {
    fn get(&mut self, name: &str) -> Option<String> {
        if let Some(value) = self.done.get(name) {
            return value.clone();
        }
        let Some(&raw) = self.winners.get(name) else {
            // not set on this element
            return self.inherited.get(name).map(str::to_owned);
        };
        if let Some(start) = self.in_progress.iter().position(|other| other == name) {
            // every property from the first use of `name` on depends on itself
            self.in_cycle.extend(self.in_progress[start..].iter().cloned());
            return None;
        }
        self.in_progress.push(name.to_owned());
        let value = match raw.trim().to_ascii_lowercase().as_str() {
            "inherit" | "unset" => self.inherited.get(name).map(str::to_owned),
            "initial" => None,
            _ => substitute(raw, &mut |other| self.get(other)).map(|value| value.trim().to_owned()),
        };
        self.in_progress.pop();
        let value = if self.in_cycle.contains(name) { None } else { value };
        self.done.insert(name.to_owned(), value.clone());
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(declared: &[(&str, &str)]) -> CustomProperties {
        compute(&CustomProperties::default(), declared.iter().copied())
    }

    fn sub(text: &str, props: &CustomProperties) -> Option<String> {
        substitute(text, &mut |name| props.get(name).map(str::to_owned))
    }

    #[test]
    fn finds_var_functions() {
        assert!(contains_var("var(--a)"));
        assert!(contains_var("calc(1px + VAR(--a))"));
        assert!(contains_var("1px (var(--a))"));
        assert!(!contains_var("1px solid red"));
        assert!(!contains_var("\"var(--a)\""));
    }

    #[test]
    fn substitutes_values_and_fallbacks() {
        let p = props(&[("--a", " 10px "), ("--b", "var(--a) solid")]);
        assert_eq!(p.get("--a"), Some("10px"));
        assert_eq!(p.get("--b"), Some("10px solid"));
        assert_eq!(sub("var(--b) red", &p).as_deref(), Some("10px solid red"));
        assert_eq!(sub("var(--missing, 3px)", &p).as_deref(), Some("3px"));
        assert_eq!(sub("var(--missing, 1px, 2px)", &p).as_deref(), Some("1px, 2px"));
        assert_eq!(sub("var(--missing, var(--a))", &p).as_deref(), Some("10px"));
        assert_eq!(sub("var(--missing)", &p), None);
        assert_eq!(sub("calc(var(--a) * 2)", &p).as_deref(), Some("calc(10px * 2)"));
    }

    #[test]
    fn cycles_are_invalid() {
        let p = props(&[("--a", "var(--b)"), ("--b", "var(--a, 1)"), ("--c", "var(--c)")]);
        assert_eq!(p.get("--a"), None);
        assert_eq!(p.get("--b"), None);
        assert_eq!(p.get("--c"), None);
    }

    #[test]
    fn keywords_and_inheritance() {
        let parent = props(&[("--a", "1"), ("--b", "2")]);
        let child = compute(&parent, [("--a", "inherit"), ("--b", "initial"), ("--c", "var(--a)")].into_iter());
        assert_eq!(child.get("--a"), Some("1"));
        assert_eq!(child.get("--b"), None);
        assert_eq!(child.get("--c"), Some("1"));
    }

    #[test]
    fn exponential_growth_is_stopped() {
        let mut declared = vec![("--a0".to_owned(), "x".to_owned())];
        for i in 1..30 {
            declared.push((format!("--a{i}"), format!("var(--a{}) var(--a{})", i - 1, i - 1)));
        }
        let p = compute(
            &CustomProperties::default(),
            declared.iter().map(|(name, value)| (name.as_str(), value.as_str())),
        );
        assert_eq!(p.get("--a29"), None);
        assert_eq!(p.get("--a3").map(str::len), Some(15));
    }
}
