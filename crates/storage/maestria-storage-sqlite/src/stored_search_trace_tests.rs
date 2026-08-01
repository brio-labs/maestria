//! Shared `#[cfg(test)]` fixtures for the stored search trace mirrors:
//! domain-side sample builders used by the facade round-trip tests and the
//! lane execution-budget rejection test. Compiled only under `cfg(test)`.

use std::collections::BTreeMap;

use maestria_domain::{
    ArtifactVersionId, ConflictSetId, ContentRange, CorpusScope, CorpusSnapshotId,
    DuplicateClusterId, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, Modality, ModalitySet, QueryId, RepresentationName,
    RerankCandidateStatus, RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank,
    RetrievalReason, RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale,
    RetrievalScoreSet, SearchBudget, SearchBudgetLimits, SearchExecution, SearchExecutionBudget,
    SearchExecutionCompletion, SearchExecutionUsage, SearchIntent, SearchLaneStatus,
    SearchRewriteAccounting, SearchRewriteOrigin, SearchRewriteStage, SearchStage,
    SearchStopReason, SearchTrace, SearchTraceCandidate, SearchTraceConstraintScore,
    SearchTraceDiversity, SearchTraceDiversityCandidate, SearchTraceExpansion, SearchTraceFilter,
    SearchTraceLane, SearchTraceLaneCandidate, SearchTraceRerank, SearchTraceRerankCandidate,
    SearchTraceRewrite, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};

pub(crate) fn sample_fingerprint() -> Result<RetrievalModelFingerprint, Box<dyn std::error::Error>>
{
    Ok(RetrievalModelFingerprint::new(
        "model:retrieval:v1".to_owned(),
    )?)
}

pub(crate) fn sample_span() -> Result<EvidenceSpan, Box<dyn std::error::Error>> {
    Ok(EvidenceSpan::new(
        Some(StructureNodeId::new(3)),
        SourceLocation::File {
            path: "docs/guide.md".to_owned(),
            start_line: 1,
            end_line: 12,
        },
        ContentRange { start: 0, end: 256 },
    )?)
}

pub(crate) fn sample_score_set() -> Result<RetrievalScoreSet, Box<dyn std::error::Error>> {
    Ok(RetrievalScoreSet::new(vec![RetrievalLaneScore::new(
        RetrievalScoreKind::Exact,
        1,
        RetrievalRawRank::Ranked { rank: 1 },
        RetrievalScoreScale::Binary,
        RepresentationName::new("text"),
        RetrievalScoreFingerprint::new(sample_fingerprint()?, BTreeMap::new()),
    )])?)
}

pub(crate) fn sample_execution() -> Result<SearchExecution, Box<dyn std::error::Error>> {
    Ok(SearchExecution::new(
        SearchExecutionBudget::with_byte_limit(20, 100, 5_000, None)?,
        SearchExecutionUsage::new(5, 10, 42, 0),
        SearchExecutionCompletion::Complete,
    ))
}

