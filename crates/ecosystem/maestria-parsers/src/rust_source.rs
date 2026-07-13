#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{
    decode_utf8, extension_is, paragraph_chunks, parsed_artifact, ranges_from_starts,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct RustSourceParser;

impl RustSourceParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for RustSourceParser {
    fn id(&self) -> &'static str {
        "rust-source-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["rs"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = rust_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

fn rust_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    let mut starts = Vec::new();
    let mut pending_attribute_start = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[") {
            pending_attribute_start.get_or_insert(index);
            continue;
        }

        if is_rust_structural_start(trimmed) {
            let start = match pending_attribute_start.take() {
                Some(attribute_start) => attribute_start,
                None => index,
            };
            starts.push(start);
        } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
            pending_attribute_start = None;
        }
    }

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    starts.sort_unstable();
    starts.dedup();
    ranges_from_starts(text, starts)
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
