use super::contract_tests::*;
use super::*;

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
