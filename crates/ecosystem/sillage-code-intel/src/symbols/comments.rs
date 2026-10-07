//! Raw-text comment scanner for task-marker keywords (todo, fixme, hack).
//!
//! `syn` does not surface comments, so the scanner works directly on source
//! bytes with a deterministic line state machine: it skips string, char, and
//! block-comment contexts and matches the task keywords right after a
//! `//`/`/*` opener. Markers are later attached to the symbol whose source
//! range contains the comment (see `attach_comment_markers`).

use crate::markers::{CodeMarker, CodeMarkerKind};
use crate::{CodeIntelError, SymbolRecord};

/// A todo/fixme/hack comment found by the raw-text scanner, before it is
/// validated into a persisted [`CodeMarker`].
pub(crate) struct CommentMarker {
    pub(crate) kind: CodeMarkerKind,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
}

const MARKER_KEYWORDS: [(&str, CodeMarkerKind); 3] = [
    (concat!("TO", "DO"), CodeMarkerKind::Todo),
    (concat!("FIX", "ME"), CodeMarkerKind::Fixme),
    ("HACK", CodeMarkerKind::Hack),
];

/// A block comment that started on an earlier line and stays open.
struct BlockComment {
    start_line: usize,
    depth: usize,
    matched: Option<CodeMarkerKind>,
}

/// String literal state that may span line boundaries.
enum StringState {
    /// Plain `"…"` literal continued across a line via a trailing `\`.
    Plain,
    /// Raw `r#"…"#` literal; `hashes` is the `#` count of the opener.
    Raw { hashes: usize },
}

/// Scan `source` for todo/fixme/hack comments, in source order, with
/// one-based inclusive line ranges. String/char literals and block comments
/// are skipped; the keyword must directly follow the comment opener (after
/// an optional `!`/`*` doc prefix and whitespace) and be followed by `(`,
/// `[`, `:`, whitespace, or end of comment.
pub(crate) fn scan_comment_markers(source: &str) -> Vec<CommentMarker> {
    let mut markers = Vec::new();
    let mut string_state: Option<StringState> = None;
    let mut block_comment: Option<BlockComment> = None;

    for (index, line) in source.lines().enumerate() {
        let line_number = index + 1;
        let bytes = line.as_bytes();
        let mut pos = 0usize;

        // Continue a block comment opened on an earlier line.
        if let Some(mut block) = block_comment.take() {
            match advance_block_comment(line, &mut block) {
                Some(next) => {
                    if let Some(kind) = block.matched {
                        markers.push(CommentMarker {
                            kind,
                            start_line: block.start_line,
                            end_line: line_number,
                        });
                    }
                    pos = next;
                }
                None => {
                    block_comment = Some(block);
                    continue;
                }
            }
        }

        while pos < bytes.len() {
            if let Some(state) = string_state.take() {
                pos = match advance_string_literal(line, pos, &state) {
                    Advance::Closed { next } => next,
                    Advance::Continues => {
                        string_state = Some(state);
                        bytes.len()
                    }
                    Advance::ClosedAtLineEnd => bytes.len(),
                };
                continue;
            }
            match bytes[pos] {
                b'"' => {
                    pos = handle_plain_string(line, pos, &mut string_state);
                }
                b'\'' => match char_literal_end(line, pos) {
                    Some(end) => pos = end,
                    // A lifetime (`'a`): not a literal; keep scanning.
                    None => pos += 1,
                },
                b'r' | b'b' | b'c' => {
                    pos = handle_prefixed_literal(line, pos, &mut string_state);
                }
                b'/' => match bytes.get(pos + 1) {
                    Some(b'/') => {
                        let comment_text = line.get(pos + 2..).map_or("", |_| &line[pos + 2..]);
                        if let Some(kind) = marker_kind_after_prefix(comment_text) {
                            markers.push(CommentMarker {
                                kind,
                                start_line: line_number,
                                end_line: line_number,
                            });
                        }
                        // The rest of the line is comment text.
                        pos = bytes.len();
                    }
                    Some(b'*') => {
                        let comment_text = line.get(pos + 2..).map_or("", |_| &line[pos + 2..]);
                        let matched = marker_kind_after_prefix(comment_text);
                        match block_comment_end(line, pos + 2, matched, line_number) {
                            Some(open) => {
                                block_comment = Some(open);
                                pos = bytes.len();
                            }
                            None => {
                                if let Some(kind) = matched {
                                    markers.push(CommentMarker {
                                        kind,
                                        start_line: line_number,
                                        end_line: line_number,
                                    });
                                }
                                pos = block_close_position(line, pos + 2);
                            }
                        }
                    }
                    _ => pos += 1,
                },
                _ => pos += 1,
            }
        }
    }
    markers
}

