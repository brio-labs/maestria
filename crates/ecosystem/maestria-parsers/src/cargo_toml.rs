#![forbid(unsafe_code)]

use crate::chunking::{paragraph_chunks, ranges_from_starts};
use crate::text_parser;

fn cargo_toml_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    let starts = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| is_toml_table_header(line).then_some(index))
        .collect::<Vec<_>>();

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    ranges_from_starts(text, starts)
}

fn is_toml_table_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

text_parser!(
    CargoTomlParser,
    "cargo-toml-parser",
    |file: &maestria_ports::FileMetadata| file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Cargo.toml")),
    "cargo-toml-v1",
    "tree-v1",
    "toml",
    cargo_toml_chunks
);
