//! Splitting text into grapheme clusters with a script each.
//!
//! Font fallback and shaping both work on these clusters, so that a base character and its
//! combining marks always get the same font, and so that characters with the `Common` or
//! `Inherited` script (spaces, digits, punctuation) are matched with the font of the script run
//! they are in (UAX #24), honouring `Script_Extensions`.

use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Whether `ch` needs a glyph of its own: not line breaks and other controls, invisible
/// formatting (joiners, directional marks, variation selectors) or the soft hyphen, which are
/// shaped with whatever font the text around them has.
pub fn needs_font(ch: char) -> bool {
    !(ch.is_control()
        || ch == '\u{00AD}'
        || matches!(ch,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}'
            | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}' | '\u{FEFF}'))
}

/// A grapheme cluster of a text and the script it is itemized as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster<'a> {
    /// Byte offset of the cluster in the text.
    pub start: usize,
    pub text: &'a str,
    pub script: Script,
}

fn is_neutral(script: Script) -> bool {
    matches!(script, Script::Common | Script::Inherited | Script::Unknown)
}

/// The script of a single character that is `Common` or `Inherited` by its `Script` property but
/// has `Script_Extensions`, given the script of the run it is in (`run`, if any). Characters that
/// are used in several scripts take the script of the run if it is one of them, and otherwise
/// Han (the CJK punctuation `、` and `。` are shared by the East Asian scripts) or the first one.
fn extension_script(ch: char, run: Option<Script>) -> Option<Script> {
    let extension = ch.script_extension();
    if extension.len() == 0 {
        return None;
    }
    if let Some(run) = run
        && extension.contains_script(run)
    {
        return Some(run);
    }
    let mut scripts = extension.iter().filter(|script| !is_neutral(*script));
    let first = scripts.next()?;
    if extension.contains_script(Script::Han) {
        return Some(Script::Han);
    }
    if scripts.next().is_none() {
        // a single script: the character belongs to it
        return Some(first);
    }
    // used in several scripts, none of which is that of the run: no preference
    None
}

/// Whether `ch` is a fullwidth form (`！`, `Ａ`, `￥`): East Asian typography, which has glyphs in
/// the CJK fonts only, although the characters are Common (punctuation and symbols) or Latin
/// (letters) by their script. Without this, a fullwidth exclamation mark after a Latin letter
/// would be itemized as Latin and never get a CJK font.
fn is_fullwidth_form(ch: char) -> bool {
    matches!(ch, '\u{FF01}'..='\u{FF60}' | '\u{FFE0}'..='\u{FFE6}')
}

