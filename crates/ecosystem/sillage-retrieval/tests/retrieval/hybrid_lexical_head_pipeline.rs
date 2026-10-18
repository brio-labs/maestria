use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use sillage_domain::{
    ArtifactVersionId, ContentRange, EvidenceCandidate, EvidenceCandidateDto, EvidenceCoverage,
    EvidenceCoverageDto, EvidenceId, EvidenceSpan, FreshnessStatus, IndexGenerationId,
    RetrievalReason, SearchExecution, SearchExecutionCompletion, SearchExecutionUsage,
    SearchIntent, SearchOutcome, SearchStage, SearchStatus, SearchTraceId, SourceLocation,
    StructureNodeId, TrustLabel,
};
use sillage_retrieval::{
    CandidateRetriever, ContextExpander, FixedKRrf, HybridExecutionPolicy, HybridLexicalHead,
    HybridPromotionRecord, RetrievalEngine, RetrievalError, RetrievalEvaluator, RetrievalResult,
    traits::CandidateReranker,
    types::{
        CandidateBatch, CandidateRequest, ContextExpansion, ExpansionPolicy, RankedCandidate,
        RerankRequest, RerankResult, RetrievalEvaluationReport, RetrievalExperiment,
    },
};

use crate::common::{fixture_scores, golden::query_plan};

fn candidate(
    id: u64,
    lexical_score: u32,
    dense_score: u32,
) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(id),
        artifact_version: ArtifactVersionId::new(id + 100),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(id)),
            SourceLocation::file(format!("source-{id}.md"), 1, 3)?,
            ContentRange::new(0, 12)?,
        )?,
        scores: fixture_scores(lexical_score, dense_score)?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: vec![format!("evidence-{id}")],
    })?)
}

fn descriptor_at(
    id: &str,
    modality: &str,
    generation: u64,
) -> sillage_retrieval::types::RetrieverDescriptor {
    sillage_retrieval::types::RetrieverDescriptor {
        id: id.to_string(),
        modality: modality.to_string(),
        representation: sillage_domain::RepresentationName::new(format!("{modality}_v1")),
        generation: IndexGenerationId::new(generation),
    }
}

struct StaticLane {
    descriptor: sillage_retrieval::types::RetrieverDescriptor,
    candidates: Vec<EvidenceCandidate>,
}

impl CandidateRetriever for StaticLane {
    fn descriptor(&self) -> &sillage_retrieval::types::RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(&self, request: CandidateRequest) -> RetrievalResult<CandidateBatch> {
        let count = self.candidates.len() as u64;
        Ok(CandidateBatch::succeeded(
            self.descriptor.clone(),
            request.query.q,
            self.candidates.clone(),
            Some(self.descriptor.generation),
            SearchExecution::new(
                request.execution_budget,
                SearchExecutionUsage::new(count, count, count, 0),
                SearchExecutionCompletion::Complete,
            ),
        ))
    }
}

struct RecordingReranker {
    seen: Arc<Mutex<Vec<EvidenceId>>>,
}

impl CandidateReranker for RecordingReranker {
    fn rerank(&self, request: RerankRequest) -> RetrievalResult<RerankResult> {
        let fingerprint = request.plan.fingerprint().clone();
        let mut candidates = request.candidates;
        let mut seen = self
            .seen
            .lock()
            .map_err(|_| RetrievalError::Internal("reranker test lock poisoned".to_string()))?;
        seen.extend(
            candidates
                .iter()
                .map(|candidate| candidate.candidate.evidence_id()),
        );
        drop(seen);
        candidates.reverse();
        let trace_candidates = candidates
            .iter_mut()
            .enumerate()
            .map(|(rank, candidate)| {
                let original_rank = candidate.rank;
                candidate.rank = rank;
                sillage_domain::SearchTraceRerankCandidate {
                    candidate_id: candidate.candidate.evidence_id(),
                    original_rank,
                    position: sillage_domain::RerankPosition::Reranked(rank),
                    relevance_score: Some(1),
                    constraint_scores: Vec::new(),
                }
            })
            .collect();
        Ok(RerankResult {
            candidates,
            trace: sillage_domain::SearchTraceRerank {
                model: "test-tail-reranker".to_string(),
                fingerprint,
                input_cap: 10,
                score_cap: 10,
                output_cap: 10,
                candidates: trace_candidates,
            },
        })
    }
}

struct ReverseExpander {
    calls: Arc<AtomicUsize>,
}

impl ContextExpander for ReverseExpander {
    fn expand(
        &self,
        candidates: &[RankedCandidate],
        policy: &ExpansionPolicy,
    ) -> RetrievalResult<ContextExpansion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut expanded = candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect::<Vec<_>>();
        expanded.reverse();
        Ok(ContextExpansion {
            candidates: expanded,
            execution: SearchExecution::new(
                policy.execution_budget,
                SearchExecutionUsage::default(),
                SearchExecutionCompletion::Complete,
            ),
        })
    }
}

