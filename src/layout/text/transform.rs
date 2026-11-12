//! The `text-transform` property: https://www.w3.org/TR/css-text-3/#text-transform-property

use crate::style::properties::computed::Language;
use crate::style::properties::specified::TextTransform;

/// Whether the language distinguishes the dotted and the dotless `i` (Turkish and Azeri).
fn is_dotted_i_language(lang: &Language) -> bool {
    matches!(lang.primary(), Some("tr" | "az"))
}

fn upper(ch: char, turkic: bool) -> String {
    if turkic && ch == 'i' {
        "İ".to_string()
    } else {
        ch.to_uppercase().collect()
    }
}

fn lower(text: &str, turkic: bool) -> String {
    if !turkic {
        return text.to_lowercase();
    }
    text.chars()
        .map(|ch| match ch {
            'I' => "ı".to_string(),
            'İ' => "i".to_string(),
            ch => ch.to_lowercase().collect(),
        })
        .collect()
}

/// The titlecase mapping of a character: the first letter of a word.
fn title(ch: char, turkic: bool) -> String {
    match ch {
        // the digraphs have a titlecase form of their own
        'ǆ' | 'ǅ' | 'Ǆ' => "ǅ".to_string(),
        'ǉ' | 'ǈ' | 'Ǉ' => "ǈ".to_string(),
        'ǌ' | 'ǋ' | 'Ǌ' => "ǋ".to_string(),
        'ǳ' | 'ǲ' | 'Ǳ' => "ǲ".to_string(),
        _ => {
            // `ß` gives `SS` and `ﬁ` gives `FI`: only the first letter stays upper case
            let uppercase = upper(ch, turkic);
            let mut mapped = uppercase.chars();
            let mut result: String = mapped.next().into_iter().collect();
            result.extend(mapped.flat_map(char::to_lowercase));
            result
        }
    }
}

fn is_word_character(ch: char) -> bool {
    ch.is_alphanumeric()
}

/// Applies `transform` to `text`. For `capitalize`, `in_word` says whether the text continues a
/// word that began in the text before it (in a preceding inline element, for example), and is
/// updated for the text that follows.
pub fn apply(
    text: &str,
    transform: TextTransform,
    lang: &Language,
    in_word: &mut bool,
) -> Option<String> {
    let turkic = is_dotted_i_language(lang);
    match transform {
        TextTransform::None => {
            *in_word = text.chars().next_back().is_some_and(is_word_character);
            None
        }
        TextTransform::Uppercase => {
            *in_word = text.chars().next_back().is_some_and(is_word_character);
            Some(text.chars().map(|ch| upper(ch, turkic)).collect())
        }
        TextTransform::Lowercase => {
            *in_word = text.chars().next_back().is_some_and(is_word_character);
            Some(lower(text, turkic))
        }
        TextTransform::Capitalize => {
            let mut result = String::with_capacity(text.len());
            let mut chars = text.chars().peekable();
            while let Some(ch) = chars.next() {
                if is_word_character(ch) {
                    if *in_word {
                        result.push(ch);
                    } else {
                        result.push_str(&title(ch, turkic));
                    }
                    *in_word = true;
                } else if matches!(ch, '\'' | '’') && *in_word && chars.peek().is_some_and(|&next| is_word_character(next)) {
                    // an apostrophe inside of a word (`don't`) does not end it
                    result.push(ch);
                } else {
                    result.push(ch);
                    *in_word = false;
                }
            }
            Some(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, transform: TextTransform, lang: &str) -> String {
        apply(text, transform, &Language::from_tag(lang), &mut false)
            .unwrap_or_else(|| text.to_string())
    }

    #[test]
    fn uppercase_and_lowercase() {
        assert_eq!(run("Grüße, ß", TextTransform::Uppercase, ""), "GRÜSSE, SS");
        assert_eq!(run("ABC Δ", TextTransform::Lowercase, ""), "abc δ");
        assert_eq!(run("abc", TextTransform::None, ""), "abc");
    }

    #[test]
    fn lowercase_uses_the_final_sigma() {
        assert_eq!(run("ΟΔΟΣ", TextTransform::Lowercase, ""), "οδος");
    }

    #[test]
    fn capitalize_words() {
        assert_eq!(run("hello wORLD, it's a ǆ-test", TextTransform::Capitalize, ""), "Hello WORLD, It's A ǅ-Test");
        assert_eq!(run("ßa ﬁsh", TextTransform::Capitalize, ""), "Ssa Fish");
        assert_eq!(run("42nd street", TextTransform::Capitalize, ""), "42nd Street");
    }

    #[test]
    fn capitalize_continues_a_word_across_text_nodes() {
        let mut in_word = false;
        let lang = Language::default();
        let a = apply("f", TextTransform::Capitalize, &lang, &mut in_word).unwrap();
        let b = apply("oo bar", TextTransform::Capitalize, &lang, &mut in_word).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("F", "oo Bar"));
    }

    #[test]
    fn turkish_has_a_dotted_capital_i() {
        assert_eq!(run("iI", TextTransform::Uppercase, "tr"), "İI");
        assert_eq!(run("Iİ", TextTransform::Lowercase, "tr"), "ıi");
        assert_eq!(run("istanbul", TextTransform::Capitalize, "tr-TR"), "İstanbul");
        assert_eq!(run("iI", TextTransform::Uppercase, "en"), "II");
    }
}
