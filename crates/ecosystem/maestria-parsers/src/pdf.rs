#![forbid(unsafe_code)]

use maestria_ports::{
    FileHandle, FileMetadata, ParseContext, ParsedArtifact, ParsedChunk, Parser, PortError,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct PdfParser;

impl PdfParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for PdfParser {
    fn id(&self) -> &'static str {
        "pdf-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.extension
            .as_deref()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        // Use lopdf to load the document from bytes
        let doc = lopdf::Document::load_mem(&file.bytes).map_err(|e| PortError::InvalidInput {
            message: format!("PDF parse error: {e}"),
        })?;

        // Extract text page by page
        let page_nums: Vec<_> = doc.get_pages().keys().copied().collect();
        let mut chunks = Vec::new();

        for page_num in &page_nums {
            #[allow(clippy::manual_unwrap_or_default)]
            let text = match doc.extract_text(&[*page_num]) {
                Ok(t) => t,
                Err(_) => String::new(),
            };
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                chunks.push(trimmed);
            }
        }

        if chunks.is_empty() {
            return Err(PortError::InvalidInput {
                message: "PDF has no extractable text".to_string(),
            });
        }

        // Build chunks with deterministic IDs
        let mut parsed_chunks = Vec::with_capacity(chunks.len());
        for (order, text) in chunks.into_iter().enumerate() {
            parsed_chunks.push(ParsedChunk {
                chunk_id: crate::chunk_id_for(context.artifact_id, order)?,
                artifact_id: context.artifact_id,
                text,
            });
        }

        let card = crate::summary_card_for(context.artifact_id, &file.path, &parsed_chunks);

        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            chunks: parsed_chunks,
            cards: vec![card],
        })
    }
}
