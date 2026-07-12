use super::contract_tests::*;
use super::*;
use maestria_domain::{EvidenceKind, LogicalTick, ValidationReportId};

#[test]
fn in_memory_artifact_repository_satisfies_contract() {
    assert_artifact_repository_round_trip(&InMemoryArtifactRepository::new());
}

#[test]
fn in_memory_chunk_repository_satisfies_contract() {
    assert_chunk_repository_round_trip(&InMemoryChunkRepository::new());
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

    let missing_res = fetcher.fetch("https://example.com/not-found-anywhere");
    assert!(
        matches!(missing_res, Err(PortError::NotFound)),
        "Missing URLs must map to PortError::NotFound, got {:?}",
        missing_res
    );

    Ok(())
}

#[test]
fn in_memory_card_repository_satisfies_contract() {
    assert_card_repository_round_trip(&InMemoryCardRepository::new());
}

#[test]
fn in_memory_evidence_repository_satisfies_contract() {
    assert_evidence_repository_round_trip(&InMemoryEvidenceRepository::new());
}

#[test]
fn in_memory_evidence_put_is_idempotent() {
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
    };
    // First insert succeeds
    repo.put(evidence.clone()).expect("first put must succeed");
    // Identical retry is idempotent
    repo.put(evidence.clone())
        .expect("identical retry must succeed");
    // Stored value is unchanged
    let stored = repo
        .get(evidence.id)
        .expect("get after retry")
        .expect("evidence must still exist");
    assert_eq!(stored, evidence);
}

#[test]
fn in_memory_evidence_repository_satisfies_replace_contract() {
    assert_evidence_repository_replace_contract(&InMemoryEvidenceRepository::new());
}

#[test]
fn in_memory_evidence_replace_overwrites_existing() {
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
    };
    repo.put(original.clone()).expect("first put");

    let replacement = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "corrected".to_string(),
        observed_at: LogicalTick::new(2),
    };

    // put rejects different content
    let err = repo.put(replacement.clone()).unwrap_err();
    assert!(matches!(err, PortError::Conflict { .. }));

    // replace succeeds with different content
    repo.replace(replacement.clone())
        .expect("replace must overwrite");

    let stored = repo
        .get(EvidenceId::new(300))
        .expect("get after replace")
        .expect("evidence must exist");
    assert_eq!(stored, replacement);
}

#[test]
fn in_memory_evidence_put_rejects_conflicting_overwrite() {
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
    };
    repo.put(first.clone()).expect("first put must succeed");

    let conflict = Evidence {
        id: EvidenceId::new(200), // same id
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2), // different report_id
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
    };
    let err = repo.put(conflict).unwrap_err();
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    // Original is preserved
    let stored = repo
        .get(first.id)
        .expect("get after conflict")
        .expect("evidence must still exist");
    assert_eq!(stored, first);
}

#[test]
fn in_memory_event_log_satisfies_contract() {
    assert_event_log_round_trip(&InMemoryEventLog::new());
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
fn in_memory_blob_store_satisfies_contract() {
    assert_blob_store_round_trip(&InMemoryBlobStore::new());
}

#[test]
fn in_memory_full_text_index_satisfies_contract() {
    assert_full_text_index_round_trip(&InMemoryFullTextIndex::new());
}

#[test]
fn in_memory_vector_index_satisfies_contract() {
    assert_vector_index_contract(&InMemoryVectorIndex::new());
}

#[test]
fn in_memory_parser_satisfies_contract() {
    assert_parser_round_trip(&InMemoryParser::new());
}

#[test]
fn in_memory_harness_adapter_satisfies_contract() {
    assert_harness_adapter_round_trip(&InMemoryHarnessAdapter::new());
}

#[test]
fn in_memory_graph_index_satisfies_contract() {
    assert_graph_index_contract(&InMemoryGraphIndex::new());
}
