#![forbid(unsafe_code)]

use crate::chunking::{extension_is, paragraph_chunks, ranges_from_starts};
use sillage_ports::{
    BoundedParseError, FileHandle, FileHandleRef, FileMetadata, FileMetadataRef, ParseBudget,
    ParseContext, ParseOutcome, ParsedArtifact, Parser, PortError,
};

fn markdown_chunks(text: &str) -> Vec<(String, sillage_ports::SourceSpan)> {
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

fn parse_markdown_bounded(
    file: FileHandleRef<'_>,
    context: ParseContext,
    budget: &mut ParseBudget<'_>,
) -> Result<ParseOutcome, BoundedParseError> {
    budget.checkpoint()?;
    let chunk_ranges = crate::bounded_markdown::chunk_ranges(file.bytes, budget)?;
    let parsed = crate::bounded_output::parsed_artifact_from_ranges_bounded(
        file,
        context,
        chunk_ranges,
        "markdown-parser",
        "1.0",
        Some("markdown"),
        budget,
    )?;
    Ok(ParseOutcome::Complete(parsed))
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
        extension_is(file, &crate::chunking::DOC_EXTENSIONS[0..2])
    }

    fn supports_bounded(
        &self,
        file: &FileMetadataRef<'_>,
        budget: &mut ParseBudget<'_>,
    ) -> Result<bool, BoundedParseError> {
        let Some(extension) = file.extension else {
            return Ok(false);
        };
        if extension.len() != 2 && extension.len() != 8 {
            return Ok(false);
        }
        budget.charge_work(extension.len() as u64)?;
        Ok(extension.eq_ignore_ascii_case(b"md") || extension.eq_ignore_ascii_case(b"markdown"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = crate::chunking::decode_utf8(&file.bytes)?;
        let chunks = markdown_chunks(text);
        crate::artifact_builder::parsed_artifact(
            context.artifact_id,
            &file.path,
            &file.bytes,
            chunks,
            "markdown-parser".to_string(),
            "1.0".to_string(),
            Some("markdown".to_string()),
        )
    }

    fn parse_outcome_bounded(
        &self,
        file: FileHandleRef<'_>,
        context: ParseContext,
        budget: &mut ParseBudget<'_>,
    ) -> Result<ParseOutcome, BoundedParseError> {
        parse_markdown_bounded(file, context, budget)
    }
}