/// What consuming a string-literal slice produced.
enum Advance {
    /// The literal closed; scanning resumes at `next`.
    Closed { next: usize },
    /// The literal continues on the next line.
    Continues,
    /// The literal ended at end of line (invalid Rust cannot reach this).
    ClosedAtLineEnd,
}

/// Advance past a string literal that started on an earlier line.
fn advance_string_literal(line: &str, pos: usize, state: &StringState) -> Advance {
    let bytes = line.as_bytes();
    let mut cursor = pos;
    while cursor < bytes.len() {
        match state {
            StringState::Plain => match bytes[cursor] {
                b'\\' => cursor += 2,
                b'"' => return Advance::Closed { next: cursor + 1 },
                _ => cursor += 1,
            },
            StringState::Raw { hashes } => {
                if bytes[cursor] == b'"'
                    && bytes
                        .get(cursor + 1..cursor + 1 + hashes)
                        .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
                {
                    return Advance::Closed {
                        next: cursor + 1 + hashes,
                    };
                }
                cursor += 1;
            }
        }
    }
    match state {
        StringState::Plain if line.ends_with('\\') => Advance::Continues,
        StringState::Plain => Advance::ClosedAtLineEnd,
        // Raw strings may span lines freely.
        StringState::Raw { .. } => Advance::Continues,
    }
}

/// Handle a plain `"…"` string opener at `pos`, updating the cross-line
/// string state and returning the next scan position.
fn handle_plain_string(line: &str, pos: usize, string_state: &mut Option<StringState>) -> usize {
    let bytes = line.as_bytes();
    let mut cursor = pos + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'"' => return cursor + 1,
            _ => cursor += 1,
        }
    }
    if line.ends_with('\\') {
        *string_state = Some(StringState::Plain);
    }
    bytes.len()
}

/// Handle `r`/`b`/`c` at `pos`: a raw string opener, a byte/char literal,
/// or an ordinary identifier character.
fn handle_prefixed_literal(
    line: &str,
    pos: usize,
    string_state: &mut Option<StringState>,
) -> usize {
    let bytes = line.as_bytes();
    // `r`/`b`/`c` inside an identifier is not a literal opener.
    if pos > 0 && is_identifier_byte(bytes[pos - 1]) {
        return pos + 1;
    }
    match bytes[pos] {
        b'r' => raw_string_from(line, pos + 1, string_state),
        b'b' | b'c' => match bytes.get(pos + 1) {
            Some(b'r') => raw_string_from(line, pos + 2, string_state),
            Some(b'"') => handle_plain_string(line, pos + 1, string_state),
            Some(b'\'') if bytes[pos] == b'b' => match char_literal_end(line, pos + 1) {
                Some(end) => end,
                None => pos + 1,
            },
            _ => pos + 1,
        },
        _ => pos + 1,
    }
}

/// Try to parse a raw string opener starting at `pos` (`"` with optional
/// `#` hash prefixes); returns the next scan position and updates the
/// cross-line state.
fn raw_string_from(line: &str, pos: usize, string_state: &mut Option<StringState>) -> usize {
    let bytes = line.as_bytes();
    let mut hashes = 0usize;
    while bytes.get(pos + hashes) == Some(&b'#') {
        hashes += 1;
    }
    if bytes.get(pos + hashes) != Some(&b'"') {
        // A raw identifier (`r#name`) or an identifier ending in `r`.
        return pos + hashes;
    }
    *string_state = Some(StringState::Raw { hashes });
    let content_start = pos + hashes + 1;
    let mut cursor = content_start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes
                .get(cursor + 1..cursor + 1 + hashes)
                .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
        {
            *string_state = None;
            return cursor + 1 + hashes;
        }
        cursor += 1;
    }
    // The raw string continues on the next line.
    bytes.len()
}

