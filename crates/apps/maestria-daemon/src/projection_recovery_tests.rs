#![allow(clippy::disallowed_methods)]

use super::*;
use maestria_domain::{
    ArtifactDetected, ArtifactVersionId, CardId, ChunkId, ContentHash, ContentRange,
    CreateCardInput, EvidenceId, EvidenceKind, LogicalTick, ParseStatus, ParserResult,
    RecordEvidenceInput, RegisterChunkInput, SourceSpan, StructureNode, StructureNodeId,
    StructureNodeType,
};
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EmbeddingProvider, EmbeddingRequest,
    EmbeddingResponse, EvidenceRepository, GraphIndex, PortError, VectorEmbedding, VectorIndex,
    VectorSearchQuery,
};

/// Fixture carrying entity IDs produced during domain-state setup.
struct RecoveryTestFixture {
    artifact_id: ArtifactId,
    chunk_id_a: ChunkId,
    chunk_id_b: ChunkId,
    card_id: CardId,
    evidence_id: EvidenceId,
}

/// Build the domain-state snapshot that a crash-replay would reconstruct.
#[allow(clippy::too_many_lines)]
fn build_recovery_domain_state(state: &mut KernelState) -> RecoveryTestFixture {
    let artifact_id = ArtifactId::new(1);
    let chunk_id_a = ChunkId::new(100);
    let chunk_id_b = ChunkId::new(101);
    let card_id = CardId::new(200);
    let evidence_id = EvidenceId::new(300);

    state
        .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "crash-test.md".to_string(),
            source_path: "/tmp/crash-test.md".to_string(),
            source_bytes: vec![4, 5, 6],
            content_hash: "sha256:fff".to_string(),
        }))
        .expect("register artifact");

    // ParserCompleted creates both chunks and cards in one input.
    state
        .apply_input(DomainInput::ParserCompleted(ParserResult {
            artifact_id,
            artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
            content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64)).unwrap(),
            status: ParseStatus::Parsed,
            tree_root_id: Some(StructureNodeId::new(chunk_id_a.value())),
            tree_nodes: vec![
                StructureNode {
                    id: StructureNodeId::new(chunk_id_a.value()),
                    parent_id: None,
                    sibling_id: None,
                    node_type: StructureNodeType::Document,
                    source_range: ContentRange { start: 0, end: 0 },
                    page: None,
                    section_path: vec![],
                    parser_generation: "test".to_string(),
                    schema_generation: "1".to_string(),
                    language: None,
                },
                StructureNode {
                    id: StructureNodeId::new(chunk_id_b.value()),
                    parent_id: Some(StructureNodeId::new(chunk_id_a.value())),
                    sibling_id: None,
                    node_type: StructureNodeType::Paragraph,
                    source_range: ContentRange { start: 0, end: 0 },
                    page: None,
                    section_path: vec![],
                    parser_generation: "test".to_string(),
                    schema_generation: "1".to_string(),
                    language: None,
                },
            ],
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: chunk_id_a,
                    artifact_id,
                    node_id: StructureNodeId::new(chunk_id_a.value()),
                    source_span: SourceSpan::TextSpan {
                        start_line: 1,
                        end_line: 1,
                    },
                    representations: vec![],
                    order: 0,
                    text: "first chunk".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: chunk_id_b,
                    artifact_id,
                    node_id: StructureNodeId::new(chunk_id_b.value()),
                    source_span: SourceSpan::TextSpan {
                        start_line: 1,
                        end_line: 1,
                    },
                    representations: vec![],
                    order: 1,
                    text: "second chunk".to_string(),
                },
            ],
            cards: vec![CreateCardInput {
                card_id,
                artifact_id,
                node_id: StructureNodeId::new(chunk_id_a.value()),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 1,
                },
                title: "test card".to_string(),
                body: "card body".to_string(),
                security: None,
            }],
        }))
        .expect("parser completed");

    state
        .apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
            evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/tmp/crash-test.md".to_string(),
                range: ContentRange { start: 0, end: 10 },
                content_hash: "sha256:fff".to_string(),
                snapshot: None,
            },
            excerpt: "first chu".to_string(),
            observed_at: LogicalTick::new(7),
            security: None,
        }))
        .expect("record evidence");

    RecoveryTestFixture {
        artifact_id,
        chunk_id_a,
        chunk_id_b,
        card_id,
        evidence_id,
    }
}

