use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use crate::provenance::{
    artifact_id_for, content_hash, decode_utf8_lossy, evidence_id_for, excerpt_for, file_metadata,
    line_range_for_chunk, title_for_path,
};
use crate::recovery::reuse_existing_artifact;
use crate::types::{IngestFileInput, IngestFileOutput};

use maestria_domain::{
    Artifact, Card, Chunk, CreateCardInput, DomainEvent, DomainEventEnvelope, EventId, Evidence,
    EvidenceKind, LogicalTick, SequenceNumber,
};
use maestria_ports::{EventFilter, FileHandle, IndexedChunk, ParsedArtifact, ParsedChunk};

pub(super) fn ingest_file_from_bytes<'a>(
    ports: &CorePorts<'a>,
    input: IngestFileInput,
) -> CoreResult<IngestFileOutput> {
    if input.bytes.is_empty() {
        return Err(CoreError::InvalidInput {
            message: "ingested file bytes must not be empty".to_string(),
        });
    }

    let metadata = file_metadata(&input.path, input.bytes.len());
    if !ports.parser.supports(&metadata) {
        return Err(CoreError::InvalidInput {
            message: format!("no parser supports {}", input.path.display()),
        });
    }

    let artifact_id = match input.artifact_id {
        Some(artifact_id) => artifact_id,
        None => artifact_id_for(&input.path, &input.bytes),
    };
    let observed_content_hash = content_hash(&input.bytes);
    let parsed = parse_input(ports, input.path.clone(), input.bytes.clone(), artifact_id)?;

    if parsed.chunks.is_empty() {
        return Err(CoreError::InvalidInput {
            message: "parser returned no chunks for non-empty input".to_string(),
        });
    }

    let blob_id = ports.blobs.put(input.bytes.clone())?;
    if let Some(existing) =
        reuse_existing_artifact(ports, artifact_id, &input, &parsed, &observed_content_hash)?
    {
        return Ok(IngestFileOutput {
            artifact: existing.artifact,
            chunks: existing.chunks,
            evidence: existing.evidence,
            blob_id,
            content_hash: observed_content_hash,
            unchanged: true,
        });
    }

    ingest_new_artifact(NewArtifactInput {
        ports,
        artifact_id,
        path: input.path,
        bytes: input.bytes,
        observed_at: input.observed_at,
        observed_content_hash,
        blob_id,
        parsed,
    })
}

fn parse_input<'a>(
    ports: &CorePorts<'a>,
    path: std::path::PathBuf,
    bytes: Vec<u8>,
    artifact_id: maestria_domain::ArtifactId,
) -> CoreResult<ParsedArtifact> {
    let parsed = ports.parser.parse(
        FileHandle { path, bytes },
        maestria_ports::ParseContext { artifact_id },
    )?;
    Ok(parsed)
}

struct NewArtifactInput<'a> {
    ports: &'a CorePorts<'a>,
    artifact_id: maestria_domain::ArtifactId,
    path: std::path::PathBuf,
    bytes: Vec<u8>,
    observed_at: LogicalTick,
    observed_content_hash: String,
    blob_id: maestria_domain::BlobId,
    parsed: ParsedArtifact,
}

fn ingest_new_artifact(input: NewArtifactInput<'_>) -> CoreResult<IngestFileOutput> {
    let NewArtifactInput {
        ports,
        artifact_id,
        path,
        bytes,
        observed_at,
        observed_content_hash,
        blob_id,
        parsed,
    } = input;
    let mut artifact = Artifact {
        id: artifact_id,
        title: title_for_path(&path),
        chunk_ids: std::collections::BTreeSet::new(),
        card_ids: std::collections::BTreeSet::new(),
        claim_ids: std::collections::BTreeSet::new(),
        evidence_ids: std::collections::BTreeSet::new(),
    };
    ports.artifacts.put(artifact.clone())?;
    append_event(
        ports,
        DomainEvent::ArtifactRegistered {
            artifact_id,
            title: artifact.title.clone(),
        },
    )?;
    let source_text = decode_utf8_lossy(&bytes);
    let mut search_start = 0usize;
    let (persisted_chunks, persisted_evidence, indexed_chunks) =
        persist_chunks(ChunkPersistenceInput {
            ports,
            artifact_id,
            path: &path,
            source_text: &source_text,
            observed_at,
            observed_content_hash: &observed_content_hash,
            blob_id,
            search_start: &mut search_start,
            parsed_chunks: parsed.chunks,
            artifact: &mut artifact,
        })?;

    persist_cards(ports, parsed.cards, &mut artifact)?;
    ports.search_index.index_chunks(indexed_chunks)?;
    append_event(
        ports,
        DomainEvent::ArtifactParsed {
            artifact_id,
            chunks_added: persisted_chunks.len().min(u32::MAX as usize) as u32,
        },
    )?;
    ports.artifacts.put(artifact.clone())?;

    Ok(IngestFileOutput {
        artifact,
        chunks: persisted_chunks,
        evidence: persisted_evidence,
        blob_id,
        content_hash: observed_content_hash,
        unchanged: false,
    })
}
struct ChunkPersistenceInput<'a, 'b> {
    ports: &'a CorePorts<'a>,
    artifact_id: maestria_domain::ArtifactId,
    path: &'a std::path::Path,
    source_text: &'a str,
    observed_at: LogicalTick,
    observed_content_hash: &'a str,
    blob_id: maestria_domain::BlobId,
    search_start: &'b mut usize,
    parsed_chunks: Vec<ParsedChunk>,
    artifact: &'b mut Artifact,
}

