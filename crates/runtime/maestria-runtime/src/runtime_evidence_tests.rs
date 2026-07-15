use super::test_support::*;
use maestria_domain::{
    Artifact, ArtifactId, DomainEventEnvelope, EventId, Evidence, EvidenceId, EvidenceKind,
    IndexStatus, LogicalTick, SequenceNumber,
};
use maestria_ports::{EvidenceRepository, InMemoryEvidenceRepository};
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[tokio::test]
async fn evidence_recorded_persistence_replaces_malformed_record() {
    let artifact_id = ArtifactId::new(1);
    let evidence_id = EvidenceId::new(1);

    // Pre-populate the evidence repository with a malformed record
    // simulating stale data from a prior incomplete replay.
    let evidence_repo = Arc::new(InMemoryEvidenceRepository::new());
    let malformed = Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/wrong.txt".into(),
            range: maestria_domain::ContentRange { start: 0, end: 1 },
            content_hash: "bad".into(),
            snapshot: None,
        },
        excerpt: "malformed excerpt".into(),
        observed_at: LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    };
    evidence_repo
        .put(malformed.clone())
        .expect("pre-populate malformed");

    let artifact = Artifact {
        id: artifact_id,
        title: "test".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Unindexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let valid_evidence = Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "/correct.txt".into(),
            range: maestria_domain::ContentRange { start: 0, end: 10 },
            content_hash: "abc".into(),
            snapshot: None,
        },
        excerpt: "valid excerpt".into(),
        observed_at: LogicalTick::new(2),
        security: maestria_domain::SecurityMetadata::default(),
    };
    let mut state = KernelState::new();
    state.artifacts.insert(artifact_id, artifact);
    state.evidences.insert(evidence_id, valid_evidence.clone());

    let adapters = Adapters {
        evidence_repo: evidence_repo.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let envelope = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::EvidenceRecorded {
            evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "/correct.txt".into(),
                range: maestria_domain::ContentRange { start: 0, end: 10 },
                content_hash: "abc".into(),
                snapshot: None,
            },
            excerpt: "valid excerpt".into(),
            observed_at: LogicalTick::new(2),
            security: maestria_domain::SecurityMetadata::default(),
        },
    };

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(state)),
        input_tx,
    );
    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::PersistEvent {
            envelope: envelope.clone(),
        },
        ctx,
        None,
    )
    .await;
    assert!(
        result,
        "persist of EvidenceRecorded should succeed despite existing malformed record"
    );

    // The repository must now contain the valid evidence (replaced), not the malformed one.
    let stored = evidence_repo
        .get(evidence_id)
        .expect("get after replace")
        .expect("evidence must exist after replace");
    assert_eq!(
        stored, valid_evidence,
        "malformed evidence must be replaced by valid evidence"
    );
    assert_ne!(stored, malformed, "malformed evidence must not remain");
}
