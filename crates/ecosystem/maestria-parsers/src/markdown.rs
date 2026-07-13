#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{
    decode_utf8, extension_is, paragraph_chunks, parsed_artifact, ranges_from_starts,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownParser;

impl MarkdownParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for MarkdownParser {
    fn id(&self) -> &'static str {
        "markdown-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["md", "markdown"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = markdown_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

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
    let marker_len = trimmed.chars().take_while(|ch| *ch == '#').count();
    (1..=6).contains(&marker_len)
        && match trimmed.chars().nth(marker_len) {
            Some(ch) => ch.is_whitespace(),
            None => true,
        }
}
