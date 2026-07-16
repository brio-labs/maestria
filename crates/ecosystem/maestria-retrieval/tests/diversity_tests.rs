use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId,
    EvidenceCandidate, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    IndexGenerationId, ModalitySet, QueryId, RetrievalModelFingerprint, RetrievalScoreSet,
    SearchBudget, SearchIntent, SearchPlan, SearchStage, SearchStatus, SourceLocation,
    StopConditions, StructureNodeId, TrustLabel,
};
use maestria_retrieval::diversity::select_candidates;
use maestria_retrieval::types::RankedCandidate;

fn plan(requirements: EvidenceRequirements, max_results: u32) -> SearchPlan {
    SearchPlan {
        query_id: QueryId::new(1),
        original_query: "query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(100, 1_000).expect("valid fixture budget"),
        stop_conditions: StopConditions {
            max_results,
            min_score_threshold: 0,
        },
        evidence_requirements: requirements,
        fingerprint: RetrievalModelFingerprint::new("fixture-model".to_string())
            .expect("valid fixture fingerprint"),
    }
}

fn requirements() -> EvidenceRequirements {
    EvidenceRequirements {
        require_primary_sources: false,
        minimum_corroboration: 1,
        required_claims: vec![],
        required_subquestions: vec![],
        minimum_sources: 0,
        minimum_documents: 0,
        minimum_sections: 0,
    }
}

fn candidate(
    id: u64,
    artifact: u64,
    path: &str,
    node: u64,
    coverage_keys: &[&str],
    duplicate_cluster: Option<u64>,
    freshness: FreshnessStatus,
) -> RankedCandidate {
    RankedCandidate {
        rank: id as usize,
        candidate: EvidenceCandidate {
            evidence_id: maestria_domain::EvidenceId::new(id),
            artifact_version: ArtifactVersionId::new(artifact),
            source_span: EvidenceSpan::new(
                Some(StructureNodeId::new(node)),
                SourceLocation::File {
                    path: path.to_string(),
                    start_line: 1,
                    end_line: 2,
                },
                ContentRange { start: 0, end: 10 },
            )
            .expect("valid fixture span"),
            scores: RetrievalScoreSet {
                bm25: 100 - id as u32,
                semantic_similarity: 90 - id as u32,
            },
            trust: TrustLabel::Verified,
            freshness,
            duplicate_cluster: duplicate_cluster.map(DuplicateClusterId::new),
            reasons: vec![],
            coverage_keys: coverage_keys.iter().map(|key| (*key).to_string()).collect(),
        },
    }
}

#[test]
fn suppresses_duplicate_clusters_and_preserves_rank_order() {
    let candidates = vec![
        candidate(1, 1, "a.md", 1, &[], Some(9), FreshnessStatus::UpToDate),
        candidate(2, 2, "b.md", 2, &[], Some(9), FreshnessStatus::UpToDate),
        candidate(3, 3, "c.md", 3, &[], None, FreshnessStatus::UpToDate),
    ];
    let selection = select_candidates(&candidates, &plan(requirements(), 10));

    assert_eq!(selection.candidates.len(), 2);
    assert_eq!(selection.candidates[0].candidate.evidence_id.value(), 1);
    assert_eq!(selection.candidates[1].candidate.evidence_id.value(), 3);
    assert_eq!(selection.trace.candidates.len(), 3);
    assert_eq!(selection.trace.candidates[1].selected_rank, None);
}

#[test]
fn maps_required_coverage_and_enforces_independent_origins() {
    let mut required = requirements();
    required.required_claims = vec!["claim".to_string()];
    required.required_subquestions = vec!["subquestion".to_string()];
    required.minimum_sources = 2;
    required.minimum_documents = 2;
    required.minimum_sections = 2;
    let candidates = vec![
        candidate(1, 1, "a.md", 1, &["claim"], None, FreshnessStatus::UpToDate),
        candidate(
            2,
            2,
            "b.md",
            2,
            &["subquestion"],
            None,
            FreshnessStatus::UpToDate,
        ),
    ];

    let selection = select_candidates(&candidates, &plan(required, 10));

    assert_eq!(selection.status, SearchStatus::Answerable);
    assert_eq!(selection.coverage.percent_covered, 100);
    assert!(selection.coverage.gaps_identified.is_empty());
    assert_eq!(selection.coverage.distinct_sources, 2);
    assert_eq!(selection.coverage.distinct_documents, 2);
    assert_eq!(selection.coverage.distinct_sections, 2);
}

#[test]
fn stops_when_marginal_gain_is_zero_after_requirements_are_met() {
    let candidates = vec![
        candidate(1, 1, "a.md", 1, &[], None, FreshnessStatus::UpToDate),
        candidate(2, 1, "a.md", 1, &[], None, FreshnessStatus::UpToDate),
    ];
    let selection = select_candidates(&candidates, &plan(requirements(), 10));

    assert_eq!(selection.candidates.len(), 1);
    assert_eq!(
        selection.trace.stop_reason,
        maestria_domain::SearchStopReason::LowMarginalGain
    );
}

#[test]
fn returns_stale_and_empty_outcomes() {
    let stale = select_candidates(
        &[candidate(
            1,
            1,
            "a.md",
            1,
            &[],
            None,
            FreshnessStatus::Stale,
        )],
        &plan(requirements(), 10),
    );
    assert_eq!(stale.status, SearchStatus::StaleEvidenceOnly);

    let empty = select_candidates(&[], &plan(requirements(), 10));
    assert_eq!(empty.status, SearchStatus::NoEvidenceFound);
    assert_eq!(empty.coverage.percent_covered, 0);
}

#[test]
fn selection_and_trace_are_deterministic() {
    let candidates = vec![
        candidate(1, 1, "a.md", 1, &["claim"], None, FreshnessStatus::UpToDate),
        candidate(2, 2, "b.md", 2, &["sub"], None, FreshnessStatus::UpToDate),
    ];
    let mut required = requirements();
    required.required_claims = vec!["claim".to_string()];
    required.required_subquestions = vec!["sub".to_string()];
    let first = select_candidates(&candidates, &plan(required.clone(), 10));
    let second = select_candidates(&candidates, &plan(required, 10));

    assert_eq!(first, second);
}
