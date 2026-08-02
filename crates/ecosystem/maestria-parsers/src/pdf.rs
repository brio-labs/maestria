#![forbid(unsafe_code)]

use crate::pdf_layout::extract_page_layouts;
use crate::pdf_tree::{build_tree_and_chunks, parsed_card_for};
use maestria_domain::{ContentHash, content_hash};
use maestria_ports::{
    FileHandle, FileMetadata, OcrPageSet, ParseContext, ParseOutcome, ParseStatus, ParsedArtifact,
    Parser, PortError,
};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct PdfParser;

impl PdfParser {
    pub fn new() -> Self {
        Self
    }

    fn parse_layout(
        &self,
        file: &FileHandle,
        context: ParseContext,
        ocr_pages: Option<&[maestria_domain::OcrPageText]>,
    ) -> Result<(ParsedArtifact, Vec<u32>), PortError> {
        let doc = lopdf::Document::load_mem(&file.bytes).map_err(|error| {
            PortError::InvalidInputContext {
                context: "PDF parse error",
                source: error.to_string(),
            }
        })?;
        let mut pages = extract_page_layouts(&doc)?;
        let recognized = match ocr_pages {
            Some(pages) => pages
                .iter()
                .map(|page| (page.page(), page.text().to_string()))
                .collect::<BTreeMap<_, _>>(),
            None => BTreeMap::new(),
        };
        for page in &mut pages {
            if page.needs_ocr
                && let Some(text) = recognized.get(&page.page)
                && !text.trim().is_empty()
            {
                page.text = text.clone();
                page.needs_ocr = false;
            }
        }
        if pages.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "PDF has no pages",
                source: "OCR and rasterization produced no pages".to_string(),
            });
        }
        let has_text = pages.iter().any(|page| !page.text.is_empty());
        let needed = pages
            .iter()
            .filter(|page| page.needs_ocr)
            .map(|page| page.page)
            .collect::<Vec<_>>();
        let (tree, parsed_chunks, root_id) = build_tree_and_chunks(context.artifact_id, &pages)?;
        let parsed_cards = if parsed_chunks.iter().any(|chunk| !chunk.text.is_empty()) {
            vec![parsed_card_for(
                context.artifact_id,
                &file.path,
                &parsed_chunks,
                root_id,
            )?]
        } else {
            Vec::new()
        };
        let hash_string = content_hash(&file.bytes);
        let content_hash = ContentHash::new(hash_string.clone()).map_err(|error| {
            PortError::InvalidInputContext {
                context: "create PDF content hash",
                source: error.to_string(),
            }
        })?;
        let artifact_version_id = crate::chunking::artifact_version_id_for(&content_hash)?;
        let status = if !needed.is_empty() || !has_text {
            ParseStatus::NeedsOcr
        } else {
            ParseStatus::Parsed
        };
        Ok((
            ParsedArtifact {
                artifact_id: context.artifact_id,
                artifact_version_id,
                content_hash,
                tree,
                status,
                chunks: parsed_chunks,
                cards: parsed_cards,
            },
            needed,
        ))
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
        Ok(match self.parse_outcome(file, context)? {
            ParseOutcome::Complete(parsed)
            | ParseOutcome::NeedsOcr {
                partial: parsed, ..
            } => parsed,
        })
    }

    fn parse_outcome(
        &self,
        file: FileHandle,
        context: ParseContext,
    ) -> Result<ParseOutcome, PortError> {
        let (parsed, needed) = self.parse_layout(&file, context, None)?;
        if needed.is_empty() {
            Ok(ParseOutcome::Complete(parsed))
        } else {
            Ok(ParseOutcome::NeedsOcr {
                partial: parsed,
                pages: OcrPageSet::try_new(needed)?,
            })
        }
    }

    fn parse_with_ocr(
        &self,
        file: FileHandle,
        context: ParseContext,
        pages: &[maestria_domain::OcrPageText],
    ) -> Result<ParsedArtifact, PortError> {
        let (parsed, needed) = self.parse_layout(&file, context, Some(pages))?;
        if !needed.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "OCR result did not resolve all requested PDF pages",
                source: format!("{} pages remain unresolved", needed.len()),
            });
        }
        Ok(parsed)
    }
}
