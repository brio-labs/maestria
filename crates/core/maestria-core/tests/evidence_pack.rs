use maestria_core::{
    ClaimCoverageStatus, ClaimEvidenceCoverage, EvidencePack, EvidencePackCompression,
    EvidencePackError, EvidencePackReproducibility, SourceGroundedSearchHit,
};
use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, BlobId, Chunk, ChunkId, ConflictSet, ContentRange,
    CorpusScope, CorpusSnapshotId, Evidence, EvidenceCandidate, EvidenceId, EvidenceKind,
    EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus, IndexGenerationId,
    IndexStatus, LogicalTick, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent, SearchPlan, SearchStage,
    SearchStopReason, SearchTrace, SourceLocation, SourceSpan, StopConditions, StructureNodeId,
    TrustLabel,
};
use std::error::Error;

fn plan(required_claims: Vec<String>) -> Result<SearchPlan, Box<dyn Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(7),
        original_query: "evidence query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_limits(100, 1_000, 2, 1, 0)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 0,
            required_claims,
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("test-fingerprint".to_string())?,
        original_intent: None,
        route_decision: None,
    })
}
fn trace_for(
    plan: &SearchPlan,
    evidence_ids: &[EvidenceId],
) -> Result<SearchTrace, Box<dyn Error>> {
    let evidence = evidence_ids
        .iter()
        .map(|evidence_id| {
            Ok(EvidenceCandidate {
                evidence_id: *evidence_id,
                artifact_version: ArtifactVersionId::new(101),
                source_span: EvidenceSpan::new(
                    Some(StructureNodeId::new(1)),
                    SourceLocation::File {
                        path: "source.md".to_string(),
                        start_line: 1,
                        end_line: 1,
                    },
                    ContentRange { start: 1, end: 1 },
                )?,
                scores: RetrievalScoreSet {
                    bm25: 1,
                    semantic_similarity: 0,
                },
                trust: TrustLabel::Verified,
                freshness: FreshnessStatus::UpToDate,
                duplicate_cluster: None,
                reasons: vec![RetrievalReason::ExactMatch],
                coverage_keys: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let mut trace = SearchTrace::from_plan(
        plan,
        Vec::new(),
        &evidence,
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    );
    trace.policy_fingerprint = Some("policy-v1".to_string());
    Ok(trace)
}
fn file_hit(snapshot: Option<BlobId>) -> SourceGroundedSearchHit {
    let artifact_id = ArtifactId::new(101);
    let chunk_id = ChunkId::new(102);
    let evidence_id = EvidenceId::new(103);
    SourceGroundedSearchHit {
        artifact: Artifact {
            id: artifact_id,
            title: "source.md".to_string(),
            chunk_ids: [chunk_id].into(),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: [evidence_id].into(),
            index_status: IndexStatus::Indexed,
            content_hash: Some("hash".to_string()),
            parse_status: None,
            security: Default::default(),
        },
        chunk: Chunk {
            id: chunk_id,
            artifact_id,
            node_id: StructureNodeId::new(1),
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
            representations: Vec::new(),
            order: 0,
            text: "evidence".to_string(),
        },
        evidence: Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "source.md".to_string(),
                range: ContentRange { start: 1, end: 1 },
                content_hash: "hash".to_string(),
                snapshot,
            },
            excerpt: "evidence".to_string(),
            observed_at: LogicalTick::new(1),
            security: Default::default(),
        },
        score: 1,
        lexical_metadata: None,
    }
}

#[test]
fn mutable_file_evidence_cannot_be_frozen() -> Result<(), Box<dyn Error>> {
    let hit = file_hit(None);
    let evidence_id = hit.evidence.id;
    let plan = plan(Vec::new())?;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &plan,
    )?;
    assert!(matches!(
        pack.freeze(trace_for(&plan, &[evidence_id])?, "policy-v1".to_string()),
        Err(EvidencePackError::InvalidFreeze(_))
    ));
    Ok(())
}

#[test]
fn missing_claims_and_live_reproducibility_are_explicit() -> Result<(), Box<dyn Error>> {
    let pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &plan(vec!["claim-a".to_string()])?,
    )?;

    assert_eq!(pack.metadata().claims_required, vec!["claim-a"]);
    assert_eq!(
        pack.metadata().claim_coverage[0].status,
        ClaimCoverageStatus::Missing
    );
    assert_eq!(pack.metadata().missing_evidence, vec!["claim-a"]);
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::RequirementsUnmet
    );
    assert!(matches!(
        pack.metadata().reproducibility,
        EvidencePackReproducibility::LiveNonReproducible { .. }
    ));
    Ok(())
}

