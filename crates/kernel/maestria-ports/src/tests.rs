use super::contract_tests::*;
use super::graph_contract_tests::assert_graph_index_contract;
use super::*;
use maestria_domain::{
    ArtifactId, BlobId, ChunkId, ContentRange, DomainEvent, DomainEventEnvelope, Evidence,
    EvidenceId, EvidenceKind, LogicalTick, RelationEndpoint, StructureNode, StructureNodeId,
    StructureNodeType, ValidationReportId,
};
use std::path::PathBuf;

fn structure_node(id: u64, parent_id: Option<u64>, sibling_id: Option<u64>) -> StructureNode {
    StructureNode {
        id: StructureNodeId::new(id),
        parent_id: parent_id.map(StructureNodeId::new),
        sibling_id: sibling_id.map(StructureNodeId::new),
        node_type: StructureNodeType::Document,
        source_range: ContentRange { start: 0, end: 0 },
        page: None,
        section_path: Vec::new(),
        parser_generation: "test".to_string(),
        schema_generation: "test".to_string(),
        language: None,
    }
}

#[test]
fn document_tree_rejects_invalid_topologies() -> Result<(), PortError> {
    let root_id = StructureNodeId::new(1);
    let root = structure_node(1, None, None);

    assert!(
        DocumentTree::new(root_id, vec![root.clone(), root])
            .is_err_and(|error| { error.is_invalid_input() })
    );
    assert!(
        DocumentTree::new(root_id, vec![structure_node(2, None, None)])
            .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, None),
                structure_node(2, Some(99), None)
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, None),
                structure_node(2, Some(3), None),
                structure_node(3, Some(2), None),
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    assert!(
        DocumentTree::new(
            root_id,
            vec![
                structure_node(1, None, Some(2)),
                structure_node(2, Some(1), Some(1)),
            ],
        )
        .is_err_and(|error| error.is_invalid_input())
    );
    Ok(())
}

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

    let zero_limit = fetcher.fetch("https://example.com/test", 0);
    assert!(zero_limit.is_err_and(|error| error.is_invalid_input()));
    let too_large = fetcher.fetch("https://example.com/test", 1);
    assert!(too_large.is_err_and(|error| error.is_invalid_input()));

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

fn artifact_filter_events(
    artifact_a: ArtifactId,
    artifact_b: ArtifactId,
) -> Vec<DomainEventEnvelope> {
    vec![
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(1),
            sequence: maestria_domain::SequenceNumber::new(1),
            event: DomainEvent::PendingIndex {
                artifact_id: artifact_a,
                content_hash: "hash-a".to_string(),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(2),
            sequence: maestria_domain::SequenceNumber::new(2),
            event: DomainEvent::FullTextIndexed {
                artifact_id: artifact_a,
                chunk_id: ChunkId::new(1),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(3),
            sequence: maestria_domain::SequenceNumber::new(3),
            event: DomainEvent::ArtifactIndexed {
                artifact_id: artifact_a,
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(4),
            sequence: maestria_domain::SequenceNumber::new(4),
            event: DomainEvent::ParserStarted {
                artifact_id: artifact_a,
                title: "doc".to_string(),
                source_path: "/a.md".to_string(),
                content_hash: "hash-a".to_string(),
                blob_id: BlobId::new(1),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(5),
            sequence: maestria_domain::SequenceNumber::new(5),
            event: DomainEvent::SourceBecameStale {
                artifact_id: artifact_a,
                source_path: "/a.md".to_string(),
                content_hash: "hash-a".to_string(),
            },
        },
        DomainEventEnvelope {
            id: maestria_domain::EventId::new(6),
            sequence: maestria_domain::SequenceNumber::new(6),
            event: DomainEvent::PendingIndex {
                artifact_id: artifact_b,
                content_hash: "hash-b".to_string(),
            },
        },
    ]
}

#[test]
fn in_memory_event_log_filters_all_artifact_variants() -> Result<(), PortError> {
    let log = InMemoryEventLog::new();
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);
    let events = artifact_filter_events(artifact_a, artifact_b);

    for event in &events {
        log.append(event.clone())?;
    }

    assert_eq!(
        log.scan(EventFilter { artifact_id: None })?,
        events,
        "global scan should return all events"
    );

    let filtered_a = log.scan(EventFilter {
        artifact_id: Some(artifact_a),
    })?;
    assert_eq!(filtered_a.len(), 5);
    for event in &filtered_a {
        match &event.event {
            DomainEvent::PendingIndex { artifact_id, .. }
            | DomainEvent::FullTextIndexed { artifact_id, .. }
            | DomainEvent::ArtifactIndexed { artifact_id }
            | DomainEvent::ParserStarted { artifact_id, .. }
            | DomainEvent::SourceBecameStale { artifact_id, .. } => {
                assert_eq!(*artifact_id, artifact_a);
            }
            other => {
                return Err(PortError::InternalContext {
                    context: "unexpected artifact-filtered event variant",
                    source: format!("{other:?}"),
                });
            }
        }
    }

    let filtered_b = log.scan(EventFilter {
        artifact_id: Some(artifact_b),
    })?;
    assert_eq!(filtered_b.len(), 1);
    assert!(matches!(
        filtered_b[0].event,
        DomainEvent::PendingIndex { artifact_id, .. } if artifact_id == artifact_b
    ));
    assert!(
        log.scan(EventFilter {
            artifact_id: Some(ArtifactId::new(99)),
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

#[test]
fn in_memory_parser_multiline_source_span() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let parsed = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"line one\nline two\nline three".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(1),
        },
    )?;
    assert_eq!(parsed.chunks.len(), 1);
    match parsed.chunks[0].source_span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => {
            assert_eq!(start_line, 1);
            assert_eq!(end_line, 3, "expected end_line == 3 for three-line input");
        }
        _ => return Err("expected TextSpan".into()),
    }
    Ok(())
}

#[test]
fn in_memory_parser_version_id_changes_with_content() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let artifact_id = ArtifactId::new(42);
    let first = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"first draft".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    let second = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"second draft".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    assert_ne!(
        first.artifact_version_id, second.artifact_version_id,
        "same artifact with different bytes must yield different version ids"
    );
    Ok(())
}

#[test]
fn in_memory_parser_version_id_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let artifact_id = ArtifactId::new(42);
    let first = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"stable content".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    let second = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"stable content".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    assert_eq!(
        first.artifact_version_id, second.artifact_version_id,
        "same artifact with identical bytes must yield identical version ids"
    );
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
