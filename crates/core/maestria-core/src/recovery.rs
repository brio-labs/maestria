use crate::error::CoreError;
use crate::ports::CorePorts;
use crate::provenance::{evidence_id_for, title_for_path};
use crate::types::IngestFileInput;

use maestria_domain::{
    Artifact, Card, Chunk, DomainEvent, DomainEventEnvelope, Evidence, EvidenceKind,
};
use maestria_ports::ParsedArtifact;

pub(super) struct ExistingArtifact {
    pub(super) artifact: Artifact,
    pub(super) chunks: Vec<Chunk>,
    pub(super) evidence: Vec<Evidence>,
}

pub(super) fn reuse_existing_artifact<'a>(
    ports: &CorePorts<'a>,
    artifact_id: maestria_domain::ArtifactId,
    input: &IngestFileInput,
    parsed: &ParsedArtifact,
    observed_content_hash: &str,
) -> Result<Option<ExistingArtifact>, CoreError> {
    let Some(existing_artifact) = ports.artifacts.get(artifact_id)? else {
        return Ok(None);
    };

    let existing_chunks = ports.chunks.list_for_artifact(artifact_id)?;
    let existing_cards = ports.cards.list_for_artifact(artifact_id)?;
    let existing_evidence = ports.evidence.list_for_artifact(artifact_id)?;
    let events = ports
        .events
        .scan(maestria_ports::EventFilter { artifact_id: None })?;

    if ingestion_is_complete(
        &IngestionCheck {
            artifact: &existing_artifact,
            chunks: &existing_chunks,
            cards: &existing_cards,
            evidence: &existing_evidence,
            events: &events,
        },
        parsed,
        &input.path,
        observed_content_hash,
    ) {
        return Ok(Some(ExistingArtifact {
            artifact: existing_artifact,
            chunks: existing_chunks,
            evidence: existing_evidence,
        }));
    }

    let source_path = input.path.display().to_string();
    let has_different_content = existing_evidence.iter().any(|evidence| {
        matches!(
            &evidence.kind,
            EvidenceKind::FileSpan {
                path,
                content_hash: existing_hash,
                ..
            } if path != &source_path || existing_hash != observed_content_hash
        )
    });
    let has_completed_event = events.iter().any(|envelope| {
        matches!(
            envelope.event,
            DomainEvent::ArtifactParsed {
                artifact_id: parsed_artifact_id,
                ..
            } if parsed_artifact_id == artifact_id
        )
    });

    if input.artifact_id.is_some()
        && (has_different_content || (has_completed_event && existing_evidence.is_empty()))
    {
        return Err(CoreError::InvalidInput {
            message: format!(
                "artifact {} already contains a different or untraceable content version",
                artifact_id
            ),
        });
    }

    Ok(None)
}

struct IngestionCheck<'a> {
    artifact: &'a Artifact,
    chunks: &'a [Chunk],
    cards: &'a [Card],
    evidence: &'a [Evidence],
    events: &'a [DomainEventEnvelope],
}

fn ingestion_is_complete(
    existing: &IngestionCheck<'_>,
    parsed: &ParsedArtifact,
    path: &std::path::Path,
    observed_content_hash: &str,
) -> bool {
    if parsed.chunks.is_empty() {
        return false;
    }
    if !has_compatible_shape(existing, parsed, path) {
        return false;
    }
    if !chunks_match(existing, parsed, path, observed_content_hash) {
        return false;
    }
    if !cards_match(existing, parsed) {
        return false;
    }
    !events_missing(existing, parsed)
}

fn has_compatible_shape(
    existing: &IngestionCheck<'_>,
    parsed: &ParsedArtifact,
    path: &std::path::Path,
) -> bool {
    let IngestionCheck {
        artifact,
        chunks,
        cards,
        evidence,
        ..
    } = existing;

    artifact.title == title_for_path(path)
        && artifact.chunk_ids.len() == parsed.chunks.len()
        && artifact.card_ids.len() == parsed.cards.len()
        && artifact.evidence_ids.len() == parsed.chunks.len()
        && chunks.len() == parsed.chunks.len()
        && cards.len() == parsed.cards.len()
        && evidence.len() == parsed.chunks.len()
}

