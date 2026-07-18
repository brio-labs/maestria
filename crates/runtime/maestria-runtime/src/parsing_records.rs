use crate::parser_mapping::{domain_representation, domain_source_span};
use maestria_domain::{
    ArtifactId, BlobId, ContentRange, EvidenceKind, LogicalTick, RecordEvidenceInput,
    RegisterChunkInput, evidence_id_for, excerpt_for,
};

pub(crate) fn build_indexable_records(
    parsed: &maestria_ports::ParsedArtifact,
    artifact_id: ArtifactId,
    blob_id: BlobId,
    source_path: &str,
    source_hash: &str,
) -> Option<(
    Vec<RecordEvidenceInput>,
    Vec<RegisterChunkInput>,
    Vec<maestria_domain::CreateCardInput>,
)> {
    let mut evidence_inputs = Vec::new();
    let mut chunks = Vec::new();
    let observed_at = LogicalTick::new(1);

    for (order, chunk) in parsed.chunks.iter().enumerate() {
        let evidence_id = evidence_id_for(artifact_id, order as u32);
        let excerpt = excerpt_for(&chunk.text);
        let kind = match &chunk.source_span {
            maestria_ports::SourceSpan::TextSpan {
                start_line,
                end_line,
            } => EvidenceKind::FileSpan {
                path: source_path.to_string(),
                range: ContentRange {
                    start: *start_line,
                    end: *end_line,
                },
                content_hash: source_hash.to_string(),
                snapshot: Some(blob_id),
            },
            maestria_ports::SourceSpan::PdfSpan { page } => {
                let page = match u32::try_from(*page) {
                    Ok(page) => page,
                    Err(_) => {
                        tracing::error!(
                            artifact_id = %artifact_id,
                            page = *page,
                            "parser PDF page exceeds domain evidence range"
                        );
                        return None;
                    }
                };
                EvidenceKind::PdfSpan {
                    blob: blob_id,
                    page_start: page,
                    page_end: page,
                }
            }
        };
        evidence_inputs.push(RecordEvidenceInput {
            evidence_id,
            artifact_id,
            claim_id: None,
            kind,
            excerpt,
            observed_at,
            security: None,
        });
        chunks.push(RegisterChunkInput {
            chunk_id: chunk.chunk_id,
            artifact_id: chunk.artifact_id,
            node_id: chunk.node_id,
            source_span: domain_source_span(&chunk.source_span),
            representations: chunk
                .representations
                .iter()
                .map(domain_representation)
                .collect(),
            order: (order.min(u32::MAX as usize)) as u32,
            text: chunk.text.clone(),
        });
    }

    let cards = parsed
        .cards
        .iter()
        .map(|parsed_card| {
            let mut card = parsed_card.card.clone();
            card.node_id = parsed_card.node_id;
            card.source_span = domain_source_span(&parsed_card.source_span);
            card
        })
        .collect();

    Some((evidence_inputs, chunks, cards))
}
