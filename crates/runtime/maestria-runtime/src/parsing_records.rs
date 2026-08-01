use std::fmt;

use crate::parser_mapping::{domain_representation, domain_source_span};
use maestria_domain::{
    ArtifactId, BlobId, ContentHash, EvidenceKind, LineRange, LogicalTick, RecordEvidenceInput,
    RegisterChunkInput, SecurityMetadata, SnapshotRef, TrustZone, evidence_id_for, excerpt_for,
};
use maestria_governance::contains_prompt_injection_risk;
use maestria_ports::{ParsedArtifact, ParsedCard, ParsedChunk, SourceSpan};

#[derive(Debug)]
pub(crate) struct IndexableRecordsError {
    artifact_id: ArtifactId,
    chunk_order: u32,
    field: &'static str,
    reason: String,
}

impl IndexableRecordsError {
    fn new(
        artifact_id: ArtifactId,
        chunk_order: u32,
        field: &'static str,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            artifact_id,
            chunk_order,
            field,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for IndexableRecordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "artifact {} chunk {} field {}: {}",
            self.artifact_id, self.chunk_order, self.field, self.reason
        )
    }
}

impl std::error::Error for IndexableRecordsError {}
type IndexableRecords = (
    Vec<RecordEvidenceInput>,
    Vec<RegisterChunkInput>,
    Vec<maestria_domain::CreateCardInput>,
);

fn security_for_text(text: &str) -> SecurityMetadata {
    let prompt_injection_risk = contains_prompt_injection_risk(text);
    SecurityMetadata {
        trust_zone: if prompt_injection_risk {
            TrustZone::Quarantined
        } else {
            TrustZone::Untrusted
        },
        prompt_injection_risk,
        ..SecurityMetadata::default()
    }
}

pub(crate) fn build_indexable_records(
    parsed: &ParsedArtifact,
    artifact_id: ArtifactId,
    blob_id: BlobId,
    source_path: &str,
    source_hash: &str,
) -> Result<IndexableRecords, IndexableRecordsError> {
    let mut evidence_inputs = Vec::new();
    let mut chunks = Vec::new();
    let observed_at = LogicalTick::new(1);

    for (order, chunk) in parsed.chunks.iter().enumerate() {
        let order = u32::try_from(order).map_err(|error| {
            IndexableRecordsError::new(
                artifact_id,
                u32::MAX,
                "chunk.order",
                format!("chunk order exceeds u32: {error}"),
            )
        })?;
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

    Ok((evidence_inputs, chunks, cards))
}

fn chunk_to_evidence(
    chunk: &ParsedChunk,
    order: u32,
    artifact_id: ArtifactId,
    blob_id: BlobId,
    source_path: &str,
    source_hash: &str,
    observed_at: LogicalTick,
) -> Result<RecordEvidenceInput, IndexableRecordsError> {
    let evidence_id = evidence_id_for(artifact_id, order);
    let excerpt = excerpt_for(&chunk.text);
    let kind = evidence_kind_from_span(
        &chunk.source_span,
        order,
        source_path,
        source_hash,
        blob_id,
        artifact_id,
    )?;
    Ok(RecordEvidenceInput {
        evidence_id,
        artifact_id,
        claim_id: None,
        kind,
        excerpt,
        observed_at,
        security: Some(security_for_text(&chunk.text)),
    })
}

fn evidence_kind_from_span(
    span: &SourceSpan,
    chunk_order: u32,
    source_path: &str,
    source_hash: &str,
    blob_id: BlobId,
    artifact_id: ArtifactId,
) -> Result<EvidenceKind, IndexableRecordsError> {
    let content_hash = || {
        ContentHash::new(source_hash.to_owned()).map_err(|error| {
            IndexableRecordsError::new(
                artifact_id,
                chunk_order,
                "source_hash",
                format!("invalid content hash: {error}"),
            )
        })
    };

    match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => {
            let range = LineRange::new(*start_line, *end_line).map_err(|error| {
                IndexableRecordsError::new(
                    artifact_id,
                    chunk_order,
                    "source_span.line_range",
                    error.to_string(),
                )
            })?;
            Ok(EvidenceKind::FileSpan {
                path: source_path.to_string(),
                range,
                snapshot: SnapshotRef::new(blob_id, content_hash()?),
            })
        }
        SourceSpan::PdfSpan { page } => {
            let page = u32::try_from(*page).map_err(|error| {
                IndexableRecordsError::new(
                    artifact_id,
                    chunk_order,
                    "source_span.page",
                    format!("PDF page exceeds domain evidence range: {error}"),
                )
            })?;
            Ok(EvidenceKind::PdfSpan {
                snapshot: SnapshotRef::new(blob_id, content_hash()?),
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
            let page = u32::try_from(*page).map_err(|error| {
                IndexableRecordsError::new(
                    artifact_id,
                    chunk_order,
                    "source_span.page",
                    format!("PDF region page exceeds domain evidence range: {error}"),
                )
            })?;
            Ok(EvidenceKind::PdfRegion {
                snapshot: SnapshotRef::new(blob_id, content_hash()?),
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
    order: u32,
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
        order,
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
            card.security = Some(security_for_text(&card.body));
            card
        })
        .collect()
}