/// Splits `text` into grapheme clusters and itemizes their scripts: a cluster takes the script
/// of its first character that is not `Common` or `Inherited`; the remaining clusters take the
/// script of the preceding cluster (the following one at the start of the text).
pub fn clusters(text: &str) -> Vec<Cluster<'_>> {
    let mut clusters: Vec<Cluster<'_>> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| Cluster {
            start,
            text: grapheme,
            script: if grapheme.chars().next().is_some_and(is_fullwidth_form) {
                Script::Han
            } else {
                grapheme
                    .chars()
                    .map(|ch| ch.script())
                    .find(|script| !is_neutral(*script))
                    .unwrap_or(Script::Common)
            },
        })
        .collect();

    // Neutral clusters follow the run before them. Characters with Script_Extensions that
    // include the script of the run belong to it; ones that do not start a run of their own
    // if they unambiguously belong to a script.
    let mut run: Option<Script> = None;
    for cluster in &mut clusters {
        if is_neutral(cluster.script) {
            let first = cluster.text.chars().next().unwrap_or(' ');
            match extension_script(first, run) {
                Some(script) => cluster.script = script,
                None => {
                    if let Some(run) = run {
                        cluster.script = run;
                    }
                }
            }
        }
        if !is_neutral(cluster.script) {
            run = Some(cluster.script);
        }
    }
    // clusters at the start of the text take the script of the next run
    let mut next: Option<Script> = None;
    for cluster in clusters.iter_mut().rev() {
        if is_neutral(cluster.script) {
            if let Some(next) = next {
                cluster.script = next;
            }
        } else {
            next = Some(cluster.script);
        }
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scripts(text: &str) -> Vec<Script> {
        clusters(text).iter().map(|c| c.script).collect()
    }

    #[test]
    fn marks_stay_with_their_base() {
        let clusters = clusters("e\u{0301}x");
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].text, "e\u{0301}");
        assert_eq!(clusters[1].start, 3);
    }

    #[test]
    fn neutral_characters_take_the_script_of_the_preceding_run() {
        assert_eq!(
            scripts("a אב 1"),
            vec![
                Script::Latin,
                Script::Latin,
                Script::Hebrew,
                Script::Hebrew,
                Script::Hebrew,
                Script::Hebrew
            ]
        );
        // leading neutral characters take the script of the next run
        assert_eq!(scripts(" 1ب"), vec![Script::Arabic; 3]);
    }

    #[test]
    fn arabic_comma_is_arabic_in_arabic_text() {
        // U+060C is Common, with Arabic among its Script_Extensions
        assert_eq!(scripts("ب\u{060C}ب"), vec![Script::Arabic; 3]);
    }

    #[test]
    fn cjk_punctuation_is_han() {
        assert_eq!(scripts("a。")[1], Script::Han);
        assert_eq!(scripts("道、道"), vec![Script::Han; 3]);
        assert_eq!(scripts("か、か")[1], Script::Hiragana);
    }

    #[test]
    fn empty_and_neutral_text() {
        assert!(clusters("").is_empty());
        assert_eq!(scripts("1 2"), vec![Script::Common; 3]);
        // a stray mark at the start takes the script of what follows
        assert_eq!(scripts("\u{0301}a"), vec![Script::Latin, Script::Latin]);
    }

    #[test]
    fn characters_of_several_scripts_follow_the_run() {
        // the danda is used by many Indic scripts but not by Latin: it stays in the Latin run
        assert_eq!(scripts("a\u{0964}"), vec![Script::Latin; 2]);
        // ... and U+0640 (tatweel) belongs to the Arabic run it is in
        assert_eq!(scripts("ب\u{0640}ب"), vec![Script::Arabic; 3]);
        // while it has no script of its own at the start of a text: the next one is taken
        assert_eq!(scripts("\u{0640}ب")[0], Script::Arabic);
    }

    #[test]
    fn needs_font_excludes_controls_and_invisible_formatting() {
        for ch in ['\n', '\t', '\u{00AD}', '\u{200D}', '\u{FE0F}', '\u{202A}', '\u{E0100}', '\u{FEFF}'] {
            assert!(!needs_font(ch), "{ch:?}");
        }
        for ch in ['a', ' ', '漢', '\u{0301}', 'ب'] {
            assert!(needs_font(ch), "{ch:?}");
        }
    }

    #[test]
    fn clusters_know_their_byte_offsets() {
        let clusters = clusters("a漢e\u{0301}");
        assert_eq!(clusters.iter().map(|c| c.start).collect::<Vec<_>>(), vec![0, 1, 4]);
        assert_eq!(clusters[2].text, "e\u{0301}");
    }

    #[test]
    fn fullwidth_forms_are_east_asian_whatever_surrounds_them() {
        assert_eq!(scripts("A\u{FF01}")[1], Script::Han);
        assert_eq!(scripts("\u{FF21}\u{FF22}"), vec![Script::Han; 2]);
        assert_eq!(scripts("a\u{FFE5}b"), vec![Script::Latin, Script::Han, Script::Latin]);
        // halfwidth katakana keep their own script, and the neighbours of a fullwidth form
        // keep theirs
        assert_eq!(scripts("\u{FF71}")[0], Script::Katakana);
        assert_eq!(scripts("ب\u{FF0C}ب")[0], Script::Arabic);
    }
}
