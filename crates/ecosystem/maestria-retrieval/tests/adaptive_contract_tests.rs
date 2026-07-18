use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, EvidenceCandidate,
    EvidenceCoverage, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint, RetrievalReason,
    RetrievalScoreSet, SearchBudget, SearchIntent, SearchOutcome, SearchPlan, SearchStage,
    SearchStatus, SearchTraceId, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};
use maestria_retrieval::{
    CandidateRetriever, RetrievalEngine, RetrievalError, RetrievalEvaluator, RetrievalResult,
};
use std::sync::Arc;

fn candidate_fixture() -> RetrievalResult<EvidenceCandidate> {
    Ok(EvidenceCandidate {
        coverage_keys: vec![],
        evidence_id: maestria_domain::EvidenceId::new(23),
        artifact_version: ArtifactVersionId::new(19),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(29)),
            SourceLocation::File {
                path: "notes/research.md".to_string(),
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
        duplicate_cluster: Some(maestria_domain::DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
    })
}

fn adaptive_plan(max_queries: u32, max_stages: u32) -> RetrievalResult<SearchPlan> {
    let mut plan = SearchPlan {
        query_id: QueryId::new(1),
        original_query: "test query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 100)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 50,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: vec!["slot".to_string()],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("dummy-model".into())?,
    };
    plan.budgets = SearchBudget::with_limits(1000, 1000, max_queries, max_stages, 0)?;
    Ok(plan)
}

struct AdaptiveLane {
    slot_only: bool,
    stale_generation: bool,
}

#[async_trait]
impl CandidateRetriever for AdaptiveLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "adaptive".to_string(),
            modality: "text".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        }
    }

    async fn retrieve(
        &self,
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        let returns_candidate = !self.slot_only || request.query.q.contains("slot");
        let mut candidates = Vec::new();
        if returns_candidate {
            let mut candidate = candidate_fixture()?;
            if self.slot_only {
                candidate.coverage_keys = vec!["slot".to_string()];
            } else {
                candidate.coverage_keys.clear();
            }
            candidates.push(candidate);
        }
        let generation = if self.stale_generation {
            IndexGenerationId::new(999)
        } else {
            request.expected_generation
        };
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            query: request.query.q.clone(),
            candidates,
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(generation),
            bytes_read: 0,
        })
    }
}

struct AdaptiveEvaluator;

#[async_trait]
impl RetrievalEvaluator for AdaptiveEvaluator {
    async fn evaluate(
        &self,
        experiment: maestria_retrieval::types::RetrievalExperiment,
    ) -> RetrievalResult<maestria_retrieval::types::RetrievalEvaluationReport> {
        let evidence = experiment.candidates;
        let covered = evidence
            .iter()
            .flat_map(|candidate| candidate.coverage_keys.iter())
            .any(|key| key == "slot");
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else if covered {
            SearchStatus::Answerable
        } else {
            SearchStatus::EvidenceIncomplete
        };
        let gaps_identified = if covered {
            Vec::new()
        } else {
            vec!["slot".to_string()]
        };
        Ok(maestria_retrieval::types::RetrievalEvaluationReport {
            evaluated_candidates: evidence.len(),
            outcome: SearchOutcome {
                trace: SearchTraceId::new(1),
                trace_data: None,
                fingerprint: experiment.plan.fingerprint.clone(),
                index_generation: experiment.plan.index_generation,
                status,
                evidence,
                coverage: EvidenceCoverage {
                    required_claims: vec!["slot".to_string()],
                    required_subquestions: vec![],
                    distinct_sources: 0,
                    distinct_documents: 0,
                    distinct_sections: 0,
                    candidate_coverage_keys: vec![],
                    percent_covered: if covered { 100 } else { 0 },
                    gaps_identified,
                },
                conflicts: vec![],
            },
        })
    }
}

#[tokio::test]
async fn bounded_search_retrieves_declared_missing_slot() -> RetrievalResult<()> {
    let plan = adaptive_plan(3, 2)?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(AdaptiveLane {
            slot_only: true,
            stale_generation: false,
        })],
        Arc::new(AdaptiveEvaluator),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.status, SearchStatus::Answerable);
    assert_eq!(outcome.evidence.len(), 1);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::EvidenceComplete
    );
    assert!(trace.rewrites.iter().any(|rewrite| {
        rewrite.origin == maestria_domain::SearchRewriteOrigin::MissingSlot
            && rewrite.missing_slot.as_deref() == Some("slot")
    }));
    Ok(())
}

#[tokio::test]
async fn bounded_search_reports_budget_exhaustion() -> RetrievalResult<()> {
    let plan = adaptive_plan(1, 1)?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(AdaptiveLane {
            slot_only: true,
            stale_generation: false,
        })],
        Arc::new(AdaptiveEvaluator),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.status, SearchStatus::NoEvidenceFound);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::BudgetExhausted
    );
    Ok(())
}

#[tokio::test]
async fn bounded_search_stops_on_low_marginal_gain() -> RetrievalResult<()> {
    let plan = adaptive_plan(3, 2)?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(AdaptiveLane {
            slot_only: false,
            stale_generation: false,
        })],
        Arc::new(AdaptiveEvaluator),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.status, SearchStatus::EvidenceIncomplete);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::LowMarginalGain
    );
    Ok(())
}

#[tokio::test]
async fn bounded_search_rejects_stale_generation_results() -> RetrievalResult<()> {
    let plan = adaptive_plan(3, 2)?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(AdaptiveLane {
            slot_only: true,
            stale_generation: true,
        })],
        Arc::new(AdaptiveEvaluator),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.status, SearchStatus::NoEvidenceFound);
    assert!(outcome.evidence.is_empty());
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert!(trace.lanes.iter().all(|lane| {
        matches!(
            lane.status,
            maestria_domain::SearchLaneStatus::Failed { .. }
        )
    }));
    Ok(())
}
