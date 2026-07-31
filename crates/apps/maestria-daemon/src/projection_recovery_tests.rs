use std::fs;

use anyhow::Result;
use maestria_governance::AutonomyProfile;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;

use super::{reconcile_graph_projection, reconcile_projections, reconcile_vector_projection};
use crate::instance_setup::prepare_instance;
use crate::runtime_construction::build_runtime;
use maestria_domain::{
    ArtifactDetected, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ContentHash,
    ContentRange, CreateCardInput, DomainInput, EvidenceId, EvidenceKind, KernelState, LineRange,
    LogicalTick, ParseStatus, ParserResult, RecordEvidenceInput, RegisterChunkInput, SnapshotRef,
    SourceSpan, StructureNode, StructureNodeId, StructureNodeType,
};
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EmbeddingProvider, EmbeddingRequest,
    EmbeddingResponse, EventFilter, EvidenceRepository, GraphIndex, GraphRelationQuery, PortError,
    VectorEmbedding, VectorIndex, VectorSearchQuery,
};

fn search_budget(
    limit: u64,
) -> Result<maestria_domain::SearchExecutionBudget, maestria_domain::SearchCompatibilityError> {
    maestria_domain::SearchExecutionBudget::new(limit, 10_000, 100_000, 0)
}

/// Fixture carrying entity IDs produced during domain-state setup.
struct RecoveryTestFixture {
    artifact_id: ArtifactId,
    chunk_id_a: ChunkId,
    chunk_id_b: ChunkId,
    card_id: CardId,
    evidence_id: EvidenceId,
}

/// Build the domain-state snapshot that a crash-replay would reconstruct.
fn make_test_parser_result(
    artifact_id: ArtifactId,
    chunk_id_a: ChunkId,
    chunk_id_b: ChunkId,
    card_id: CardId,
) -> Result<ParserResult, Box<dyn std::error::Error>> {
    Ok(ParserResult {
        artifact_id,
        artifact_version_id: ArtifactVersionId::new(artifact_id.value()),
        content_hash: ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
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
    })
}

fn build_recovery_domain_state(
    state: &mut KernelState,
) -> Result<RecoveryTestFixture, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(1);
    let chunk_id_a = ChunkId::new(100);
    let chunk_id_b = ChunkId::new(101);
    let card_id = CardId::new(200);
    let evidence_id = EvidenceId::new(300);

    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "crash-test.md".to_string(),
        source_path: "/tmp/crash-test.md".to_string(),
        source_bytes: vec![4, 5, 6],
        content_hash: "sha256:fff".to_string(),
    }))?;

    let parser_result = make_test_parser_result(artifact_id, chunk_id_a, chunk_id_b, card_id)?;
    state.apply_input(DomainInput::ParserCompleted(parser_result))?;

    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/crash-test.md".to_string(),
            range: LineRange::new(1, 10)?,
            snapshot: SnapshotRef::new(
                BlobId::new(42),
                ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
            ),
        },
        excerpt: "first chu".to_string(),
        observed_at: LogicalTick::new(7),
        security: None,
    }))?;

    Ok(RecoveryTestFixture {
        artifact_id,
        chunk_id_a,
        chunk_id_b,
        card_id,
        evidence_id,
    })
}

struct RecoveryEmbeddingProvider;

impl EmbeddingProvider for RecoveryEmbeddingProvider {
    fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
        maestria_ports::ProviderDisclosure {
            remote: false,
            retention: maestria_ports::RetentionPolicy::NoRetention,
        }
    }
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
            identity: request.identity,
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        })
    }

    fn identity(&self) -> Option<maestria_ports::EmbeddingIdentity> {
        maestria_ports::EmbeddingIdentity::legacy("recovery-model", 2).ok()
    }
}

