use maestria_domain::{
    ArtifactId, ArtifactVersion, ArtifactVersionId, ConflictSet, ConflictSetId, ContentHash,
    ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId, EvidenceCandidate,
    EvidenceCoverage, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchCompatibilityError, SearchIntent,
    SearchOutcome, SearchPlan, SearchStage, SearchStatus, SearchStopReason, SearchTrace,
    SearchTraceFilter, SearchTraceId, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(7),
        original_query: "What changed?".to_owned(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::MaximumAgeDays(30),
        modalities: ModalitySet::new(vec![Modality::Code, Modality::Text, Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval, SearchStage::Reranking],
        budgets: SearchBudget::new(2_000, 5_000)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 70,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: true,
            minimum_corroboration: 2,
        },
        fingerprint: RetrievalModelFingerprint::new("model:v1".to_owned())?,
    })
}
fn artifact_version() -> Result<ArtifactVersion, Box<dyn std::error::Error>> {
    Ok(ArtifactVersion::new(
        ArtifactVersionId::new(19),
        ArtifactId::new(23),
        ContentHash::new(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        )?,
    ))
}

fn candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate {
        evidence_id: EvidenceId::new(23),
        artifact_version: ArtifactVersionId::new(19),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(29)),
            SourceLocation::File {
                path: "notes/research.md".to_owned(),
                start_line: 4,
                end_line: 8,
            },
            ContentRange { start: 32, end: 96 },
        )?,
        scores: RetrievalScoreSet {
            bm25: 91,
            semantic_similarity: 88,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
        coverage_keys: vec![],
    })
}

fn outcome() -> Result<SearchOutcome, Box<dyn std::error::Error>> {
    Ok(SearchOutcome {
        trace: SearchTraceId::new(37),
        trace_data: None,
        fingerprint: RetrievalModelFingerprint::new("model:v1".to_owned())?,
        index_generation: IndexGenerationId::new(13),
        status: SearchStatus::Answerable,
        evidence: vec![candidate()?],
        coverage: EvidenceCoverage {
            percent_covered: 50,
            gaps_identified: vec!["missing section".to_owned()],
            required_claims: vec![],
            required_subquestions: vec![],
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            candidate_coverage_keys: vec![],
        },
        conflicts: vec![ConflictSet {
            id: ConflictSetId::new(41),
            candidates: vec![candidate()?],
        }],
    })
}

#[test]
fn plan_and_outcome_serialize_deterministically_and_round_trip()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let outcome = outcome()?;

    let version = artifact_version()?;
    let version_json = serde_json::to_string(&version)?;
    let plan_json = serde_json::to_string(&plan)?;
    let outcome_json = serde_json::to_string(&outcome)?;

    assert_eq!(plan_json, serde_json::to_string(&plan)?);
    assert_eq!(outcome_json, serde_json::to_string(&outcome)?);
    assert_eq!(version_json, serde_json::to_string(&version)?);
    assert_eq!(plan, serde_json::from_str(&plan_json)?);
    assert_eq!(outcome, serde_json::from_str(&outcome_json)?);
    assert_eq!(version, serde_json::from_str(&version_json)?);
    Ok(())
}

#[test]
fn compatibility_rejects_model_and_index_mismatches() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut outcome = outcome()?;

    assert_eq!(outcome.verify_compatibility(&plan), Ok(()));

    outcome.fingerprint = RetrievalModelFingerprint::new("model:v2".to_owned())?;
    assert!(matches!(
        outcome.verify_compatibility(&plan),
        Err(SearchCompatibilityError::ModelFingerprintMismatch { .. })
    ));

    outcome.fingerprint = plan.fingerprint.clone();
    outcome.index_generation = IndexGenerationId::new(47);
    assert!(matches!(
        outcome.verify_compatibility(&plan),
        Err(SearchCompatibilityError::IndexGenerationMismatch { .. })
    ));
    Ok(())
}