fn persist_chunks(
    input: ChunkPersistenceInput<'_, '_>,
) -> CoreResult<(Vec<Chunk>, Vec<Evidence>, Vec<IndexedChunk>)> {
    let ChunkPersistenceInput {
        ports,
        artifact_id,
        path,
        source_text,
        observed_at,
        observed_content_hash,
        blob_id,
        search_start,
        parsed_chunks,
        artifact,
    } = input;
    let mut persisted_chunks = Vec::with_capacity(parsed_chunks.len());
    let mut persisted_evidence = Vec::with_capacity(parsed_chunks.len());
    let mut indexed_chunks = Vec::with_capacity(parsed_chunks.len());

    for (order, parsed_chunk) in parsed_chunks.into_iter().enumerate() {
        let order = u32::try_from(order).map_err(|_| CoreError::InvalidInput {
            message: "parsed chunk count exceeds u32 order range".to_string(),
        })?;

        let chunk = Chunk {
            id: parsed_chunk.chunk_id,
            artifact_id: parsed_chunk.artifact_id,
            order,
            text: parsed_chunk.text,
        };
        let range = line_range_for_chunk(source_text, &chunk.text, search_start);
        let evidence_id = evidence_id_for(artifact_id, order);
        let proposed_evidence = Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: path.display().to_string(),
                range,
                content_hash: observed_content_hash.to_string(),
                snapshot: Some(blob_id),
            },
            excerpt: excerpt_for(&chunk.text),
            observed_at,
        };

        let evidence = match ports.evidence.get(evidence_id)? {
            Some(existing)
                if existing.artifact_id == proposed_evidence.artifact_id
                    && existing.claim_id == proposed_evidence.claim_id
                    && existing.kind == proposed_evidence.kind
                    && existing.excerpt == proposed_evidence.excerpt =>
            {
                existing
            }
            _ => proposed_evidence,
        };

        ports.chunks.put(chunk.clone())?;
        append_event(
            ports,
            DomainEvent::ChunkRegistered {
                chunk_id: chunk.id,
                artifact_id,
                order,
                text: chunk.text.clone(),
            },
        )?;
        ports.evidence.put(evidence.clone())?;
        append_event(
            ports,
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id: None,
                kind: evidence.kind.clone(),
                excerpt: evidence.excerpt.clone(),
                observed_at: evidence.observed_at,
            },
        )?;

        artifact.chunk_ids.insert(chunk.id);
        artifact.evidence_ids.insert(evidence_id);
        indexed_chunks.push(IndexedChunk {
            artifact_id,
            chunk_id: chunk.id,
            text: chunk.text.clone(),
        });
        persisted_chunks.push(chunk);
        persisted_evidence.push(evidence);
    }

    Ok((persisted_chunks, persisted_evidence, indexed_chunks))
}

fn persist_cards<'a>(
    ports: &CorePorts<'a>,
    parsed_cards: Vec<CreateCardInput>,
    artifact: &mut Artifact,
) -> CoreResult<()> {
    for card_input in parsed_cards {
        let card = Card {
            id: card_input.card_id,
            artifact_id: card_input.artifact_id,
            title: card_input.title,
            body: card_input.body,
            claim_ids: std::collections::BTreeSet::new(),
        };
        ports.cards.put(card.clone())?;
        append_event(
            ports,
            DomainEvent::CardCreated {
                card_id: card.id,
                artifact_id: artifact.id,
                title: card.title.clone(),
                body: card.body.clone(),
            },
        )?;
        artifact.card_ids.insert(card.id);
    }

    Ok(())
}

fn append_event<'a>(ports: &CorePorts<'a>, event: DomainEvent) -> CoreResult<DomainEventEnvelope> {
    let events = ports.events.scan(EventFilter { artifact_id: None })?;

    if let Some(existing) = events
        .iter()
        .find(|envelope| same_event_identity(&envelope.event, &event))
    {
        return Ok(existing.clone());
    }

    let latest_sequence = events.iter().map(|event| event.sequence.value()).max();
    let next = match latest_sequence {
        Some(sequence) => sequence.saturating_add(1),
        None => 1,
    };
    let envelope = DomainEventEnvelope {
        id: EventId::new(next),
        sequence: SequenceNumber::new(next),
        event,
    };
    ports.events.append(envelope.clone())?;
    Ok(envelope)
}

fn same_event_identity(
    left: &maestria_domain::DomainEvent,
    right: &maestria_domain::DomainEvent,
) -> bool {
    match (left, right) {
        (
            maestria_domain::DomainEvent::EvidenceRecorded {
                evidence_id: left_id,
                artifact_id: left_artifact_id,
                claim_id: left_claim_id,
                kind: left_kind,
                excerpt: left_excerpt,
                ..
            },
            maestria_domain::DomainEvent::EvidenceRecorded {
                evidence_id: right_id,
                artifact_id: right_artifact_id,
                claim_id: right_claim_id,
                kind: right_kind,
                excerpt: right_excerpt,
                ..
            },
        ) => {
            left_id == right_id
                && left_artifact_id == right_artifact_id
                && left_claim_id == right_claim_id
                && left_kind == right_kind
                && left_excerpt == right_excerpt
        }
        _ => left == right,
    }
}