struct RecoveryEmbeddingProvider;

impl EmbeddingProvider for RecoveryEmbeddingProvider {
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        let vector = if request.text.contains("first") {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        };
        Ok(EmbeddingResponse {
            vector,
            provider_id: "recovery-provider".to_string(),
            model: request.model,
            model_version: "recovery-v1".to_string(),
        })
    }
}

/// Assert that every projection repository reports absence for the given
/// entity ids (pre-reconcile guard).
fn assert_projections_absent(store: &SqliteStore, f: &RecoveryTestFixture) {
    assert_eq!(
        ArtifactRepository::get(store, f.artifact_id).expect("get artifact"),
        None,
        "artifact should be absent before reconcile"
    );
    assert_eq!(
        ChunkRepository::get(store, f.chunk_id_a).expect("get chunk"),
        None,
        "chunk should be absent before reconcile"
    );
    assert_eq!(
        CardRepository::get(store, f.card_id).expect("get card"),
        None,
        "card should be absent before reconcile"
    );
    assert_eq!(
        EvidenceRepository::get(store, f.evidence_id).expect("get evidence"),
        None,
        "evidence should be absent before reconcile"
    );
}
/// Simulate crash recovery: events are intact but projection rows are
/// missing.  `reconcile_projections` rebuilds them from the replayed
/// `KernelState` without appending events.
#[test]
fn reconcile_projections_repairs_missing_rows() {
    // ---- build domain truth from inputs (same as a replay would) ----
    let mut state = KernelState::new();
    let f = build_recovery_domain_state(&mut state);

    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(state.chunks.len(), 2);
    assert_eq!(state.cards.len(), 1);
    assert_eq!(state.evidences.len(), 1);

    // ---- projection store starts empty ----
    let store = SqliteStore::in_memory().expect("open in-memory store");
    assert_projections_absent(&store, &f);

    // ---- reconcile ----
    reconcile_projections(&state, &store).expect("reconcile should succeed");

    // ---- all rows are now present ----
    let artifact = ArtifactRepository::get(&store, f.artifact_id)
        .expect("get artifact after reconcile")
        .expect("artifact should exist after reconcile");
    assert_eq!(artifact.id, f.artifact_id);
    assert_eq!(artifact.title, "crash-test.md");
    assert_eq!(artifact.chunk_ids.len(), 2);
    assert_eq!(artifact.card_ids.len(), 1);
    assert_eq!(artifact.evidence_ids.len(), 1);

    let chunk = ChunkRepository::get(&store, f.chunk_id_a)
        .expect("get chunk after reconcile")
        .expect("chunk should exist after reconcile");
    assert_eq!(chunk.id, f.chunk_id_a);
    assert_eq!(chunk.text, "first chunk");

    let card = CardRepository::get(&store, f.card_id)
        .expect("get card after reconcile")
        .expect("card should exist after reconcile");
    assert_eq!(card.id, f.card_id);
    assert_eq!(card.title, "test card");

    let evidence = EvidenceRepository::get(&store, f.evidence_id)
        .expect("get evidence after reconcile")
        .expect("evidence should exist after reconcile");
    assert_eq!(evidence.id, f.evidence_id);
    assert_eq!(evidence.excerpt, "first chu");

    // ---- idempotence: reconcile again without error / change ----
    reconcile_projections(&state, &store).expect("second reconcile should succeed");

    // Rows unchanged after idempotent reconciliation.
    let artifact2 = ArtifactRepository::get(&store, f.artifact_id)
        .expect("get artifact after second reconcile")
        .expect("artifact should still exist");
    assert_eq!(artifact2.title, "crash-test.md");

    let chunk2 = ChunkRepository::get(&store, f.chunk_id_b)
        .expect("get chunk after second reconcile")
        .expect("chunk should still exist");
    assert_eq!(chunk2.text, "second chunk");
}

/// Projection repair only writes the four projection entity types;
/// it never appends domain events.
#[test]
fn reconcile_projections_does_not_emit_events() {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(42);
    state
        .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "no-events.md".to_string(),
            source_path: "/tmp/no-events.md".to_string(),
            source_bytes: vec![7, 8, 9],
            content_hash: "sha256:eee".to_string(),
        }))
        .expect("register artifact");

    let store = SqliteStore::in_memory().expect("open in-memory store");
    let event_count_before =
        maestria_ports::EventLog::scan(&store, EventFilter { artifact_id: None })
            .expect("scan")
            .len();

    reconcile_projections(&state, &store).expect("reconcile should succeed");

    let event_count_after =
        maestria_ports::EventLog::scan(&store, EventFilter { artifact_id: None })
            .expect("scan")
            .len();
    assert_eq!(
        event_count_after, event_count_before,
        "reconcile_projections must not append domain events"
    );
}

