use super::*;
use maestria_domain::{
    ArtifactDetected, CardId, ChunkId, ContentRange, CreateCardInput, EvidenceId, EvidenceKind,
    LogicalTick, ParserResult, RecordEvidenceInput, RegisterChunkInput,
};
use maestria_ports::{ArtifactRepository, CardRepository, ChunkRepository, EvidenceRepository};

/// Fixture carrying entity IDs produced during domain-state setup.
struct RecoveryTestFixture {
    artifact_id: ArtifactId,
    chunk_id_a: ChunkId,
    chunk_id_b: ChunkId,
    card_id: CardId,
    evidence_id: EvidenceId,
}

/// Build the domain-state snapshot that a crash-replay would reconstruct.
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
            chunks: vec![
                RegisterChunkInput {
                    chunk_id: chunk_id_a,
                    artifact_id,
                    order: 0,
                    text: "first chunk".to_string(),
                },
                RegisterChunkInput {
                    chunk_id: chunk_id_b,
                    artifact_id,
                    order: 1,
                    text: "second chunk".to_string(),
                },
            ],
            cards: vec![CreateCardInput {
                card_id,
                artifact_id,
                title: "test card".to_string(),
                body: "card body".to_string(),
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