struct RecordingEvaluator {
    reverse: bool,
}

impl RetrievalEvaluator for RecordingEvaluator {
    fn evaluate(
        &self,
        experiment: RetrievalExperiment,
    ) -> RetrievalResult<RetrievalEvaluationReport> {
        let mut evidence = experiment.candidates;
        if self.reverse {
            evidence.reverse();
        }
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else {
            SearchStatus::Answerable
        };
        let percent_covered = if evidence.is_empty() { 0 } else { 100 };
        let evaluated_candidates = evidence.len();
        Ok(RetrievalEvaluationReport {
            outcome: SearchOutcome {
                trace: SearchTraceId::new(0),
                trace_data: None,
                fingerprint: experiment.plan.fingerprint().clone(),
                index_generation: experiment.plan.index_generation(),
                status,
                evidence,
                coverage: EvidenceCoverage::new(EvidenceCoverageDto {
                    required_claims: Vec::new(),
                    required_subquestions: Vec::new(),
                    distinct_sources: 0,
                    distinct_documents: 0,
                    distinct_sections: 0,
                    candidate_coverage_keys: Vec::new(),
                    percent_covered,
                    gaps_identified: Vec::new(),
                })?,
                conflicts: Vec::new(),
            },
            evaluated_candidates,
        })
    }
}

fn hybrid_engine(
    stages: Vec<SearchStage>,
    reverse_evaluator: bool,
) -> Result<(RetrievalEngine, sillage_domain::SearchPlan), Box<dyn std::error::Error>> {
    let head = candidate(1, 100, 0)?;
    let tail = candidate(2, 80, 0)?;
    let semantic = candidate(3, 0, 90)?;
    let retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![
        Arc::new(StaticLane {
            descriptor: descriptor_at("lexical_primary", "text", 13),
            candidates: vec![head, tail.clone()],
        }),
        Arc::new(StaticLane {
            descriptor: descriptor_at("dense_semantic", "dense", 13),
            candidates: vec![tail, semantic],
        }),
    ];
    let security_policy = sillage_governance::RetrievalSecurityPolicy::new()
        .require_read_allowed(true)
        .allow_unscoped_items(true);
    let mut plan = query_plan(
        sillage_domain::QueryId::new(701),
        "test query",
        SearchIntent::FactualLocal,
        stages,
    )?;
    let authorization = security_policy.authorization_context(plan.scope())?;
    plan = plan.with_authorization(authorization.policy_snapshot()?)?;
    let served_classes = sillage_retrieval::LearnedSparseQueryClass::all()
        .into_iter()
        .collect();
    let promotion = HybridPromotionRecord::new(
        "fixture".to_string(),
        "test".to_string(),
        served_classes,
        sillage_retrieval::HYBRID_SERVING_POLICY_ID,
    )
    .ok_or("hybrid fixture promotion was rejected")?;
    let capabilities = sillage_governance::SearchCapabilities::core_defaults(
        sillage_domain::CorpusSnapshotId::new(11),
        IndexGenerationId::new(13),
        (1_000, 30_000),
    )
    .with_stage(SearchStage::Reranking)
    .with_stage(SearchStage::Filtering)
    .max_budgets(1_000, 30_000, 8, 3, 0);
    let engine = RetrievalEngine::new(
        retrievers,
        Arc::new(RecordingEvaluator {
            reverse: reverse_evaluator,
        }),
        security_policy,
    )
    .with_hybrid_policy(HybridExecutionPolicy::Active(promotion))
    .with_fusion(Arc::new(HybridLexicalHead::new(FixedKRrf::new(60))))
    .with_capabilities(capabilities);
    Ok((engine, plan))
}

#[test]
fn reranking_and_evaluator_reordering_cannot_displace_the_lexical_head()
-> Result<(), Box<dyn std::error::Error>> {
    let (engine, plan) = hybrid_engine(
        vec![SearchStage::InitialRetrieval, SearchStage::Reranking],
        true,
    )?;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let engine = engine.with_reranker(Arc::new(RecordingReranker {
        seen: Arc::clone(&seen),
    }));

    let outcome = engine.search(&plan)?;
    assert_eq!(outcome.evidence[0].evidence_id(), EvidenceId::new(1));
    assert!(
        !seen
            .lock()
            .map_err(|_| "reranker test lock poisoned")?
            .contains(&EvidenceId::new(1))
    );
    let trace = outcome
        .trace_data
        .as_deref()
        .ok_or("search trace missing")?;
    assert_eq!(
        trace.fusion.as_deref(),
        Some("hybrid-lexical-head-preserving-v1+fixed-k-rrf-v1:k=60")
    );
    let rerank_trace = trace.rerank.as_ref().ok_or("rerank trace missing")?;
    assert!(rerank_trace.candidates.iter().any(|candidate| {
        candidate.candidate_id == EvidenceId::new(1)
            && candidate.position == sillage_domain::RerankPosition::SkippedNotApplicable
    }));
    Ok(())
}