fn chunks_match(
    existing: &IngestionCheck<'_>,
    parsed: &ParsedArtifact,
    path: &std::path::Path,
    observed_content_hash: &str,
) -> bool {
    let IngestionCheck {
        artifact,
        chunks,
        evidence,
        ..
    } = existing;
    let source_path = path.display().to_string();

    for (order, expected) in parsed.chunks.iter().enumerate() {
        let Some(chunk) = chunks.iter().find(|chunk| chunk.id == expected.chunk_id) else {
            return false;
        };
        if chunk.artifact_id != artifact.id
            || chunk.order != order as u32
            || chunk.text != expected.text
            || !artifact.chunk_ids.contains(&chunk.id)
        {
            return false;
        }

        let expected_evidence_id = evidence_id_for(artifact.id, order as u32);
        let Some(source_evidence) = evidence.iter().find(|item| item.id == expected_evidence_id)
        else {
            return false;
        };
        if !artifact.evidence_ids.contains(&source_evidence.id)
            || source_evidence.artifact_id != artifact.id
            || !matches!(
                &source_evidence.kind,
                EvidenceKind::FileSpan {
                    path,
                    content_hash: existing_hash,
                    ..
                } if path == &source_path && existing_hash == observed_content_hash
            )
        {
            return false;
        }
    }

    true
}

fn cards_match(existing: &IngestionCheck<'_>, parsed: &ParsedArtifact) -> bool {
    let IngestionCheck {
        artifact, cards, ..
    } = existing;

    for expected in &parsed.cards {
        let Some(card) = cards.iter().find(|card| card.id == expected.card_id) else {
            return false;
        };
        if card.artifact_id != artifact.id
            || card.title != expected.title
            || card.body != expected.body
            || !artifact.card_ids.contains(&card.id)
        {
            return false;
        }
    }

    true
}

fn events_missing(existing: &IngestionCheck<'_>, parsed: &ParsedArtifact) -> bool {
    let IngestionCheck {
        artifact, events, ..
    } = existing;

    let has_event = |predicate: &dyn Fn(&DomainEvent) -> bool| {
        events.iter().any(|envelope| predicate(&envelope.event))
    };

    if !has_event(&|event| {
        matches!(
            event,
            DomainEvent::ArtifactRegistered {
                artifact_id,
                title
            } if *artifact_id == artifact.id && title == &artifact.title
        )
    }) {
        return true;
    }

    if !has_event(&|event| {
        matches!(
            event,
            DomainEvent::ArtifactParsed {
                artifact_id,
                chunks_added
            } if *artifact_id == artifact.id && *chunks_added == parsed.chunks.len() as u32
        )
    }) {
        return true;
    }

    for (order, expected) in parsed.chunks.iter().enumerate() {
        if !has_event(&|event| {
            matches!(
                event,
                DomainEvent::ChunkRegistered {
                    chunk_id,
                    artifact_id,
                    order: event_order,
                    text
                } if *chunk_id == expected.chunk_id
                    && *artifact_id == artifact.id
                    && *event_order == order as u32
                    && text == &expected.text
            )
        }) {
            return true;
        }
        if !has_event(&|event| {
            matches!(
                event,
                DomainEvent::EvidenceRecorded {
                    evidence_id,
                    artifact_id,
                    ..
                } if *evidence_id == evidence_id_for(artifact.id, order as u32)
                    && *artifact_id == artifact.id
            )
        }) {
            return true;
        }
    }

    !parsed.cards.iter().all(|expected| {
        has_event(&|event| {
            matches!(
                event,
                DomainEvent::CardCreated {
                    card_id,
                    artifact_id,
                    title,
                    body
                } if *card_id == expected.card_id
                    && *artifact_id == artifact.id
                    && title == &expected.title
                    && body == &expected.body
            )
        })
    })
}
