use maestria_domain::{
    ArtifactId, ArtifactVersion, ArtifactVersionId, ConflictSet, ConflictSetId, ContentRange,
    ContentHash, CorpusScope, CorpusSnapshotId, DuplicateClusterId, EvidenceCandidate,
    EvidenceCoverage, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprintId,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchCompatibilityError, SearchIntent,
    SearchOutcome, SearchPlan, SearchStage, SearchStatus, SearchTraceId, SourceLocation,
    StopConditions, StructureNodeId, TrustLabel,
};

fn plan() -> SearchPlan {
    SearchPlan {
        query_id: QueryId::new(7),
        original_query: "What changed?".to_owned(),
        intent: SearchIntent::FactVerification,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::MaximumAgeDays(30),
        modalities: ModalitySet {
            values: vec![Modality::Text, Modality::Code],
        },
        stages: vec![SearchStage::InitialRetrieval, SearchStage::Reranking],
        budgets: SearchBudget::new(2_000, 5_000).expect("valid budget"),
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 70,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: true,
            minimum_corroboration: 2,
        },
        fingerprint: RetrievalModelFingerprintId::new(17),
    }
}

fn candidate() -> EvidenceCandidate {
    EvidenceCandidate {
        artifact_version: ArtifactVersion::new(
            ArtifactVersionId::new(19),
            ArtifactId::new(23),
            ContentHash::new("sha256:abc123".to_owned()).expect("valid hash"),
        ),
        source_span: EvidenceSpan {
            node_id: Some(StructureNodeId::new(29)),
            location: SourceLocation::File {
                path: "notes/research.md".to_owned(),
                start_line: 4,
                end_line: 8,
            },
            range: ContentRange { start: 32, end: 96 },
        },
        retrieval_score: RetrievalScoreSet {
            bm25: 91,
            semantic_similarity: 88,
        },
        trust_label: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
    }
}

fn outcome() -> SearchOutcome {
    SearchOutcome {
        trace_id: SearchTraceId::new(37),
        fingerprint: RetrievalModelFingerprintId::new(17),
        index_generation: IndexGenerationId::new(13),
        status: SearchStatus::Success,
        coverage: EvidenceCoverage {
            percent_covered: 100,
            gaps_identified: Vec::new(),
        },
        conflicts: vec![ConflictSet {
            id: ConflictSetId::new(41),
            candidates: vec![candidate()],
        }],
        candidates: vec![candidate()],
    }
}

#[test]
fn plan_and_outcome_serialize_deterministically_and_round_trip() {
    let plan = plan();
    let outcome = outcome();

    let plan_json = serde_json::to_string(&plan).expect("plan serializes");
    let outcome_json = serde_json::to_string(&outcome).expect("outcome serializes");

    assert_eq!(plan_json, serde_json::to_string(&plan).expect("plan re-serializes"));
    assert_eq!(
        outcome_json,
        serde_json::to_string(&outcome).expect("outcome re-serializes")
    );
    assert_eq!(plan, serde_json::from_str(&plan_json).expect("plan round trips"));
    assert_eq!(
        outcome,
        serde_json::from_str(&outcome_json).expect("outcome round trips")
    );
}

#[test]
fn compatibility_rejects_model_and_index_mismatches() {
    let plan = plan();
    let mut outcome = outcome();

    assert_eq!(outcome.verify_compatibility(&plan), Ok(()));

    outcome.fingerprint = RetrievalModelFingerprintId::new(43);
    assert!(matches!(
        outcome.verify_compatibility(&plan),
        Err(SearchCompatibilityError::ModelFingerprintMismatch { .. })
    ));

    outcome.fingerprint = plan.fingerprint;
    outcome.index_generation = IndexGenerationId::new(47);
    assert!(matches!(
        outcome.verify_compatibility(&plan),
        Err(SearchCompatibilityError::IndexGenerationMismatch { .. })
    ));
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
}
