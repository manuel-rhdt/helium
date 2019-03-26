pub fn transform_text(text: &mut String) {
    fn is_whitespace(ch: char) -> bool {
        match ch {
            ' ' | '\t' | '\n' => true,
            _ => false,
        }
    }

    let mut last_whitespace = false;
    text.retain(|ch| {
        let should_keep = !(last_whitespace && is_whitespace(ch));
        last_whitespace = is_whitespace(ch);
        should_keep
    })
}
