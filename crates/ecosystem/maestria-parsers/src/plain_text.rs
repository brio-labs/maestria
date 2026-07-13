#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{decode_utf8, extension_is, paragraph_chunks, parsed_artifact};

#[derive(Debug, Clone, Copy, Default)]
pub struct PlainTextParser;

impl PlainTextParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for PlainTextParser {
    fn id(&self) -> &'static str {
        "plain-text-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["txt", "text"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = paragraph_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}
