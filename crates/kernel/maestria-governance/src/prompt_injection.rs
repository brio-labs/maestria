/// Normalize a query string for prompt-injection matching.
///
/// Normalization preserves only lower-case alphanumeric tokens separated by
/// single spaces so punctuation cannot break marker matching. Single pass
/// into one pre-sized buffer.
fn normalized_prompt_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_separator = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push(' ');
            }
            pending_separator = false;
            normalized.push(character.to_ascii_lowercase());
        } else {
            // Whitespace and non-alphanumeric characters are separators.
            pending_separator = true;
        }
    }
    normalized
}

/// Detect canonical prompt-injection phrases in user text.
///
/// This is intentionally lightweight and deterministic: a best-effort
/// classifier used before retrieval or web evidence persistence.
pub fn contains_prompt_injection_risk(text: &str) -> bool {
    const PROMPT_INJECTION_MARKERS: &[&str] = &[
        "ignore previous instructions",
        "ignore all instructions",
        "reveal system prompt",
        "reveal secrets",
        "disable safety",
        "approve this action",
        "disregard prior instructions",
        "override previous instructions",
        "ignore prior commands",
        "skip previous directions",
        "do not follow instructions",
        "bypass all restrictions",
        "forget prior constraints",
    ];
    let normalized = normalized_prompt_text(text);
    PROMPT_INJECTION_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}