/// Advance past a char literal starting at `start` (the opening `'`),
/// returning the index just past the closing `'`, or `None` for a lifetime.
fn char_literal_end(line: &str, start: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut cursor = start + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'\'' => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

/// Advance a block comment that started on an earlier line. Returns the
/// next scan position when the block closed on this line, `None` when it
/// stays open.
fn advance_block_comment(line: &str, block: &mut BlockComment) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes.get(pos..pos + 2) == Some(b"/*") {
            block.depth += 1;
            pos += 2;
        } else if bytes.get(pos..pos + 2) == Some(b"*/") {
            block.depth -= 1;
            pos += 2;
            if block.depth == 0 {
                return Some(pos);
            }
        } else {
            pos += 1;
        }
    }
    None
}

/// Scan a block comment opening at `content_start` on `line`. Returns the
/// cross-line state when the block stays open, or `None` when it closes on
/// this line.
fn block_comment_end(
    line: &str,
    content_start: usize,
    matched: Option<CodeMarkerKind>,
    start_line: usize,
) -> Option<BlockComment> {
    let bytes = line.as_bytes();
    let mut depth = 1usize;
    let mut pos = content_start;
    while pos < bytes.len() {
        if bytes.get(pos..pos + 2) == Some(b"/*") {
            depth += 1;
            pos += 2;
        } else if bytes.get(pos..pos + 2) == Some(b"*/") {
            depth -= 1;
            pos += 2;
            if depth == 0 {
                return None;
            }
        } else {
            pos += 1;
        }
    }
    Some(BlockComment {
        start_line,
        depth,
        matched,
    })
}

/// Position just past a `*/` that closes a block comment whose content
/// starts at `content_start`, or end of line when the close is not found
/// (callers only invoke this after `block_comment_end` reported a close).
fn block_close_position(line: &str, content_start: usize) -> usize {
    let bytes = line.as_bytes();
    let mut pos = content_start;
    while pos + 1 < bytes.len() {
        if bytes.get(pos..pos + 2) == Some(b"*/") {
            return pos + 2;
        }
        pos += 1;
    }
    bytes.len()
}

/// The keyword kind directly after a `//`/`/*` opener (an optional `!` or
/// `*` doc prefix and whitespace are allowed before the keyword).
fn marker_kind_after_prefix(comment_text: &str) -> Option<CodeMarkerKind> {
    let mut text = comment_text;
    if let Some(rest) = text.strip_prefix('!').or_else(|| text.strip_prefix('*')) {
        text = rest;
    }
    let text = text.trim_start();
    let upper = text.to_ascii_uppercase();
    for (keyword, kind) in MARKER_KEYWORDS {
        let Some(rest) = upper.strip_prefix(keyword) else {
            continue;
        };
        let boundary = rest.chars().next();
        let boundary_ok = match boundary {
            None => true,
            Some('(') | Some('[') | Some(':') => true,
            Some(character) => character.is_whitespace(),
        };
        if boundary_ok {
            return Some(kind);
        }
    }
    None
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Attach each marker to the innermost symbol whose source range contains
/// the marker's start line. Returns the markers that are contained by no
/// symbol — the caller attaches those to the file's root module symbol.
pub(crate) fn attach_comment_markers(
    symbols: &mut [SymbolRecord],
    markers: Vec<CommentMarker>,
) -> Result<Vec<CommentMarker>, CodeIntelError> {
    let mut orphans = Vec::new();
    for marker in markers {
        let code_marker = CodeMarker::new(marker.kind, marker.start_line, marker.end_line)
            .map_err(|error| CodeIntelError::Integrity {
                context: "attach comment marker to symbol".to_string(),
                details: error.to_string(),
            })?;
        match innermost_symbol_index(symbols, marker.start_line) {
            Some(index) => symbols[index].markers.code_markers.push(code_marker),
            None => orphans.push(marker),
        }
    }
    Ok(orphans)
}

/// Index of the innermost symbol (smallest range, first on ties) whose
/// source range contains `line`.
fn innermost_symbol_index(symbols: &[SymbolRecord], line: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_size = usize::MAX;
    for (index, symbol) in symbols.iter().enumerate() {
        let range = &symbol.provenance.source_range;
        if range.start_line() <= line && line <= range.end_line() {
            let size = range.end_line() - range.start_line();
            if size < best_size {
                best_size = size;
                best = Some(index);
            }
        }
    }
    best
}

#[cfg(test)]
#[path = "comments_tests.rs"]
mod tests;
