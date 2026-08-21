#![forbid(unsafe_code)]

use crate::chunking::{extension_is, paragraph_chunks, ranges_from_starts};
use crate::text_parser;

fn markdown_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    let heading_lines = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| is_markdown_heading(line).then_some(index))
        .collect::<Vec<_>>();

    if heading_lines.is_empty() {
        return paragraph_chunks(text);
    }

    ranges_from_starts(text, heading_lines)
}

fn is_markdown_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let mut marker_len = 0;
    while marker_len < bytes.len() && bytes[marker_len] == b'#' {
        marker_len += 1;
    }
    (1..=6).contains(&marker_len)
        && match bytes.get(marker_len) {
            Some(&b) => b == b' ' || b == b'\t' || b == b'\r' || b == b'\n',
            None => true,
        }
}

text_parser!(
    MarkdownParser,
    "markdown-parser",
    |file: &maestria_ports::FileMetadata| {
        extension_is(file, &crate::chunking::DOC_EXTENSIONS[0..2])
    },
    "markdown-parser",
    "1.0",
    "markdown",
    markdown_chunks
);
