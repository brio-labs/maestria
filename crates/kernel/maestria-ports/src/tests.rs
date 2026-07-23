use super::contract_tests::*;
use super::graph_contract_tests::assert_graph_index_contract;
use super::*;
use maestria_domain::{
    ArtifactId, DomainEvent, DomainEventEnvelope, Evidence, EvidenceId, EvidenceKind, LogicalTick,
    RelationEndpoint, ValidationReportId,
};
use std::path::PathBuf;

#[test]
fn in_memory_artifact_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_artifact_repository_round_trip(&InMemoryArtifactRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_chunk_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_chunk_repository_round_trip(&InMemoryChunkRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_web_fetcher_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = InMemoryWebFetcher::new();
    fetcher.seed("https://example.com/test", "<html><body>test</body></html>")?;
    assert_web_fetcher_contract(
        &fetcher,
        "https://example.com/test",
        "<html><body>test</body></html>",
    )?;

    let missing_res = fetcher.fetch("https://example.com/not-found-anywhere", usize::MAX);
    assert!(
        matches!(missing_res, Err(PortError::NotFound)),
        "Missing URLs must map to PortError::NotFound, got {:?}",
        missing_res
    );

    Ok(())
}

#[test]
fn in_memory_card_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_card_repository_round_trip(&InMemoryCardRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_repository_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_evidence_repository_round_trip(&InMemoryEvidenceRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_put_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let repo = InMemoryEvidenceRepository::new();
    let evidence = Evidence {
        id: EvidenceId::new(100),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "test excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    // First insert succeeds
    repo.put(evidence.clone())?;
    // Identical retry is idempotent
    repo.put(evidence.clone())?;
    // Stored value is unchanged
    let stored = repo
        .get(evidence.id)?
        .ok_or_else(|| std::io::Error::other("stored evidence missing"))?;
    assert_eq!(stored, evidence);
    Ok(())
}

#[test]
fn in_memory_evidence_repository_satisfies_replace_contract()
-> Result<(), Box<dyn std::error::Error>> {
    assert_evidence_repository_replace_contract(&InMemoryEvidenceRepository::new())?;
    Ok(())
}

#[test]
fn in_memory_evidence_replace_overwrites_existing() -> Result<(), Box<dyn std::error::Error>> {
    let repo = InMemoryEvidenceRepository::new();
    let original = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "malformed".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    repo.put(original.clone())?;

    let replacement = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "corrected".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };

    // put rejects different content
    let Err(err) = repo.put(replacement.clone()) else {
        return Err("expected error".into());
    };
    assert!(matches!(err, PortError::Conflict { .. }));

    // replace succeeds with different content
    repo.replace(replacement.clone())?;

    let stored = repo
        .get(EvidenceId::new(300))?
        .ok_or_else(|| std::io::Error::other("replacement evidence missing"))?;
    assert_eq!(stored, replacement);
    Ok(())
}

#[test]
fn in_memory_evidence_put_rejects_conflicting_overwrite() -> Result<(), Box<dyn std::error::Error>>
{
    let repo = InMemoryEvidenceRepository::new();
    let first = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    repo.put(first.clone())?;

    let conflict = Evidence {
        id: EvidenceId::new(200), // same id
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2), // different report_id
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let Err(err) = repo.put(conflict) else {
        return Err("expected error".into());
    };
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    // Original is preserved
    let stored = repo
        .get(first.id)?
        .ok_or_else(|| std::io::Error::other("original evidence missing"))?;
    assert_eq!(stored, first);
    Ok(())
}

#[test]
fn in_memory_event_log_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_event_log_round_trip(&InMemoryEventLog::new())?;
    Ok(())
}

#[test]
fn in_memory_event_log_filters_task_artifact_events() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let task = DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::TaskOpened {
            task_id: maestria_domain::TaskId::new(1),
            title: "task".to_string(),
            priority: maestria_domain::TaskPriority::Normal,
            artifact_id: Some(maestria_domain::ArtifactId::new(7)),
        },
    };
    log.append(task.clone())?;
    assert_eq!(
        log.scan(EventFilter {
            artifact_id: Some(maestria_domain::ArtifactId::new(7)),
        })?,
        vec![task]
    );
    Ok(())
}

#[test]
fn in_memory_event_log_roundtrips_search_executed() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let envelope = DomainEventEnvelope {
        id: maestria_domain::EventId::new(1),
        sequence: maestria_domain::SequenceNumber::new(1),
        event: DomainEvent::SearchExecuted {
            query: "audit".to_string(),
            limit: 3,
            evidence_ids: vec![maestria_domain::EvidenceId::new(5)],
            pack_metadata: None,
            at: maestria_domain::LogicalTick::new(2),
        },
    };
    log.append(envelope.clone())?;
    // Full scan must return the event.
    assert_eq!(
        log.scan(EventFilter { artifact_id: None })?,
        vec![envelope.clone()]
    );
    // Artifact-filtered scan must exclude SearchExecuted (no artifact_id field).
    assert!(
        log.scan(EventFilter {
            artifact_id: Some(maestria_domain::ArtifactId::new(1)),
        })?
        .is_empty()
    );
    Ok(())
}

#[test]
fn in_memory_blob_store_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_blob_store_round_trip(&InMemoryBlobStore::new())?;
    Ok(())
}

#[test]
fn in_memory_full_text_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_full_text_index_round_trip(&InMemoryFullTextIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_vector_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_vector_index_contract(&InMemoryVectorIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_parser_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_parser_round_trip(
        &InMemoryParser::new(),
        &FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"alpha".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[tokio::test]
async fn in_memory_harness_adapter_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_harness_adapter_round_trip(&InMemoryHarnessAdapter::new()).await?;
    Ok(())
}

#[test]
fn in_memory_graph_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_graph_index_contract(&InMemoryGraphIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_graph_index_clear_removes_all_relations() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    index.insert_relation(rel.clone())?;
    assert_eq!(index.get_relations_for(ep)?.len(), 1);

    index.clear()?;
    assert!(index.get_relations_for(ep)?.is_empty());
    Ok(())
}

#[test]
fn in_memory_graph_index_delete_relations_ignores_empty_list()
-> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    index.insert_relation(rel.clone())?;

    index.delete_relations(&[])?;
    assert_eq!(index.get_relations_for(ep)?.len(), 1);
    Ok(())
}

#[test]
fn in_memory_graph_index_rebuild_preserves_new_relations() -> Result<(), Box<dyn std::error::Error>>
{
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel1 = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel2 = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(2),
        source: ep,
        target: RelationEndpoint::Claim(maestria_domain::ClaimId::new(4)),
        kind: maestria_domain::RelationKind::Supports,
        evidence_id: Some(maestria_domain::EvidenceId::new(5)),
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };

    index.insert_relation(rel1.clone())?;
    assert_eq!(index.get_relations_for(ep)?.len(), 1);

    index.rebuild(vec![rel2.clone()])?;

    let current = index.get_relations_for(ep)?;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0], rel2);
    Ok(())
}
