//! Shared text utilities for port adapters.

/// Truncates `text` to at most `max_bytes` bytes without splitting a UTF-8
/// code point.
pub fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    let limit = max_bytes.min(text.len());
    &text[..text.floor_char_boundary(limit)]
}