/// Evidence `replace` overwrites a stale or malformed row instead of
/// failing with a `Conflict` error.  This guards against the case where a
/// previous crash left a partial evidence row whose fields differ from
/// the replayed domain truth; `put` would reject the mismatch as a
/// conflict, but `replace` corrects the row unconditionally.
///
/// The test directly exercises the store-level `replace` contract and
/// then verifies that `reconcile_projections` uses it to overwrite.
#[test]
fn reconcile_projections_evidence_replace_overwrites_stale_row() {
    // Build state with one evidence row.
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(10);
    let evidence_id = EvidenceId::new(400);

    // Register the artifact so the state is consistent.
    state
        .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "replace-test.md".to_string(),
            source_path: "/tmp/replace-test.md".to_string(),
            source_bytes: vec![1, 2, 3],
            content_hash: "sha256:rrr".to_string(),
        }))
        .expect("register artifact");

    let stale_evidence = maestria_domain::Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/replace-test.md".to_string(),
            range: ContentRange { start: 0, end: 5 },
            content_hash: "sha256:rrr".to_string(),
            snapshot: None,
        },
        excerpt: "stale excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };

    // Directly insert into state (bypass domain validation for the stale row).
    state.evidences.insert(evidence_id, stale_evidence.clone());

    let store = SqliteStore::in_memory().expect("open in-memory store");

    // First reconcile writes the stale evidence.
    reconcile_projections(&state, &store).expect("first reconcile");
    let stored = EvidenceRepository::get(&store, evidence_id)
        .expect("get evidence")
        .expect("evidence should exist");
    assert_eq!(stored.excerpt, "stale excerpt");

    // Now simulate a replay that corrects the evidence excerpt.
    let mut corrected_state = KernelState::new();
    corrected_state
        .apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "replace-test.md".to_string(),
            source_path: "/tmp/replace-test.md".to_string(),
            source_bytes: vec![1, 2, 3],
            content_hash: "sha256:rrr".to_string(),
        }))
        .expect("register artifact again");

    let corrected_evidence = maestria_domain::Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/replace-test.md".to_string(),
            range: ContentRange { start: 0, end: 5 },
            content_hash: "sha256:rrr".to_string(),
            snapshot: None,
        },
        excerpt: "corrected excerpt".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };
    corrected_state
        .evidences
        .insert(evidence_id, corrected_evidence.clone());

    // Second reconcile must overwrite the stale row with the corrected one.
    reconcile_projections(&corrected_state, &store).expect("second reconcile");

    let corrected = EvidenceRepository::get(&store, evidence_id)
        .expect("get evidence after replace")
        .expect("evidence should still exist after replace");
    assert_eq!(
        corrected.excerpt, "corrected excerpt",
        "evidence replace must overwrite stale excerpt"
    );
    assert_eq!(
        corrected.observed_at,
        LogicalTick::new(2),
        "evidence replace must update observed_at"
    );
}

#[test]
fn reconcile_graph_projection_repairs_missing_rows_and_filters_unevidenced() {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state);
    let valid = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: maestria_domain::RelationEndpoint::Artifact(fixture.artifact_id),
        target: maestria_domain::RelationEndpoint::Claim(maestria_domain::ClaimId::new(9)),
        kind: maestria_domain::RelationKind::Supports,
        evidence_id: Some(fixture.evidence_id),
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let unevidenced = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(2),
        source: maestria_domain::RelationEndpoint::Artifact(fixture.artifact_id),
        target: maestria_domain::RelationEndpoint::Claim(maestria_domain::ClaimId::new(10)),
        kind: maestria_domain::RelationKind::Supports,
        evidence_id: None,
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };
    state.relations.insert(valid.id, valid.clone());
    state.relations.insert(unevidenced.id, unevidenced);

    let graph = maestria_graph_sqlite::SqliteGraphIndex::in_memory().expect("open graph index");
    graph
        .insert_relation(maestria_domain::Relation {
            id: maestria_domain::RelationId::new(99),
            source: maestria_domain::RelationEndpoint::Artifact(fixture.artifact_id),
            target: maestria_domain::RelationEndpoint::Claim(maestria_domain::ClaimId::new(11)),
            kind: maestria_domain::RelationKind::Supports,
            evidence_id: Some(fixture.evidence_id),
            confidence_milli: 1000,
            security: maestria_domain::SecurityMetadata::default(),
        })
        .expect("seed stale graph relation");

    reconcile_graph_projection(&state, &graph).expect("reconcile graph");

    assert_eq!(
        graph
            .get_relations_for(maestria_domain::RelationEndpoint::Artifact(
                fixture.artifact_id
            ))
            .expect("read graph relations"),
        vec![valid]
    );
}

