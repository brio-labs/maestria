use maestria_domain::{
    ArtifactId, ArtifactVersion, ArtifactVersionId, ConflictSet, ConflictSetId, ContentHash,
    ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId, EvidenceCandidate,
    EvidenceCoverage, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchCompatibilityError, SearchIntent,
    SearchOutcome, SearchPlan, SearchStage, SearchStatus, SearchTraceId, SourceLocation,
    StopConditions, StructureNodeId, TrustLabel,
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
    }
}

fn outcome() -> SearchOutcome {
    SearchOutcome {
        trace: SearchTraceId::new(37),
        fingerprint: RetrievalModelFingerprint::new("model:v1".to_owned())
            .expect("valid fingerprint"),
        index_generation: IndexGenerationId::new(13),
        status: SearchStatus::Answerable,
        evidence: vec![candidate()],
        coverage: EvidenceCoverage {
            percent_covered: 100,
            gaps_identified: Vec::new(),
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
