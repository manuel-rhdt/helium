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

#[cfg(test)]
mod test {
    use super::*;

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