#[test]
fn diversity_expansion_cannot_displace_the_lexical_head() -> Result<(), Box<dyn std::error::Error>>
{
    let (engine, plan) = hybrid_engine(
        vec![SearchStage::InitialRetrieval, SearchStage::Filtering],
        false,
    )?;
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = engine.with_expander(Arc::new(ReverseExpander {
        calls: Arc::clone(&calls),
    }));

    let outcome = engine.search(&plan)?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.evidence[0].evidence_id(), EvidenceId::new(1));
    Ok(())
}

struct RewriteLane {
    descriptor: sillage_retrieval::types::RetrieverDescriptor,
    original: Vec<EvidenceCandidate>,
    rewritten: Vec<EvidenceCandidate>,
}

impl CandidateRetriever for RewriteLane {
    fn descriptor(&self) -> &sillage_retrieval::types::RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(&self, request: CandidateRequest) -> RetrievalResult<CandidateBatch> {
        let candidates = if request.query.q == "grant_trace" {
            &self.rewritten
        } else {
            &self.original
        };
        let count = candidates.len() as u64;
        let completion = if count == request.execution_budget.max_results() {
            SearchExecutionCompletion::Exhausted(sillage_domain::SearchExecutionResource::Results)
        } else {
            SearchExecutionCompletion::Complete
        };
        Ok(CandidateBatch::succeeded(
            self.descriptor.clone(),
            request.query.q,
            candidates.clone(),
            Some(self.descriptor.generation),
            SearchExecution::new(
                request.execution_budget,
                SearchExecutionUsage::new(count, count, count, 0),
                completion,
            ),
        ))
    }
}

#[test]
fn dense_lane_ceiling_cannot_starve_the_actual_lexical_baseline_head()
-> Result<(), Box<dyn std::error::Error>> {
    let initial_head = candidate(1, 100, 0)?;
    let rewritten_head = candidate(2, 80, 0)?;
    let dense_candidates = (3..=6)
        .map(|id| candidate(id, 0, 90))
        .collect::<Result<Vec<_>, _>>()?;
    let retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![
        Arc::new(RewriteLane {
            descriptor: descriptor_at("lexical_primary", "text", 13),
            original: vec![initial_head.clone(), rewritten_head.clone()],
            rewritten: vec![rewritten_head.clone()],
        }),
        Arc::new(RewriteLane {
            descriptor: descriptor_at("lexical_secondary", "text", 13),
            original: vec![initial_head],
            rewritten: vec![rewritten_head],
        }),
        Arc::new(RewriteLane {
            descriptor: descriptor_at("dense_semantic", "dense", 13),
            original: dense_candidates.clone(),
            rewritten: dense_candidates,
        }),
    ];
    let security_policy = sillage_governance::RetrievalSecurityPolicy::new()
        .require_read_allowed(true)
        .allow_unscoped_items(true);
    let mut plan = query_plan(
        sillage_domain::QueryId::new(702),
        "grant-trace",
        SearchIntent::ExactLookup,
        vec![SearchStage::InitialRetrieval],
    )?
    .with_budgets(sillage_domain::SearchBudget::with_resource_limits(
        1_000, 30_000, 2, 1, 0, 0, 1,
    )?)?
    .with_stop_conditions(sillage_domain::StopConditions {
        max_results: 4,
        min_score_threshold: 0,
    })?;
    let authorization = security_policy.authorization_context(plan.scope())?;
    plan = plan.with_authorization(authorization.policy_snapshot()?)?;
    let capabilities = sillage_governance::SearchCapabilities::core_defaults(
        sillage_domain::CorpusSnapshotId::new(11),
        IndexGenerationId::new(13),
        (1_000, 30_000),
    )
    .max_budgets(1_000, 30_000, 8, 3, 0);
    let make_engine = |policy| {
        RetrievalEngine::new(
            retrievers.clone(),
            Arc::new(RecordingEvaluator { reverse: false }),
            security_policy.clone(),
        )
        .with_hybrid_policy(policy)
        .with_fusion(Arc::new(HybridLexicalHead::new(FixedKRrf::new(60))))
        .with_capabilities(capabilities.clone())
    };
    let baseline = make_engine(HybridExecutionPolicy::Shadow).search(&plan)?;
    let promotion = HybridPromotionRecord::new(
        "fixture".to_string(),
        "test".to_string(),
        sillage_retrieval::LearnedSparseQueryClass::all()
            .into_iter()
            .collect(),
        sillage_retrieval::HYBRID_SERVING_POLICY_ID,
    )
    .ok_or("hybrid fixture promotion was rejected")?;
    let hybrid = make_engine(HybridExecutionPolicy::Active(promotion)).search(&plan)?;

    assert_eq!(baseline.evidence[0].evidence_id(), EvidenceId::new(2));
    assert_eq!(hybrid.evidence[0], baseline.evidence[0]);
    assert!(hybrid.evidence.len() <= 4);
    Ok(())
}
