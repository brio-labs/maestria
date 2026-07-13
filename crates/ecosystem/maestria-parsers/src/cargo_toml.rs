#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{decode_utf8, paragraph_chunks, parsed_artifact, ranges_from_starts};

#[derive(Debug, Clone, Copy, Default)]
pub struct CargoTomlParser;

impl CargoTomlParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for CargoTomlParser {
    fn id(&self) -> &'static str {
        "cargo-toml-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("Cargo.toml"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = cargo_toml_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

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
