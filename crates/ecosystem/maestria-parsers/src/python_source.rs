#![forbid(unsafe_code)]

use crate::chunking::{extension_is, structural_chunks};
use crate::text_parser;

fn is_python_pending(line: &str) -> bool {
    line.starts_with('@')
}

fn is_python_comment(line: &str) -> bool {
    line.starts_with('#')
}

fn is_python_structural_start(trimmed: &str) -> bool {
    trimmed.starts_with("class ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("async def ")
}

fn python_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    structural_chunks(
        text,
        is_python_pending,
        is_python_structural_start,
        is_python_comment,
    )
}

text_parser!(
    PythonSourceParser,
    "python-source-parser",
    |file: &maestria_ports::FileMetadata| {
        extension_is(file, &crate::chunking::CODE_EXTENSIONS[1..2])
    },
    "python-source-v1",
    "tree-v1",
    "python",
    python_chunks
);