#[test]
fn trace_captures_plan_and_rejects_incompatible_replay() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let trace = SearchTrace::from_plan(
        &plan,
        vec!["cards".to_owned(), "lexical_chunks".to_owned()],
        &[candidate()?],
        vec![SearchTraceFilter::Acl, SearchTraceFilter::PromptInjection],
        Some("rrf-fixed-k60".to_owned()),
        vec![],
        SearchStopReason::EvidenceComplete,
    )
    .with_gaps_and_conflicts(
        vec!["missing section".to_owned()],
        vec![ConflictSetId::new(41)],
    );
    let mut outcome = outcome()?;
    outcome.trace = trace.deterministic_id();
    outcome.trace_data = Some(Box::new(trace));

    assert_eq!(outcome.verify_compatibility(&plan), Ok(()));
    let mut incompatible = plan.clone();
    incompatible.original_query = "different query".to_owned();
    assert!(matches!(
        outcome.verify_compatibility(&incompatible),
        Err(SearchCompatibilityError::TracePlanMismatch(_))
    ));
    let mut mismatched_evidence = outcome;
    mismatched_evidence.evidence.clear();
    assert!(matches!(
        mismatched_evidence.verify_compatibility(&plan),
        Err(SearchCompatibilityError::TracePlanMismatch(_))
    ));
    Ok(())
}
#[test]
fn trace_lane_changes_alter_deterministic_identity() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_domain::{SearchLaneStatus, SearchTraceLane, SearchTraceLaneCandidate};
    let plan = plan()?;
    let mut trace = SearchTrace::from_plan(
        &plan,
        vec![],
        &[],
        vec![],
        None,
        vec![],
        SearchStopReason::EvidenceComplete,
    );
    let id_without_lanes = trace.deterministic_id();

    trace = trace.with_lanes(vec![SearchTraceLane {
        retriever_id: "lexical".to_owned(),
        query: "test query".to_owned(),
        generation: None,
        status: SearchLaneStatus::Succeeded,
        candidates: vec![SearchTraceLaneCandidate {
            evidence_id: EvidenceId::new(42),
            artifact_version: ArtifactVersionId::new(10),
            source_span: EvidenceSpan::new(
                None,
                SourceLocation::File {
                    path: "test".to_owned(),
                    start_line: 1,
                    end_line: 2,
                },
                maestria_domain::ContentRange { start: 0, end: 10 },
            )?,
            lane_rank: 1,
            duplicate_cluster: None,
            scores: RetrievalScoreSet {
                bm25: 1,
                semantic_similarity: 0,
            },
            reasons: vec![],
        }],
    }]);
    let id_with_succeeded_lane = trace.deterministic_id();
    assert_ne!(id_without_lanes, id_with_succeeded_lane);

    trace = trace.with_lanes(vec![SearchTraceLane {
        retriever_id: "lexical".to_owned(),
        query: "test query".to_owned(),
        generation: None,
        status: SearchLaneStatus::Failed {
            error: "timeout".to_owned(),
        },
        candidates: vec![],
    }]);
    let id_with_failed_lane = trace.deterministic_id();
    assert_ne!(id_with_succeeded_lane, id_with_failed_lane);
    Ok(())
}

#[test]
fn trace_identity_versions_do_not_alias() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let trace = SearchTrace::from_plan(
        &plan,
        Vec::new(),
        &[],
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    );
    let current_id = trace.deterministic_id();
    let mut previous = trace.clone();
    previous.identity_version = 2;
    let mut future = trace;
    future.identity_version = 4;

    assert_ne!(current_id, previous.deterministic_id());
    assert_ne!(current_id, future.deterministic_id());
    assert_ne!(previous.deterministic_id(), future.deterministic_id());
    Ok(())
}

#[test]
fn trace_lanes_serialize_and_deserialize_without_fallback() -> Result<(), Box<dyn std::error::Error>>
{
    use maestria_domain::{SearchLaneStatus, SearchTraceLane};
    let plan = plan()?;
    let trace = SearchTrace::from_plan(
        &plan,
        vec![],
        &[],
        vec![],
        None,
        vec![],
        SearchStopReason::EvidenceComplete,
    )
    .with_lanes(vec![SearchTraceLane {
        retriever_id: "dense".to_owned(),
        query: "test query".to_owned(),
        generation: None,
        status: SearchLaneStatus::Failed {
            error: "unreachable".to_owned(),
        },
        candidates: vec![],
    }]);

    let json = serde_json::to_string(&trace)?;
    assert!(json.contains("Failed"));
    assert!(json.contains("unreachable"));

    let round_tripped: SearchTrace = serde_json::from_str(&json)?;
    assert_eq!(round_tripped.lanes.len(), 1);
    assert!(matches!(
        round_tripped.lanes[0].status,
        SearchLaneStatus::Failed { .. }
    ));
    Ok(())
}

#[test]
fn invalid_budget_and_content_hash_are_typed_errors() -> Result<(), Box<dyn std::error::Error>> {
    assert!(matches!(
        SearchBudget::new(0, 1),
        Err(SearchCompatibilityError::InvalidBudget(_))
    ));
    assert!(matches!(
        ContentHash::new("md5:abc".to_owned()),
        Err(SearchCompatibilityError::InvalidContentHash(_))
    ));
    assert!(matches!(
        RetrievalModelFingerprint::new("  ".to_owned()),
        Err(SearchCompatibilityError::InvalidFingerprint(_))
    ));
    Ok(())
}

#[test]
fn serde_rejects_invalid_spans_and_coverage() -> Result<(), Box<dyn std::error::Error>> {
    let invalid_span = r#"{
        "node_id": null,
        "location": {"File": {"path": "notes.md", "start_line": 4, "end_line": 3}},
        "range": {"start": 8, "end": 2}
    }"#;
    assert!(serde_json::from_str::<EvidenceSpan>(invalid_span).is_err());

    let invalid_coverage = r#"{"percent_covered":101,"gaps_identified":[]}"#;
    assert!(serde_json::from_str::<EvidenceCoverage>(invalid_coverage).is_err());
    Ok(())
}
