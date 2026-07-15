use crate::SqliteStore;
use maestria_domain::*;
use maestria_ports::*;

use super::artifact;

#[test]
fn artifact_put_get_and_missing() {
    let store = SqliteStore::in_memory().expect("test setup");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(9)).expect("missing artifact lookup"),
        None
    );

    let artifact = artifact(1);
    ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
        Some(artifact)
    );
}

#[test]
fn artifact_relationship_sets_round_trip() {
    let store = SqliteStore::in_memory().expect("test setup");
    let mut artifact = artifact(1);
    artifact
        .chunk_ids
        .extend([ChunkId::new(10), ChunkId::new(11)]);
    artifact.card_ids.extend([CardId::new(20), CardId::new(21)]);
    artifact
        .claim_ids
        .extend([ClaimId::new(30), ClaimId::new(31)]);
    artifact
        .evidence_ids
        .extend([EvidenceId::new(40), EvidenceId::new(41)]);

    ArtifactRepository::put(&store, artifact.clone()).expect("test setup");

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("stored artifact lookup"),
        Some(artifact)
    );
}

#[test]
fn brain_state_round_trips_and_lists_deterministically() {
    let store = SqliteStore::in_memory().expect("test setup");
    let late = Chunk {
        id: ChunkId::new(10),
        artifact_id: ArtifactId::new(1),
        order: 2,
        text: "late".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        representations: vec![],
    };
    let early = Chunk {
        id: ChunkId::new(11),
        artifact_id: ArtifactId::new(1),
        order: 1,
        text: "early".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        representations: vec![],
    };
    let card = Card {
        id: CardId::new(20),
        artifact_id: ArtifactId::new(1),
        title: "card".to_string(),
        body: "body".to_string(),
        node_id: maestria_domain::StructureNodeId::new(0),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        claim_ids: [ClaimId::new(5), ClaimId::new(3)].into(),
    };
    let evidence = Evidence {
        id: EvidenceId::new(30),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(3)),
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range: ContentRange { start: 3, end: 8 },
            content_hash: "sha256:abc".to_string(),
            snapshot: None,
        },
        excerpt: "grounded excerpt".to_string(),
        observed_at: LogicalTick::new(4),
    };

    ChunkRepository::put(&store, late.clone()).expect("late chunk put");
    ChunkRepository::put(&store, early.clone()).expect("early chunk put");
    CardRepository::put(&store, card.clone()).expect("card put");
    EvidenceRepository::put(&store, evidence.clone()).expect("evidence put");

    assert_eq!(
        ChunkRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("chunk list"),
        vec![early.clone(), late.clone()]
    );
    assert_eq!(
        ChunkRepository::get(&store, late.id).expect("chunk get"),
        Some(late)
    );
    assert_eq!(
        CardRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("card list"),
        vec![card.clone()]
    );
    assert_eq!(
        CardRepository::get(&store, card.id).expect("card get"),
        Some(card)
    );
    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("evidence list"),
        vec![evidence.clone()]
    );
    assert_eq!(
        EvidenceRepository::get(&store, evidence.id).expect("evidence get"),
        Some(evidence)
    );
}

#[test]
fn evidence_kind_persists_without_domain_serde() {
    let store = SqliteStore::in_memory().expect("test setup");
    let command = Evidence {
        id: EvidenceId::new(31),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::CommandOutput {
            harness_run: HarnessRunId::new(8),
            stream: OutputStream::Stderr,
            blob: BlobId::new(9),
        },
        excerpt: "stderr excerpt".to_string(),
        observed_at: LogicalTick::new(5),
    };
    let validation = Evidence {
        id: EvidenceId::new(32),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(10),
        },
        excerpt: "validation excerpt".to_string(),
        observed_at: LogicalTick::new(6),
    };

    EvidenceRepository::put(&store, command.clone()).expect("command evidence put");
    EvidenceRepository::put(&store, validation.clone()).expect("validation evidence put");

    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1)).expect("evidence list"),
        vec![command, validation]
    );
}

#[test]
fn evidence_put_is_idempotent() {
    let store = SqliteStore::in_memory().expect("test setup");
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
    EvidenceRepository::put(&store, evidence.clone()).expect("first put must succeed");
    EvidenceRepository::put(&store, evidence.clone()).expect("identical retry must succeed");
    let stored = EvidenceRepository::get(&store, evidence.id)
        .expect("get after retry")
        .expect("evidence must still exist");
    assert_eq!(stored, evidence);
}

#[test]
fn evidence_put_rejects_conflicting_overwrite() {
    let store = SqliteStore::in_memory().expect("test setup");
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
    EvidenceRepository::put(&store, first.clone()).expect("first put must succeed");

    let conflict = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
    };
    let err = EvidenceRepository::put(&store, conflict).unwrap_err();
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    let stored = EvidenceRepository::get(&store, first.id)
        .expect("get after conflict")
        .expect("evidence must still exist");
    assert_eq!(stored, first);
}

#[test]
fn evidence_replace_overwrites_existing() {
    let store = SqliteStore::in_memory().expect("test setup");
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
    EvidenceRepository::put(&store, original.clone()).expect("first put");

    let replacement = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(2),
        claim_id: Some(ClaimId::new(9)),
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "corrected".to_string(),
        observed_at: LogicalTick::new(2),
    };

    // put rejects different content
    let err = EvidenceRepository::put(&store, replacement.clone()).unwrap_err();
    assert!(matches!(err, PortError::Conflict { .. }));

    // replace succeeds
    EvidenceRepository::replace(&store, replacement.clone()).expect("replace must overwrite");

    let stored = EvidenceRepository::get(&store, EvidenceId::new(300))
        .expect("get after replace")
        .expect("evidence must exist");
    assert_eq!(stored, replacement);

    // list_for_artifact reflects replacement
    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(2))
            .expect("list after replace"),
        vec![replacement]
    );
}

#[test]
fn artifact_index_status_round_trips_through_repository() {
    let store = SqliteStore::in_memory().expect("test setup");

    // Default artifact has Unindexed status and no content_hash
    let mut artifact = artifact(1);
    assert_eq!(artifact.index_status, IndexStatus::Unindexed);
    assert_eq!(artifact.content_hash, None);
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact.clone())
    );

    // Update to Pending with a content_hash
    artifact.index_status = IndexStatus::Pending;
    artifact.content_hash = Some("sha256:def456".to_string());
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact.clone())
    );

    // Update to Indexed, keep content_hash
    artifact.index_status = IndexStatus::Indexed;
    ArtifactRepository::put(&store, artifact.clone()).expect("put");
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1)).expect("get"),
        Some(artifact)
    );
}