/// Assert that every projection repository reports absence for the given
/// entity ids (pre-reconcile guard).
fn assert_projections_absent(
    store: &SqliteStore,
    f: &RecoveryTestFixture,
) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        ArtifactRepository::get(store, f.artifact_id)?,
        None,
        "artifact should be absent before reconcile"
    );
    assert_eq!(
        ChunkRepository::get(store, f.chunk_id_a)?,
        None,
        "chunk should be absent before reconcile"
    );
    assert_eq!(
        CardRepository::get(store, f.card_id)?,
        None,
        "card should be absent before reconcile"
    );
    assert_eq!(
        EvidenceRepository::get(store, f.evidence_id)?,
        None,
        "evidence should be absent before reconcile"
    );
    Ok(())
}
/// Simulate crash recovery: events are intact but projection rows are
/// missing.  `reconcile_projections` rebuilds them from the replayed
/// `KernelState` without appending events.
#[test]
fn reconcile_projections_repairs_missing_rows() -> Result<(), Box<dyn std::error::Error>> {
    // ---- build domain truth from inputs (same as a replay would) ----
    let mut state = KernelState::new();
    let f = build_recovery_domain_state(&mut state)?;

    assert_eq!(state.artifacts.len(), 1);
    assert_eq!(state.chunks.len(), 2);
    assert_eq!(state.cards.len(), 1);
    assert_eq!(state.evidences.len(), 1);

    // ---- projection store starts empty ----
    let store = SqliteStore::in_memory()?;
    assert_projections_absent(&store, &f)?;

    // ---- reconcile ----
    reconcile_projections(&state, &store)?;

    // ---- all rows are now present ----
    let artifact = ArtifactRepository::get(&store, f.artifact_id)?
        .ok_or_else(|| std::io::Error::other("artifact projection missing"))?;
    assert_eq!(artifact.id, f.artifact_id);
    assert_eq!(artifact.title, "crash-test.md");
    assert_eq!(artifact.chunk_ids.len(), 2);
    assert_eq!(artifact.card_ids.len(), 1);
    assert_eq!(artifact.evidence_ids.len(), 1);

    let chunk = ChunkRepository::get(&store, f.chunk_id_a)?
        .ok_or_else(|| std::io::Error::other("chunk projection missing"))?;
    assert_eq!(chunk.id, f.chunk_id_a);
    assert_eq!(chunk.text, "first chunk");

    let card = CardRepository::get(&store, f.card_id)?
        .ok_or_else(|| std::io::Error::other("card projection missing"))?;
    assert_eq!(card.id, f.card_id);
    assert_eq!(card.title, "test card");

    let evidence = EvidenceRepository::get(&store, f.evidence_id)?
        .ok_or_else(|| std::io::Error::other("evidence projection missing"))?;
    assert_eq!(evidence.id, f.evidence_id);
    assert_eq!(evidence.excerpt, "first chu");

    // ---- idempotence: reconcile again without error / change ----
    reconcile_projections(&state, &store)?;

    // Rows unchanged after idempotent reconciliation.
    let artifact2 = ArtifactRepository::get(&store, f.artifact_id)?
        .ok_or_else(|| std::io::Error::other("artifact projection missing after reconcile"))?;
    assert_eq!(artifact2.title, "crash-test.md");

    let chunk2 = ChunkRepository::get(&store, f.chunk_id_b)?
        .ok_or_else(|| std::io::Error::other("second chunk projection missing"))?;
    assert_eq!(chunk2.text, "second chunk");
    Ok(())
}

