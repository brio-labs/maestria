pub(super) fn excerpt_segments(excerpt: &str, query: &str) -> (String, String, String) {
    let range = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .find_map(|term| {
            let term: String = term.chars().take(80).collect();
            find_casefolded_range(excerpt, &term)
        });
    let Some((start, end)) = range else {
        let end = excerpt
            .char_indices()
            .nth(240)
            .map_or(excerpt.len(), |(index, character)| {
                index + character.len_utf8()
            });
        let mut text = excerpt[..end].to_string();
        if end < excerpt.len() {
            text.push('…');
        }
        return (text, String::new(), String::new());
    };

    let context_start = excerpt[..start]
        .char_indices()
        .rev()
        .nth(95)
        .map_or(0, |(index, _)| index);
    let context_end = excerpt[end..]
        .char_indices()
        .nth(160)
        .map_or(excerpt.len(), |(index, character)| {
            end + index + character.len_utf8()
        });
    let mut before = excerpt[context_start..start].to_string();
    if context_start > 0 {
        before.insert(0, '…');
    }
    let matched = excerpt[start..end].to_string();
    let mut after = excerpt[end..context_end].to_string();
    if context_end < excerpt.len() {
        after.push('…');
    }
    (before, matched, after)
}

fn find_casefolded_range(input: &str, needle: &str) -> Option<(usize, usize)> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return None;
    }
    let folded: Vec<(char, usize, usize)> = input
        .char_indices()
        .flat_map(|(start, character)| {
            let end = start + character.len_utf8();
            character
                .to_lowercase()
                .map(move |lowered| (lowered, start, end))
        })
        .collect();
    let index = folded.windows(needle.len()).position(|window| {
        window
            .iter()
            .map(|(character, _, _)| *character)
            .eq(needle.iter().copied())
    })?;
    Some((folded[index].1, folded[index + needle.len() - 1].2))
}