#[test]
fn reconcile_vector_projection_repairs_missing_and_stale_rows() {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state);
    let vector_root =
        std::env::temp_dir().join(format!("maestria-vector-recovery-{}", std::process::id()));
    let _ = fs::remove_dir_all(&vector_root);
    fs::create_dir_all(&vector_root).expect("create vector projection directory");
    let vector_path = vector_root.join("projection.db");
    let index = SqliteVectorIndex::open(&vector_path).expect("open vector index");

    index
        .index_embeddings(vec![VectorEmbedding {
            chunk_id: fixture.chunk_id_a,
            vector: vec![0.0, 1.0],
            provenance: maestria_ports::EmbeddingProvenance {
                content_hash: "stale".to_string(),
                provider_id: "stale-provider".to_string(),
                model: "stale-model".to_string(),
                model_version: "stale-v1".to_string(),
            },
        }])
        .expect("seed stale embedding");

    reconcile_vector_projection(
        &state,
        &index,
        Some(&RecoveryEmbeddingProvider),
        Some("recovery-model"),
    )
    .expect("reconcile vector projection");

    let first_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![1.0, 0.0],
            limit: 1,
            provider_id: Some("recovery-provider".to_string()),
            model: Some("recovery-model".to_string()),
            model_version: Some("recovery-v1".to_string()),
        })
        .expect("search first embedding");
    assert_eq!(
        first_hits
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>(),
        vec![fixture.chunk_id_a],
        "recovery must replace stale provenance and preserve chunk identity"
    );

    let second_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![0.0, 1.0],
            limit: 1,
            provider_id: Some("recovery-provider".to_string()),
            model: Some("recovery-model".to_string()),
            model_version: Some("recovery-v1".to_string()),
        })
        .expect("search second embedding");
    assert_eq!(
        second_hits
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>(),
        vec![fixture.chunk_id_b],
        "recovery must rebuild chunks missing from the projection"
    );

    let stale_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![0.0, 1.0],
            limit: 10,
            provider_id: Some("stale-provider".to_string()),
            model: Some("stale-model".to_string()),
            model_version: Some("stale-v1".to_string()),
        })
        .expect("search stale provenance");
    assert!(
        stale_hits.is_empty(),
        "rebuild must remove stale vector provenance"
    );

    drop(index);
    let restarted = SqliteVectorIndex::open(&vector_path).expect("reopen vector index");
    let restarted_hits = restarted
        .search_similar(VectorSearchQuery {
            vector: vec![1.0, 0.0],
            limit: 1,
            provider_id: Some("recovery-provider".to_string()),
            model: Some("recovery-model".to_string()),
            model_version: Some("recovery-v1".to_string()),
        })
        .expect("search after restart");
    assert_eq!(
        restarted_hits
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>(),
        vec![fixture.chunk_id_a],
        "vector retrieval must remain stable after reopening the projection"
    );
    drop(restarted);
    let _ = fs::remove_dir_all(&vector_root);
}

#[test]
fn build_runtime_fails_on_corrupt_vector_projection() {
    let root = std::env::temp_dir().join(format!("maestria-corrupt-vector-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let layout = prepare_instance(root.clone()).expect("prepare instance");
    fs::write(
        layout.vector_index_dir.join("projection.db"),
        b"not a sqlite database",
    )
    .expect("write corrupt vector projection");

    let result = build_runtime(&layout, KernelState::new(), AutonomyProfile::ReadOnly);
    let Some(error) = result.err() else {
        let _ = fs::remove_dir_all(&root);
        panic!("corrupt vector projection must fail runtime startup");
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("open vector index"),
        "startup error must preserve vector index context: {message}"
    );
    let _ = fs::remove_dir_all(&root);
}
