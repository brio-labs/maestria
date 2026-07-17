use crate::SqliteStore;
use maestria_domain::*;
use maestria_ports::*;

use super::artifact;

#[test]
fn artifact_put_get_and_missing() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    assert_eq!(ArtifactRepository::get(&store, ArtifactId::new(9))?, None);

    let artifact = artifact(1);
    ArtifactRepository::put(&store, artifact.clone())?;

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1))?,
        Some(artifact)
    );
    Ok(())
}

#[test]
fn artifact_relationship_sets_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
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

    ArtifactRepository::put(&store, artifact.clone())?;

    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1))?,
        Some(artifact)
    );
    Ok(())
}

#[test]
fn brain_state_round_trips_and_lists_deterministically() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
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
        security: SecurityMetadata::default(),
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
        security: SecurityMetadata::default(),
    };

    ChunkRepository::put(&store, late.clone())?;
    ChunkRepository::put(&store, early.clone())?;
    CardRepository::put(&store, card.clone())?;
    EvidenceRepository::put(&store, evidence.clone())?;

    assert_eq!(
        ChunkRepository::list_for_artifact(&store, ArtifactId::new(1))?,
        vec![early.clone(), late.clone()]
    );
    assert_eq!(ChunkRepository::get(&store, late.id)?, Some(late));
    assert_eq!(
        CardRepository::list_for_artifact(&store, ArtifactId::new(1))?,
        vec![card.clone()]
    );
    assert_eq!(CardRepository::get(&store, card.id)?, Some(card));
    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1))?,
        vec![evidence.clone()]
    );
    assert_eq!(
        EvidenceRepository::get(&store, evidence.id)?,
        Some(evidence)
    );
    Ok(())
}

#[test]
fn evidence_kind_persists_without_domain_serde() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
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
        security: SecurityMetadata::default(),
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
        security: SecurityMetadata::default(),
    };

    EvidenceRepository::put(&store, command.clone())?;
    EvidenceRepository::put(&store, validation.clone())?;

    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(1))?,
        vec![command, validation]
    );
    Ok(())
}

#[test]
fn evidence_put_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let evidence = Evidence {
        id: EvidenceId::new(100),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "test excerpt".to_string(),
        observed_at: LogicalTick::new(1),
        security: SecurityMetadata::default(),
    };
    EvidenceRepository::put(&store, evidence.clone())?;
    EvidenceRepository::put(&store, evidence.clone())?;
    let stored = EvidenceRepository::get(&store, evidence.id)?.ok_or(PortError::NotFound)?;
    assert_eq!(stored, evidence);
    Ok(())
}

#[test]
fn evidence_put_rejects_conflicting_overwrite() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let first = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original".to_string(),
        observed_at: LogicalTick::new(1),
        security: SecurityMetadata::default(),
    };
    EvidenceRepository::put(&store, first.clone())?;

    let conflict = Evidence {
        id: EvidenceId::new(200),
        artifact_id: ArtifactId::new(10),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "different".to_string(),
        observed_at: LogicalTick::new(2),
        security: SecurityMetadata::default(),
    };
    let err = match EvidenceRepository::put(&store, conflict) {
        Err(e) => e,
        Ok(_) => return Err("expected error".into()),
    };
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "conflicting put must return Conflict, got {err:?}"
    );

    let stored = EvidenceRepository::get(&store, first.id)?.ok_or(PortError::NotFound)?;
    assert_eq!(stored, first);
    Ok(())
}

