pub fn transform_text(text: &mut String) {
    fn is_whitespace(ch: char) -> bool {
        match ch {
            ' ' | '\t' | '\n' => true,
            _ => false,
        }
    }

    let mut last_whitespace = true;
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
    })
}