#[test]
fn empty_pack_reports_no_evidence() -> Result<(), Box<dyn Error>> {
    let pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &plan(Vec::new())?,
    )?;

    assert_eq!(pack.metadata().stop_reason, SearchStopReason::NoEvidence);
    Ok(())
}
#[test]
fn pack_rejects_unmaterialized_evidence_ids() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        EvidencePack::from_plan(
            "evidence query".to_string(),
            Vec::new(),
            Vec::new(),
            vec![EvidenceId::new(31)],
            &plan(Vec::new())?,
        ),
        Err(EvidencePackError::UnmaterializedEvidence(_))
    ));
    Ok(())
}
#[test]
fn explicit_claim_coverage_can_be_recorded_without_guessing() -> Result<(), Box<dyn Error>> {
    let pack_plan = plan(vec!["claim-a".to_string()])?;
    let hit = file_hit(Some(BlobId::new(23)));
    let evidence_id = hit.evidence.id;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &pack_plan,
    )?;
    pack.set_claim_coverage(vec![ClaimEvidenceCoverage {
        claim: "claim-a".to_string(),
        evidence_ids: vec![evidence_id],
        status: ClaimCoverageStatus::Supported,
    }])?;
    assert!(pack.metadata().missing_evidence.is_empty());
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::EvidenceComplete
    );
    Ok(())
}

#[test]
fn primary_source_verification_is_a_lifecycle_transition() -> Result<(), Box<dyn Error>> {
    let hit = file_hit(Some(BlobId::new(23)));
    let evidence_id = hit.evidence.id;
    let mut primary_plan = plan(Vec::new())?;
    primary_plan.evidence_requirements.require_primary_sources = true;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &primary_plan,
    )?;

    assert!(!pack.metadata().primary_sources_verified);
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::RequirementsUnmet
    );
    pack.mark_primary_sources_verified(true)?;
    assert!(pack.metadata().primary_sources_verified);
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::EvidenceComplete
    );
    Ok(())
}

#[test]
fn frozen_pack_reproduces_only_for_the_same_identity() -> Result<(), Box<dyn Error>> {
    let hit = file_hit(Some(BlobId::new(23)));
    let evidence_id = hit.evidence.id;
    let plan = plan(Vec::new())?;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &plan,
    )?;
    pack.freeze(trace_for(&plan, &[evidence_id])?, "policy-v1".to_string())?;

    let key = match &pack.metadata().reproducibility {
        EvidencePackReproducibility::Frozen(key) => key.clone(),
        EvidencePackReproducibility::LiveNonReproducible { .. } => {
            return Err("freeze did not produce a frozen replay key".into());
        }
    };
    let reproduced = pack.reproduce(&key)?;
    assert_eq!(reproduced, pack);
    assert!(matches!(
        pack.set_claim_coverage(Vec::new()),
        Err(EvidencePackError::FrozenMutation(_))
    ));

    let wrong_key = maestria_core::EvidencePackReplayKey {
        policy_fingerprint: "policy-v2".to_string(),
        ..key
    };
    assert_eq!(
        pack.reproduce(&wrong_key),
        Err(EvidencePackError::ReplayIdentityMismatch)
    );
    Ok(())
}

#[test]
fn freeze_rejects_mismatched_candidate_provenance() -> Result<(), Box<dyn Error>> {
    let hit = file_hit(Some(BlobId::new(23)));
    let evidence_id = hit.evidence.id;
    let plan = plan(Vec::new())?;
    let mut trace = trace_for(&plan, &[evidence_id])?;
    trace.raw_candidates[0].source_span = EvidenceSpan::new(
        Some(StructureNodeId::new(1)),
        SourceLocation::File {
            path: "source.md".to_string(),
            start_line: 2,
            end_line: 2,
        },
        ContentRange { start: 1, end: 1 },
    )?;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &plan,
    )?;

    assert!(matches!(
        pack.freeze(trace, "policy-v1".to_string()),
        Err(EvidencePackError::InvalidFreeze(_))
    ));
    Ok(())
}

#[test]
fn compression_preserves_lineage_and_is_not_verbatim() -> Result<(), Box<dyn Error>> {
    let hit = file_hit(Some(BlobId::new(23)));
    let evidence_id = hit.evidence.id;
    let mut pack = EvidencePack::from_plan(
        "evidence query".to_string(),
        Vec::new(),
        vec![hit],
        vec![evidence_id],
        &plan(Vec::new())?,
    )?;
    pack.set_conflicts(
        vec![ConflictSet {
            id: maestria_domain::ConflictSetId::new(41),
            candidates: Vec::new(),
        }],
        vec![evidence_id],
    )?;
    assert_eq!(pack.metadata().counterevidence, vec![evidence_id]);
    assert_eq!(pack.metadata().conflicts.len(), 1);
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::RequirementsUnmet
    );

    assert!(
        pack.compress(vec![evidence_id], "retain-primary".to_string())
            .is_ok()
    );
    assert!(matches!(
        pack.metadata().compression,
        EvidencePackCompression::Compressed { .. }
    ));
    assert_eq!(pack.metadata().counterevidence, vec![evidence_id]);
    assert_eq!(pack.metadata().conflicts.len(), 1);
    assert_eq!(
        pack.metadata().stop_reason,
        SearchStopReason::RequirementsUnmet
    );
    assert!(matches!(
        pack.compress(Vec::new(), String::new()),
        Err(EvidencePackError::InvalidCompression(_))
    ));
    Ok(())
}