pub(crate) fn sample_trace() -> Result<SearchTrace, Box<dyn std::error::Error>> {
    let fingerprint = sample_fingerprint()?;
    Ok(SearchTrace {
        query_id: QueryId::new(42),
        original_query: "how does routing work".to_owned(),
        intent: SearchIntent::MultiHop,
        original_intent: Some(SearchIntent::FactualLocal),
        unavailable_capability: Some("visual".to_owned()),
        route_decision: Some("default".to_owned()),
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(7),
        index_generation: IndexGenerationId::new(2),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text, Modality::Code]),
        degradation: Some("visual->text".to_owned()),
        stages: vec![
            SearchStage::InitialRetrieval,
            SearchStage::Reranking,
            SearchStage::Filtering,
            SearchStage::Synthesis,
        ],
        budgets: SearchBudget::with_execution_limits(SearchBudgetLimits {
            max_tokens: 4096,
            max_latency_ms: 30_000,
            max_queries: 4,
            max_stages: 4,
            max_web_requests: 8,
            max_bytes_read: 1_048_576,
            max_concurrency: 2,
            max_candidates: 100,
            max_work_units: 50_000,
        })?,
        stop_conditions: StopConditions {
            max_results: 20,
            min_score_threshold: 500,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: true,
            minimum_corroboration: 1,
            required_claims: vec!["claim-a".to_owned()],
            required_subquestions: vec!["sub-q".to_owned()],
            minimum_sources: 2,
            minimum_documents: 1,
            minimum_sections: 1,
        },
        fingerprint: fingerprint.clone(),
        identity_version: 7,
        retrievers: vec!["bm25".to_owned(), "vector".to_owned()],
        policy_fingerprint: Some("policy:v2".to_owned()),
        raw_candidates: vec![sample_trace_candidate()?],
        fusion: Some("rrf".to_owned()),
        filters: vec![
            SearchTraceFilter::Scope,
            SearchTraceFilter::Sensitivity,
            SearchTraceFilter::PromptInjection,
        ],
        expansions: vec![SearchTraceExpansion {
            strategy: "synonym".to_owned(),
            added_candidates: Some(5),
        }],
        rewrites: vec![SearchTraceRewrite {
            query: "how does routing work".to_owned(),
            origin: SearchRewriteOrigin::Deterministic,
            stage: SearchRewriteStage::IterativeRetrieval,
            accounting: SearchRewriteAccounting {
                token_estimate: 16,
                latency_budget_units: 2,
                is_proposal: false,
            },
            missing_slot: Some("visual".to_owned()),
        }],
        missing_evidence: vec!["claim-b".to_owned()],
        conflicts: vec![ConflictSetId::new(9)],
        stop_reason: SearchStopReason::EvidenceComplete,
        lanes: vec![sample_trace_lane()?],
        rerank: Some(sample_trace_rerank(fingerprint)),
        diversity: Some(sample_trace_diversity()),
    })
}

pub(crate) fn sample_trace_candidate() -> Result<SearchTraceCandidate, Box<dyn std::error::Error>> {
    Ok(SearchTraceCandidate {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(11),
        source_span: sample_span()?,
        rank: 0,
        scores: sample_score_set()?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(5)),
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: vec!["k-1".to_owned()],
    })
}

pub(crate) fn sample_trace_lane() -> Result<SearchTraceLane, Box<dyn std::error::Error>> {
    Ok(SearchTraceLane {
        retriever_id: "bm25".to_owned(),
        query: "routing".to_owned(),
        generation: Some(IndexGenerationId::new(2)),
        status: SearchLaneStatus::Succeeded,
        candidates: vec![SearchTraceLaneCandidate {
            evidence_id: EvidenceId::new(1),
            artifact_version: ArtifactVersionId::new(11),
            source_span: sample_span()?,
            lane_rank: 0,
            duplicate_cluster: Some(DuplicateClusterId::new(5)),
            scores: sample_score_set()?,
            reasons: vec![RetrievalReason::LexicalMatch],
        }],
        execution: sample_execution()?,
    })
}

pub(crate) fn sample_trace_rerank(fingerprint: RetrievalModelFingerprint) -> SearchTraceRerank {
    SearchTraceRerank {
        model: "cross-encoder".to_owned(),
        fingerprint,
        input_cap: 100,
        score_cap: 50,
        output_cap: 20,
        candidates: vec![SearchTraceRerankCandidate {
            candidate_id: EvidenceId::new(1),
            original_rank: 0,
            new_rank: Some(0),
            status: RerankCandidateStatus::Reranked,
            relevance_score: Some(95),
            constraint_score: Some(80),
            constraint_scores: vec![SearchTraceConstraintScore {
                name: "freshness".to_owned(),
                score: 80,
            }],
        }],
    }
}

pub(crate) fn sample_trace_diversity() -> SearchTraceDiversity {
    SearchTraceDiversity {
        distinct_sources: 3,
        distinct_documents: 2,
        distinct_sections: 4,
        required_claims: vec!["claim-a".to_owned()],
        required_subquestions: vec!["sub-q".to_owned()],
        covered_keys: vec!["k-1".to_owned()],
        stop_reason: SearchStopReason::EvidenceComplete,
        candidates: vec![SearchTraceDiversityCandidate {
            candidate_id: EvidenceId::new(1),
            original_rank: 0,
            selected_rank: Some(0),
            duplicate_cluster: Some(DuplicateClusterId::new(5)),
            marginal_coverage: 3,
            coverage_keys: vec!["k-1".to_owned()],
        }],
    }
}