/// Reconciliation is an exact child projection rebuild: rows seeded from a
/// previous state disappear when replayed state no longer contains them,
/// while still-valid rows survive.
#[test]
fn reconcile_projections_removes_stale_children_and_preserves_valid_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state)?;
    let store = SqliteStore::in_memory()?;

    // Seed the projection with the complete previous state. The second chunk,
    // card, and evidence become stale in the corrected replay below.
    for chunk in state.chunks.values() {
        ChunkRepository::put(&store, chunk.clone())?;
    }
    for card in state.cards.values() {
        CardRepository::put(&store, card.clone())?;
    }
    for evidence in state.evidences.values() {
        EvidenceRepository::put(&store, evidence.clone())?;
    }

    let mut corrected_state = state.clone();
    corrected_state.chunks.remove(&fixture.chunk_id_b);
    corrected_state.chunk_nodes.remove(&fixture.chunk_id_b);
    corrected_state.cards.clear();
    corrected_state.evidences.clear();
    let artifact = corrected_state
        .artifacts
        .get_mut(&fixture.artifact_id)
        .ok_or_else(|| std::io::Error::other("replay artifact missing"))?;
    artifact.chunk_ids.remove(&fixture.chunk_id_b);
    artifact.card_ids.clear();
    artifact.evidence_ids.clear();

    reconcile_projections(&corrected_state, &store)?;
    let reconciled_artifact = ArtifactRepository::get(&store, fixture.artifact_id)?
        .ok_or_else(|| std::io::Error::other("reconciled artifact projection missing"))?;
    assert_eq!(
        reconciled_artifact.chunk_ids,
        corrected_state
            .artifacts
            .get(&fixture.artifact_id)
            .ok_or_else(|| std::io::Error::other("corrected artifact missing"))?
            .chunk_ids
    );
    assert!(reconciled_artifact.card_ids.is_empty());
    assert!(reconciled_artifact.evidence_ids.is_empty());

    let chunks = ChunkRepository::list_for_artifact(&store, fixture.artifact_id)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].id, fixture.chunk_id_a);
    assert!(
        ChunkRepository::get(&store, fixture.chunk_id_b)?.is_none(),
        "chunk absent from replayed state must be removed"
    );
    assert!(
        CardRepository::list_for_artifact(&store, fixture.artifact_id)?.is_empty(),
        "stale card must be absent from artifact listing"
    );
    assert!(
        CardRepository::get(&store, fixture.card_id)?.is_none(),
        "card absent from replayed state must be removed"
    );
    assert!(
        EvidenceRepository::list_for_artifact(&store, fixture.artifact_id)?.is_empty(),
        "stale evidence must be absent from artifact listing"
    );
    assert!(
        EvidenceRepository::get(&store, fixture.evidence_id)?.is_none(),
        "evidence absent from replayed state must be removed"
    );

    // A second rebuild must remain idempotent and keep the valid chunk.
    reconcile_projections(&corrected_state, &store)?;
    assert_eq!(
        ChunkRepository::get(&store, fixture.chunk_id_a)?
            .ok_or_else(|| std::io::Error::other("valid chunk was removed"))?
            .text,
        "first chunk"
    );
    Ok(())
}

/// Reconciliation removes stale artifact parents and their child mappings when
/// replayed state no longer contains the artifact.
#[test]
fn reconcile_projections_removes_stale_artifact_parent_and_mappings()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state)?;
    let store = SqliteStore::in_memory()?;

    reconcile_projections(&state, &store)?;
    let seeded = ArtifactRepository::get(&store, fixture.artifact_id)?
        .ok_or_else(|| std::io::Error::other("seeded artifact projection missing"))?;
    assert_eq!(seeded.chunk_ids.len(), 2);
    assert_eq!(seeded.card_ids.len(), 1);
    assert_eq!(seeded.evidence_ids.len(), 1);

    let replayed_without_artifact = KernelState::new();
    reconcile_projections(&replayed_without_artifact, &store)?;

    assert!(
        ArtifactRepository::get(&store, fixture.artifact_id)?.is_none(),
        "artifact absent from replayed state must be deleted"
    );
    assert!(
        ChunkRepository::list_for_artifact(&store, fixture.artifact_id)?.is_empty(),
        "artifact chunk mappings must be absent after parent deletion"
    );
    assert!(
        CardRepository::list_for_artifact(&store, fixture.artifact_id)?.is_empty(),
        "artifact card mappings must be absent after parent deletion"
    );
    assert!(
        EvidenceRepository::list_for_artifact(&store, fixture.artifact_id)?.is_empty(),
        "artifact evidence mappings must be absent after parent deletion"
    );

    // A second replay of the same empty state must remain idempotent.
    reconcile_projections(&replayed_without_artifact, &store)?;
    assert!(ArtifactRepository::get(&store, fixture.artifact_id)?.is_none());
    Ok(())
}

