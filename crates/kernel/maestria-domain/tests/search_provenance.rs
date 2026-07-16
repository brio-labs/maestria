use maestria_domain::{
    ArtifactId, ArtifactVersion, ArtifactVersionId, ConflictSet, ConflictSetId, ContentHash,
    ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId, EvidenceCandidate,
    EvidenceCoverage, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchCompatibilityError, SearchIntent,
    SearchOutcome, SearchPlan, SearchStage, SearchStatus, SearchStopReason, SearchTrace,
    SearchTraceFilter, SearchTraceId, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};

fn plan() -> SearchPlan {
    SearchPlan {
        query_id: QueryId::new(7),
        original_query: "What changed?".to_owned(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::MaximumAgeDays(30),
        modalities: ModalitySet::new(vec![Modality::Code, Modality::Text, Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval, SearchStage::Reranking],
        budgets: SearchBudget::new(2_000, 5_000).expect("valid budget"),
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
        fingerprint: RetrievalModelFingerprint::new("model:v1".to_owned())
            .expect("valid fingerprint"),
    }
}
fn artifact_version() -> ArtifactVersion {
    ArtifactVersion::new(
        ArtifactVersionId::new(19),
        ArtifactId::new(23),
        ContentHash::new(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        )
        .expect("valid hash"),
    )
}

fn candidate() -> EvidenceCandidate {
    EvidenceCandidate {
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
        )
        .expect("valid source span"),
        scores: RetrievalScoreSet {
            bm25: 91,
            semantic_similarity: 88,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
        coverage_keys: vec![],
    }
}

fn outcome() -> SearchOutcome {
    SearchOutcome {
        trace: SearchTraceId::new(37),
        trace_data: None,
        fingerprint: RetrievalModelFingerprint::new("model:v1".to_owned())
            .expect("valid fingerprint"),
        index_generation: IndexGenerationId::new(13),
        status: SearchStatus::Answerable,
        evidence: vec![candidate()],
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
            candidates: vec![candidate()],
        }],
    }
}

#[test]
fn plan_and_outcome_serialize_deterministically_and_round_trip() {
    let plan = plan();
    let outcome = outcome();

    let version = artifact_version();
    let version_json = serde_json::to_string(&version).expect("version serializes");
    let plan_json = serde_json::to_string(&plan).expect("plan serializes");
    let outcome_json = serde_json::to_string(&outcome).expect("outcome serializes");

    assert_eq!(
        plan_json,
        serde_json::to_string(&plan).expect("plan re-serializes")
    );
    assert_eq!(
        outcome_json,
        serde_json::to_string(&outcome).expect("outcome re-serializes")
    );
    assert_eq!(
        version_json,
        serde_json::to_string(&version).expect("version re-serializes")
    );
    assert_eq!(
        plan,
        serde_json::from_str(&plan_json).expect("plan round trips")
    );
    assert_eq!(
        outcome,
        serde_json::from_str(&outcome_json).expect("outcome round trips")
    );
    assert_eq!(
        version,
        serde_json::from_str(&version_json).expect("version round trips")
    );
}

#[test]
fn compatibility_rejects_model_and_index_mismatches() {
    let plan = plan();
    let mut outcome = outcome();

    assert_eq!(outcome.verify_compatibility(&plan), Ok(()));

    outcome.fingerprint =
        RetrievalModelFingerprint::new("model:v2".to_owned()).expect("valid fingerprint");
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
}

#[test]
fn trace_captures_plan_and_rejects_incompatible_replay() {
    let plan = plan();
    let trace = SearchTrace::from_plan(
        &plan,
        vec!["cards".to_owned(), "lexical_chunks".to_owned()],
        &[candidate()],
        vec![SearchTraceFilter::Acl, SearchTraceFilter::PromptInjection],
        Some("rrf-fixed-k60".to_owned()),
        vec![],
        SearchStopReason::EvidenceComplete,
    )
    .with_gaps_and_conflicts(
        vec!["missing section".to_owned()],
        vec![ConflictSetId::new(41)],
    );
    let mut outcome = outcome();
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
}
#[test]
fn trace_lane_changes_alter_deterministic_identity() -> Result<(), Box<dyn std::error::Error>> {
    use maestria_domain::{SearchLaneStatus, SearchTraceLane, SearchTraceLaneCandidate};
    let plan = plan();
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
fn trace_lanes_serialize_and_deserialize_without_fallback() -> Result<(), Box<dyn std::error::Error>>
{
    use maestria_domain::{SearchLaneStatus, SearchTraceLane};
    let plan = plan();
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
fn invalid_budget_and_content_hash_are_typed_errors() {
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
}

#[test]
fn serde_rejects_invalid_spans_and_coverage() {
    let invalid_span = r#"{
        "node_id": null,
        "location": {"File": {"path": "notes.md", "start_line": 4, "end_line": 3}},
        "range": {"start": 8, "end": 2}
    }"#;
    assert!(serde_json::from_str::<EvidenceSpan>(invalid_span).is_err());

    let invalid_coverage = r#"{"percent_covered":101,"gaps_identified":[]}"#;
    assert!(serde_json::from_str::<EvidenceCoverage>(invalid_coverage).is_err());
}
