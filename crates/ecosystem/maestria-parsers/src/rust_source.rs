#![forbid(unsafe_code)]

use crate::chunking::{extension_is, structural_chunks};
use crate::text_parser;

fn is_rust_pending(line: &str) -> bool {
    line.starts_with("#[")
}

fn is_rust_comment(line: &str) -> bool {
    line.starts_with("//")
}

fn is_rust_structural_start(trimmed: &str) -> bool {
    let without_visibility = match trimmed.strip_prefix("pub ") {
        Some(stripped) => stripped,
        None => trimmed,
    };
    let without_async = match without_visibility.strip_prefix("async ") {
        Some(stripped) => stripped,
        None => without_visibility,
    };
    without_async.starts_with("fn ")
        || without_async.starts_with("struct ")
        || without_async.starts_with("enum ")
        || without_async.starts_with("trait ")
        || without_visibility.starts_with("impl")
}

fn rust_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    structural_chunks(
        text,
        is_rust_pending,
        is_rust_structural_start,
        is_rust_comment,
    )
}

text_parser!(
    RustSourceParser,
    "rust-source-parser",
    |file: &maestria_ports::FileMetadata| {
        extension_is(file, &crate::chunking::CODE_EXTENSIONS[0..1])
    },
    "rust-source-v1",
    "tree-v1",
    "rust",
    rust_chunks
);