/// Projection repair only writes the four projection entity types;
/// it never appends domain events.
#[test]
fn reconcile_projections_does_not_emit_events() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(42);
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "no-events.md".to_string(),
        source_path: "/tmp/no-events.md".to_string(),
        source_bytes: vec![7, 8, 9],
        content_hash: "sha256:eee".to_string(),
    }))?;

    let store = SqliteStore::in_memory()?;
    let event_count_before =
        maestria_ports::EventLog::scan(&store, EventFilter { artifact_id: None })?.len();

    reconcile_projections(&state, &store)?;

    let event_count_after =
        maestria_ports::EventLog::scan(&store, EventFilter { artifact_id: None })?.len();
    assert_eq!(
        event_count_after, event_count_before,
        "reconcile_projections must not append domain events"
    );
    Ok(())
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
fn reconcile_projections_evidence_replace_overwrites_stale_row()
-> Result<(), Box<dyn std::error::Error>> {
    // Build state with one evidence row.
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(10);
    let evidence_id = EvidenceId::new(400);

    // Register the artifact so the state is consistent.
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "replace-test.md".to_string(),
        source_path: "/tmp/replace-test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:rrr".to_string(),
    }))?;

    let stale_evidence = maestria_domain::Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/replace-test.md".to_string(),
            range: LineRange::new(1, 5)?,
            snapshot: SnapshotRef::new(
                BlobId::new(42),
                ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
            ),
        },
        excerpt: "stale excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };

    // Directly insert into state (bypass domain validation for the stale row).
    state.evidences.insert(evidence_id, stale_evidence.clone());

    let store = SqliteStore::in_memory()?;

    // First reconcile writes the stale evidence.
    reconcile_projections(&state, &store)?;
    let stored = EvidenceRepository::get(&store, evidence_id)?
        .ok_or_else(|| std::io::Error::other("stored evidence missing"))?;
    assert_eq!(stored.excerpt, "stale excerpt");

    // Now simulate a replay that corrects the evidence excerpt.
    let mut corrected_state = KernelState::new();
    corrected_state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title: "replace-test.md".to_string(),
        source_path: "/tmp/replace-test.md".to_string(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:rrr".to_string(),
    }))?;

    let corrected_evidence = maestria_domain::Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/tmp/replace-test.md".to_string(),
            range: LineRange::new(1, 5)?,
            snapshot: SnapshotRef::new(
                BlobId::new(42),
                ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
            ),
        },
        excerpt: "corrected excerpt".to_string(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };
    corrected_state
        .evidences
        .insert(evidence_id, corrected_evidence.clone());

    // Second reconcile must overwrite the stale row with the corrected one.
    reconcile_projections(&corrected_state, &store)?;

    let corrected = EvidenceRepository::get(&store, evidence_id)?
        .ok_or_else(|| std::io::Error::other("corrected evidence missing"))?;
    assert_eq!(
        corrected.excerpt, "corrected excerpt",
        "evidence replace must overwrite stale excerpt"
    );
    assert_eq!(
        corrected.observed_at,
        LogicalTick::new(2),
        "evidence replace must update observed_at"
    );
    Ok(())
}

#[test]
fn reconcile_graph_projection_repairs_missing_rows_and_filters_unevidenced()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state)?;
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

    let graph = maestria_graph_sqlite::SqliteGraphIndex::in_memory()?;
    graph.insert_relation(maestria_domain::Relation {
        id: maestria_domain::RelationId::new(99),
        source: maestria_domain::RelationEndpoint::Artifact(fixture.artifact_id),
        target: maestria_domain::RelationEndpoint::Claim(maestria_domain::ClaimId::new(11)),
        kind: maestria_domain::RelationKind::Supports,
        evidence_id: Some(fixture.evidence_id),
        confidence_milli: 1000,
        security: maestria_domain::SecurityMetadata::default(),
    })?;

    reconcile_graph_projection(&state, &graph)?;

    let query = GraphRelationQuery::new(
        maestria_domain::RelationEndpoint::Artifact(fixture.artifact_id),
        u64::MAX,
    )
    .ok_or("graph query limit must be positive")?;
    assert_eq!(graph.get_relations_for(query)?.relations, vec![valid]);
    Ok(())
}