#[test]
fn evidence_replace_overwrites_existing() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let original = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "malformed".to_string(),
        observed_at: LogicalTick::new(1),
        security: SecurityMetadata::default(),
    };
    EvidenceRepository::put(&store, original.clone())?;

    let replacement = Evidence {
        id: EvidenceId::new(300),
        artifact_id: ArtifactId::new(2),
        claim_id: Some(ClaimId::new(9)),
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "corrected".to_string(),
        observed_at: LogicalTick::new(2),
        security: SecurityMetadata::default(),
    };

    // put rejects different content
    let err = match EvidenceRepository::put(&store, replacement.clone()) {
        Err(e) => e,
        Ok(_) => return Err("expected error".into()),
    };
    assert!(matches!(err, PortError::Conflict { .. }));

    // replace succeeds
    EvidenceRepository::replace(&store, replacement.clone())?;

    let stored =
        EvidenceRepository::get(&store, EvidenceId::new(300))?.ok_or(PortError::NotFound)?;
    assert_eq!(stored, replacement);

    // list_for_artifact reflects replacement
    assert_eq!(
        EvidenceRepository::list_for_artifact(&store, ArtifactId::new(2))?,
        vec![replacement]
    );
    Ok(())
}

#[test]
fn artifact_index_status_round_trips_through_repository() -> Result<(), Box<dyn std::error::Error>>
{
    let store = SqliteStore::in_memory()?;

    // Default artifact has Unindexed status and no content_hash

    let mut artifact = artifact(1);
    assert_eq!(artifact.index_status, IndexStatus::Unindexed);
    assert_eq!(artifact.content_hash, None);
    ArtifactRepository::put(&store, artifact.clone())?;
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1))?,
        Some(artifact.clone())
    );

    // Update to Pending with a content_hash
    artifact.index_status = IndexStatus::Pending;
    artifact.content_hash = Some("sha256:def456".to_string());
    ArtifactRepository::put(&store, artifact.clone())?;
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1))?,
        Some(artifact.clone())
    );

    // Update to Indexed, keep content_hash
    artifact.index_status = IndexStatus::Indexed;
    ArtifactRepository::put(&store, artifact.clone())?;
    assert_eq!(
        ArtifactRepository::get(&store, ArtifactId::new(1))?,
        Some(artifact)
    );
    Ok(())
}

#[test]
fn security_metadata_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let mut a = artifact(1);

    let sec = maestria_domain::SecurityMetadata {
        trust_zone: maestria_domain::TrustZone::System,
        authority: maestria_domain::Authority::System,
        integrity: maestria_domain::IntegrityState::Verified,
        sensitivity: maestria_domain::Sensitivity::Internal,
        review_status: maestria_domain::ReviewStatus::Approved,
        quarantined: true,
        prompt_injection_risk: true,
        poisoning_flags: vec!["test".to_string()],
        read_allowed: true,
        write_allowed: true,
        scope_id: None,
    };
    a.security = sec.clone();

    ArtifactRepository::put(&store, a.clone())?;
    let fetched = ArtifactRepository::get(&store, a.id)?.ok_or(PortError::NotFound)?;
    assert_eq!(fetched.security, sec);

    let card = Card {
        id: CardId::new(2),
        artifact_id: a.id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span: maestria_domain::SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        },
        title: "Test Card".to_string(),
        body: "Card body".to_string(),
        claim_ids: std::collections::BTreeSet::new(),
        security: sec.clone(),
    };
    CardRepository::put(&store, card.clone())?;
    let fetched_card = CardRepository::get(&store, card.id)?.ok_or(PortError::NotFound)?;
    assert_eq!(fetched_card.security, sec);

    let evidence = Evidence {
        id: EvidenceId::new(3),
        artifact_id: a.id,
        claim_id: None,
        kind: maestria_domain::EvidenceKind::Validation {
            report_id: maestria_domain::ValidationReportId::new(1),
        },
        excerpt: "evidence body".to_string(),
        observed_at: LogicalTick::new(1),
        security: sec.clone(),
    };
    EvidenceRepository::put(&store, evidence.clone())?;
    let fetched_ev = EvidenceRepository::get(&store, evidence.id)?.ok_or(PortError::NotFound)?;
    assert_eq!(fetched_ev.security, sec);
    Ok(())
}
