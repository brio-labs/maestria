use crate::{
    ChunkId, FileHandle, FileMetadata, ParseContext, ParsedArtifact, ParsedChunk, Parser,
    PortError, SourceSpan,
};

#[derive(Clone)]
pub struct InMemoryParser;

impl Default for InMemoryParser {
    fn default() -> Self {
        Self
    }
}

impl InMemoryParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for InMemoryParser {
    fn id(&self) -> &'static str {
        "in-memory-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        match file.extension.as_deref() {
            Some(ext) => matches!(ext, "md" | "txt" | "rs" | "toml"),
            None => false,
        }
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        if file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "input file is empty".to_string(),
            });
        }

        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;

        let chunk = ParsedChunk {
            chunk_id: ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            text,
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
        };
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            chunks: vec![chunk],
            cards: Vec::new(),
        })
    }
}