#[test]
fn reconcile_vector_projection_repairs_missing_and_stale_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    let fixture = build_recovery_domain_state(&mut state)?;
    let vector_root =
        std::env::temp_dir().join(format!("maestria-vector-recovery-{}", std::process::id()));
    let _ = fs::remove_dir_all(&vector_root);
    fs::create_dir_all(&vector_root)?;
    let vector_path = vector_root.join("projection.db");
    let index = SqliteVectorIndex::open(&vector_path)?;

    index.index_embeddings(vec![VectorEmbedding {
        chunk_id: fixture.chunk_id_a,
        vector: vec![0.0, 1.0],
        provenance: maestria_ports::EmbeddingProvenance {
            content_hash: "stale".to_string(),
            identity: maestria_ports::EmbeddingIdentity::legacy("stale-model", 2)?,
            provider_id: "stale-provider".to_string(),
            model: "stale-model".to_string(),
            model_version: "stale-v1".to_string(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        },
    }])?;

    reconcile_vector_projection(
        &state,
        &index,
        Some(&RecoveryEmbeddingProvider),
        Some("recovery-model"),
    )?;

    let first_hits = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 1,
        execution_budget: search_budget(1)?,
        provider_id: Some("recovery-provider".to_string()),
        model: Some("recovery-model".to_string()),
        model_version: Some("recovery-v1".to_string()),
        identity: None,
    })?;
    assert_eq!(
        first_hits.hits.first().map(|hit| hit.chunk_id),
        Some(fixture.chunk_id_a),
        "recovery must replace stale provenance and preserve chunk identity"
    );

    let second_hits = index.search_similar(VectorSearchQuery {
        vector: vec![0.0, 1.0],
        limit: 1,
        execution_budget: search_budget(1)?,
        provider_id: Some("recovery-provider".to_string()),
        model: Some("recovery-model".to_string()),
        model_version: Some("recovery-v1".to_string()),
        identity: None,
    })?;
    assert_eq!(
        second_hits.hits.first().map(|hit| hit.chunk_id),
        Some(fixture.chunk_id_b),
        "recovery must rebuild chunks missing from the projection"
    );

    let stale_hits = index.search_similar(VectorSearchQuery {
        vector: vec![0.0, 1.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        provider_id: Some("stale-provider".to_string()),
        model: Some("stale-model".to_string()),
        model_version: Some("stale-v1".to_string()),
        identity: None,
    })?;
    assert!(
        stale_hits.hits.is_empty(),
        "rebuild must remove stale vector provenance"
    );

    drop(index);
    let restarted = SqliteVectorIndex::open(&vector_path)?;
    let restarted_hits = restarted.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 1,
        execution_budget: search_budget(1)?,
        provider_id: Some("recovery-provider".to_string()),
        model: Some("recovery-model".to_string()),
        model_version: Some("recovery-v1".to_string()),
        identity: None,
    })?;
    assert_eq!(
        restarted_hits.hits.first().map(|hit| hit.chunk_id),
        Some(fixture.chunk_id_a),
        "vector retrieval must remain stable after reopening the projection"
    );
    drop(restarted);
    let _ = fs::remove_dir_all(&vector_root);
    Ok(())
}

#[test]
fn build_runtime_fails_on_corrupt_vector_projection() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("maestria-corrupt-vector-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let layout = prepare_instance(root.clone())?;
    // Configure the dense capability so the corrupt projection is a configured
    // store: a disabled embedding profile must not open the vector projection
    // (mirroring the search runtime's provider-gated boundary).
    let mut manifest = maestria_core::InstanceManifest::default_for_root(layout.root.clone());
    manifest.embeddings = Some(maestria_core::EmbeddingConfig {
        enabled: true,
        endpoint: "http://127.0.0.1:9/v1/embeddings".to_string(),
        model: "fixture-embedding".to_string(),
        dimensions: 8,
        provider: "fixture-onnx".to_string(),
        revision: "fixture-v1".to_string(),
        artifact_hash: format!("sha256:{}", "0".repeat(64)),
        preprocessing_version: "fixture-v1".to_string(),
        remote_provider: false,
        retention_policy: maestria_ports::RetentionPolicy::NoRetention,
    });
    fs::write(&layout.manifest_path, manifest.encode())?;
    let mut state = crate::instance_setup::load_kernel_state(&layout)?;
    crate::vector_startup::reconcile_retrieval_generations(&layout, &mut state, &manifest)?;
    fs::write(
        layout.vector_index_dir.join("projection.db"),
        b"not a sqlite database",
    )?;

    let result = build_runtime(&layout, state, AutonomyProfile::ReadOnly);
    let Some(error) = result.err() else {
        let _ = fs::remove_dir_all(&root);
        return Err("corrupt vector projection must fail runtime startup"
            .to_string()
            .into());
    };
    let message = format!("{error:#}");
    assert!(
        message.contains("open vector index"),
        "startup error must preserve vector index context: {message}"
    );
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
