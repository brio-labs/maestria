#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{
    decode_utf8, extension_is, paragraph_chunks, parsed_artifact, ranges_from_starts,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct PythonSourceParser;

impl PythonSourceParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for PythonSourceParser {
    fn id(&self) -> &'static str {
        "python-source-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["py"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes.clone())?;
        let chunks = python_chunks(&text);
        parsed_artifact(
            context.artifact_id,
            &file.path,
            &file.bytes,
            chunks,
            "python-source-v1".to_string(),
            "tree-v1".to_string(),
            Some("python".to_string()),
        )
    }
}

/// Structural chunking for Python source: one chunk per `class`/`def`/
/// `async def` declaration (with its `@` decorator lines), mirroring
/// `rust_chunks`. Falls back to paragraph chunks when the file has no
/// declarations.
fn python_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    let mut starts = Vec::new();
    let mut pending_decorator_start = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('@') {
            pending_decorator_start.get_or_insert(index);
            continue;
        }

        if is_python_structural_start(trimmed) {
            let start = match pending_decorator_start.take() {
                Some(decorator_start) => decorator_start,
                None => index,
            };
            starts.push(start);
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            pending_decorator_start = None;
        }
    }

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    starts.sort_unstable();
    starts.dedup();
    ranges_from_starts(text, starts)
}

fn is_python_structural_start(trimmed: &str) -> bool {
    trimmed.starts_with("class ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("async def ")
}
