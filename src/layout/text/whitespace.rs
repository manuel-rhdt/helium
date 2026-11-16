fn is_whitespace(ch: char) -> bool {
    match ch {
        ' ' | '\t' | '\n' => true,
        _ => false,
    }
}
pub fn transform_text(text: &mut String, after_whitespace: bool) {
    let mut last_whitespace = after_whitespace;
    for i in 0..text.len() {
        if !text.is_char_boundary(i) {
            continue;
        }
        let ch = text.as_bytes()[i] as char;
        if is_whitespace(ch) {
            text.replace_range(i..=i, " ");
        }
    }

    text.retain(|ch| {
        let should_keep = !(last_whitespace && is_whitespace(ch));
        last_whitespace = is_whitespace(ch);
        should_keep
    });
}

/// `transform_text` for `white-space: pre-line`: spaces and tabs are collapsed, and white space
/// at the start and the end of a line is removed (a space at the end of the text stays, like for
/// `transform_text`), but the new lines are kept.
/// https://www.w3.org/TR/css-text-3/#white-space-phase-1
pub fn collapse_keeping_newlines(text: &mut String, after_whitespace: bool) {
    let mut collapsed = String::with_capacity(text.len());
    // a space is written when a character follows it (and it is not at the start of a line)
    let mut pending_space = false;
    let mut at_line_start = after_whitespace;
    for ch in text.chars() {
        match ch {
            '\n' => {
                pending_space = false;
                collapsed.push('\n');
                at_line_start = true;
            }
            ' ' | '\t' | '\r' | '\u{c}' => pending_space |= !at_line_start,
            ch => {
                if pending_space {
                    collapsed.push(' ');
                    pending_space = false;
                }
                collapsed.push(ch);
                at_line_start = false;
            }
        }
    }
    if pending_space {
        collapsed.push(' ');
    }
    *text = collapsed;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn pre_line_keeps_the_new_lines_and_collapses_the_rest() {
        let mut text = String::from("  a 	 b  
   c d

 e ");
        collapse_keeping_newlines(&mut text, true);
        // the spaces around the new lines go; a space at the end of the text stays
        assert_eq!(text, "a b
c d

e ");
        let mut text = String::from(" x");
        collapse_keeping_newlines(&mut text, false);
        assert_eq!(text, " x");
    }

    #[quickcheck]
    fn text_transform_removes_consecutive_whitespace(mut string: String) -> bool {
        transform_text(&mut string, true);
        let mut last_character_was_whitespace = true;
        for ch in string.chars() {
            if is_whitespace(ch) && last_character_was_whitespace {
                return false;
            }
            last_character_was_whitespace = is_whitespace(ch);
        }
        true
    }
}
