use crate::parser_mapping::{domain_representation, domain_source_span};
use maestria_domain::{
    ArtifactId, BlobId, ContentRange, EvidenceKind, LogicalTick, RecordEvidenceInput,
    RegisterChunkInput, evidence_id_for, excerpt_for,
};
use maestria_ports::{ParsedArtifact, ParsedCard, ParsedChunk, SourceSpan};

pub(crate) fn build_indexable_records(
    parsed: &ParsedArtifact,
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
        let evidence = chunk_to_evidence(
            chunk,
            order,
            artifact_id,
            blob_id,
            source_path,
            source_hash,
            observed_at,
        )?;
        let registration = chunk_to_registration(chunk, order, artifact_id);
        evidence_inputs.push(evidence);
        chunks.push(registration);
    }

    let cards = build_cards(&parsed.cards);

    Some((evidence_inputs, chunks, cards))
}

fn chunk_to_evidence(
    chunk: &ParsedChunk,
    order: usize,
    artifact_id: ArtifactId,
    blob_id: BlobId,
    source_path: &str,
    source_hash: &str,
    observed_at: LogicalTick,
) -> Option<RecordEvidenceInput> {
    let evidence_id = evidence_id_for(artifact_id, order as u32);
    let excerpt = excerpt_for(&chunk.text);
    let kind = evidence_kind_from_span(
        &chunk.source_span,
        source_path,
        source_hash,
        blob_id,
        artifact_id,
    )?;
    Some(RecordEvidenceInput {
        evidence_id,
        artifact_id,
        claim_id: None,
        kind,
        excerpt,
        observed_at,
        security: None,
    })
}

fn evidence_kind_from_span(
    span: &SourceSpan,
    source_path: &str,
    source_hash: &str,
    blob_id: BlobId,
    artifact_id: ArtifactId,
) -> Option<EvidenceKind> {
    match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => Some(EvidenceKind::FileSpan {
            path: source_path.to_string(),
            range: ContentRange {
                start: *start_line,
                end: *end_line,
            },
            content_hash: source_hash.to_string(),
            snapshot: Some(blob_id),
        }),
        SourceSpan::PdfSpan { page } => {
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
            Some(EvidenceKind::PdfSpan {
                blob: blob_id,
                page_start: page,
                page_end: page,
            })
        }
        SourceSpan::PdfRegion {
            page,
            x,
            y,
            width,
            height,
        } => {
            let page = match u32::try_from(*page) {
                Ok(page) => page,
                Err(_) => {
                    tracing::error!(
                        artifact_id = %artifact_id,
                        page = *page,
                        "parser PDF region page exceeds domain evidence range"
                    );
                    return None;
                }
            };
            Some(EvidenceKind::PdfRegion {
                blob: blob_id,
                page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            })
        }
    }
}

fn chunk_to_registration(
    chunk: &ParsedChunk,
    order: usize,
    artifact_id: ArtifactId,
) -> RegisterChunkInput {
    RegisterChunkInput {
        chunk_id: chunk.chunk_id,
        artifact_id,
        node_id: chunk.node_id,
        source_span: domain_source_span(&chunk.source_span),
        representations: chunk
            .representations
            .iter()
            .map(domain_representation)
            .collect(),
        order: (order.min(u32::MAX as usize)) as u32,
        text: chunk.text.clone(),
    }
}

fn build_cards(parsed_cards: &[ParsedCard]) -> Vec<maestria_domain::CreateCardInput> {
    parsed_cards
        .iter()
        .map(|parsed_card| {
            let mut card = parsed_card.card.clone();
            card.node_id = parsed_card.node_id;
            card.source_span = domain_source_span(&parsed_card.source_span);
            card
        })
        .collect()
}
