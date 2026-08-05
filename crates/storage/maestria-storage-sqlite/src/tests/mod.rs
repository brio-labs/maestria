mod contract_tests;
mod event_tests;
mod id_allocator_tests;
mod index_event_tests;
mod index_generation_tests;
mod learned_sparse_observation_tests;
mod learned_sparse_projection_tests;
mod migration_tests;
mod realm_read_grant_tests;
mod repository_tests;

use std::collections::BTreeSet;

use maestria_domain::*;

pub(super) fn artifact(id: u64) -> Artifact {
    Artifact {
        id: ArtifactId::new(id),
        title: format!("artifact {id}"),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::default(),
        parse_status: None,
        content_hash: None,
        security: SecurityMetadata::default(),
    }
}

pub(super) fn registered(event_id: u64, sequence: u64, artifact_id: u64) -> DomainEventEnvelope {
    DomainEventEnvelope {
        id: EventId::new(event_id),
        sequence: SequenceNumber::new(sequence),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(artifact_id),
            title: format!("artifact {artifact_id}"),
            security: SecurityMetadata::default(),
        },
    }
}

/// A verified, exactly-matched evidence candidate shared by the stored
/// search-candidate and search-outcome round-trip tests (Rule 26: fixtures
/// are shared through explicit helpers, never copied between test modules).
pub(crate) fn sample_evidence_candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;

    let lane = RetrievalLaneScore::new(
        RetrievalScoreKind::Exact,
        1,
        RetrievalRawRank::ranked(1),
        RetrievalScoreScale::Binary,
        RepresentationName::new("text/plain"),
        RetrievalScoreFingerprint::new(
            RetrievalModelFingerprint::new("fp-v1".to_string())?,
            BTreeMap::from([("model".to_string(), "exact".to_string())]),
        ),
    );
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(41),
        artifact_version: ArtifactVersionId::new(42),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(3)),
            SourceLocation::file("/repo/src/lib.rs".to_string(), 10, 20)?,
            ContentRange::new(100, 250)?,
        )?,
        scores: RetrievalScoreSet::new(vec![lane])?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(11)),
        reasons: vec![
            RetrievalReason::ExactMatch,
            RetrievalReason::LearnedSparse(Box::new(LearnedSparseReason::new(vec![
                LearnedSparseContribution {
                    term_id: 5,
                    contribution_micros: 42,
                },
            ]))),
        ],
        coverage_keys: vec!["doc:7".to_string()],
    })?)
}
mod effect_journal_tests;
