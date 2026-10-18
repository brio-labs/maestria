use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sillage_domain::{IndexGenerationId, RepresentationName};
use sillage_ports::SearchQuery;

use super::batch_is_eligible;
use crate::types::{HybridExecutionPolicy, HybridPromotionRecord, RetrieverDescriptor};
use crate::visual_benchmark::visual_lane_is_eligible;

fn descriptor(id: &str, modality: &str) -> RetrieverDescriptor {
    RetrieverDescriptor {
        id: id.to_string(),
        modality: modality.to_string(),
        representation: RepresentationName::new("test"),
        generation: IndexGenerationId::new(1),
    }
}

fn active_hybrid_policy() -> Result<HybridExecutionPolicy, &'static str> {
    let mut served = std::collections::BTreeSet::new();
    served.insert(crate::LearnedSparseQueryClass::DomainTerminology);
    let Some(record) = HybridPromotionRecord::new(
        "hybrid".to_string(),
        "2026-07-18".to_string(),
        served,
        crate::HYBRID_SERVING_POLICY_ID,
    ) else {
        return Err("valid test promotion record was rejected");
    };
    Ok(HybridExecutionPolicy::Active(record))
}

#[test]
fn repository_code_lane_is_shadowed_until_promoted_for_query_class() -> Result<(), &'static str> {
    let code = descriptor("code_intel_symbols", "code");
    let hybrid = active_hybrid_policy()?;
    assert!(!batch_is_eligible(&code, &hybrid, false, "needle"));
    assert!(batch_is_eligible(&code, &hybrid, true, "needle"));
    Ok(())
}

#[test]
fn dense_shadow_filter_remains_independent_of_repository_policy() -> Result<(), &'static str> {
    let dense = descriptor("dense", "text");
    assert!(!batch_is_eligible(
        &dense,
        &HybridExecutionPolicy::Shadow,
        true,
        "needle"
    ));
    assert!(batch_is_eligible(
        &dense,
        &active_hybrid_policy()?,
        true,
        "needle"
    ));
    Ok(())
}

#[test]
fn visual_lane_is_shadowed_until_a_winning_query_class_is_promoted() {
    let visual = descriptor("visual_page_regions", "image");
    let text = descriptor("lexical", "text");
    assert!(!visual_lane_is_eligible(&visual, false));
    assert!(visual_lane_is_eligible(&visual, true));
    assert!(visual_lane_is_eligible(&text, false));
}

struct StubRetriever {
    descriptor: RetrieverDescriptor,
}
impl crate::traits::CandidateRetriever for StubRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(
        &self,
        request: crate::types::CandidateRequest,
    ) -> Result<crate::types::CandidateBatch, crate::RetrievalError> {
        Ok(crate::types::CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q.clone(),
            candidates: Vec::new(),
            status: sillage_domain::SearchLaneStatus::Empty,
            generation: Some(self.descriptor.generation),
            execution: sillage_domain::SearchExecution::default(),
        })
    }
}

#[test]
fn dense_only_engine_claims_no_generation_and_plan_validation_fails_closed()
-> Result<(), &'static str> {
    let dense = std::sync::Arc::new(StubRetriever {
        descriptor: descriptor("dense_retriever", "dense"),
    });
    let capabilities = super::engine_capabilities::capabilities_from_retrievers(&[dense]);
    let plan = sillage_domain::SearchPlan::builder()
        .query_id(sillage_domain::QueryId::from_query_text("test query"))
        .original_query("test query".to_string())
        .intent(sillage_domain::SearchIntent::FactualLocal)
        .scope(sillage_domain::CorpusScope::Global)
        .corpus_snapshot(sillage_domain::DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(sillage_domain::IndexGenerationId::new(1))
        .freshness(sillage_domain::FreshnessRequirement::Any)
        .modalities(sillage_domain::ModalitySet::new(vec![
            sillage_domain::Modality::Text,
        ]))
        .stages(vec![sillage_domain::SearchStage::InitialRetrieval])
        .budgets(
            sillage_domain::SearchBudget::with_limits(1000, 1000, 8, 3, 0)
                .map_err(|_| "valid test budget was rejected")?,
        )
        .stop_conditions(sillage_domain::StopConditions {
            max_results: 10,
            min_score_threshold: 50,
        })
        .evidence_requirements(sillage_domain::EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(
            sillage_domain::RetrievalModelFingerprint::new("dummy-model".to_string())
                .map_err(|_| "valid fingerprint was rejected")?,
        )
        .authorization(sillage_domain::RetrievalPolicySnapshot::global_default())
        .build()
        .map_err(|_| "valid test plan was rejected")?;

    assert!(
        sillage_governance::SearchPlanValidator::validate(
            &plan,
            &capabilities,
            &sillage_governance::RetrievalSecurityPolicy::default(),
        )
        .is_err(),
        "a dense-only engine must not authorize plans against a fabricated generation"
    );
    Ok(())
}

struct EmptyRecordingRetriever {
    descriptor: RetrieverDescriptor,
    calls: Arc<AtomicUsize>,
}
impl crate::traits::CandidateRetriever for EmptyRecordingRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(
        &self,
        request: crate::types::CandidateRequest,
    ) -> Result<crate::types::CandidateBatch, crate::RetrievalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(crate::types::CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates: Vec::new(),
            status: sillage_domain::SearchLaneStatus::Empty,
            generation: Some(self.descriptor.generation),
            execution: sillage_domain::SearchExecution::new(
                request.execution_budget,
                sillage_domain::SearchExecutionUsage::default(),
                sillage_domain::SearchExecutionCompletion::Complete,
            ),
        })
    }
}

fn tight_execution_plan()
-> Result<sillage_domain::SearchPlan, sillage_domain::SearchCompatibilityError> {
    sillage_domain::SearchPlan::builder()
        .query_id(sillage_domain::QueryId::new(1))
        .original_query("needle".to_string())
        .intent(sillage_domain::SearchIntent::FactualLocal)
        .scope(sillage_domain::CorpusScope::Global)
        .corpus_snapshot(sillage_domain::DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(sillage_domain::IndexGenerationId::new(1))
        .freshness(sillage_domain::FreshnessRequirement::Any)
        .modalities(sillage_domain::ModalitySet::new(vec![
            sillage_domain::Modality::Text,
        ]))
        .stages(vec![sillage_domain::SearchStage::InitialRetrieval])
        .budgets(sillage_domain::SearchBudget::with_execution_limits(
            sillage_domain::SearchBudgetLimits {
                max_tokens: 1,
                max_latency_ms: 1,
                max_queries: 1,
                max_stages: 1,
                max_web_requests: 0,
                max_bytes_read: 0,
                max_concurrency: 1,
                max_candidates: 1,
                max_work_units: 1,
            },
        )?)
        .stop_conditions(sillage_domain::StopConditions {
            max_results: 1,
            min_score_threshold: 0,
        })
        .evidence_requirements(sillage_domain::EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(sillage_domain::RetrievalModelFingerprint::new(
            "sillage:test".to_string(),
        )?)
        .authorization(sillage_domain::RetrievalPolicySnapshot::global_default())
        .build()
}

#[test]
fn empty_tightly_budgeted_lane_releases_capacity_to_later_lane()
-> Result<(), Box<dyn std::error::Error>> {
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let retrievers: Vec<Arc<dyn crate::traits::CandidateRetriever>> = vec![
        Arc::new(EmptyRecordingRetriever {
            descriptor: descriptor("first", "text"),
            calls: first_calls.clone(),
        }),
        Arc::new(EmptyRecordingRetriever {
            descriptor: descriptor("second", "text"),
            calls: second_calls.clone(),
        }),
    ];
    let plan = tight_execution_plan()?;
    let authorization = sillage_governance::RetrievalSecurityPolicy::default()
        .authorization_context(plan.scope())?;
    let query = SearchQuery {
        q: "needle".to_string(),
        limit: 1,
        offset: 0,
        execution_budget: plan.execution_budget()?,
    };
    let mut web_requests_used = 0;
    let mut execution_usage = sillage_domain::SearchExecutionUsage::default();

    let batches = super::engine_pipeline::collect_batches(
        &retrievers,
        &plan,
        &query,
        &authorization,
        None,
        &mut web_requests_used,
        &mut execution_usage,
    )?;

    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(batches.len(), 2);
    assert_eq!(
        execution_usage,
        sillage_domain::SearchExecutionUsage::default()
    );
    Ok(())
}
struct OrderedFixtureRetriever {
    descriptor: RetrieverDescriptor,
    candidates: Vec<sillage_domain::EvidenceCandidate>,
}

impl crate::traits::CandidateRetriever for OrderedFixtureRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(
        &self,
        request: crate::types::CandidateRequest,
    ) -> Result<crate::types::CandidateBatch, crate::RetrievalError> {
        let candidates = self
            .candidates
            .iter()
            .take(request.query.limit)
            .cloned()
            .collect::<Vec<_>>();
        let count = sillage_domain::saturating_u64(candidates.len());
        let status = if candidates.is_empty() {
            sillage_domain::SearchLaneStatus::Empty
        } else {
            sillage_domain::SearchLaneStatus::Succeeded
        };
        Ok(crate::types::CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates,
            status,
            generation: Some(self.descriptor.generation),
            execution: sillage_domain::SearchExecution::new(
                request.execution_budget,
                sillage_domain::SearchExecutionUsage::new(count, count, count, 0),
                sillage_domain::SearchExecutionCompletion::Complete,
            ),
        })
    }
}

fn ranking_window_plan() -> Result<sillage_domain::SearchPlan, Box<dyn std::error::Error>> {
    let query = "how might amber kites navigate the northern orchard";
    Ok(sillage_domain::SearchPlan::builder()
        .query_id(sillage_domain::QueryId::from_query_text(query))
        .original_query(query.to_string())
        .intent(sillage_domain::SearchIntent::FactualLocal)
        .scope(sillage_domain::CorpusScope::Global)
        .corpus_snapshot(sillage_domain::DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(sillage_domain::IndexGenerationId::new(1))
        .freshness(sillage_domain::FreshnessRequirement::Any)
        .modalities(sillage_domain::ModalitySet::new(vec![
            sillage_domain::Modality::Text,
        ]))
        .stages(vec![sillage_domain::SearchStage::InitialRetrieval])
        .budgets(sillage_domain::SearchBudget::with_execution_limits(
            sillage_domain::SearchBudgetLimits {
                max_tokens: 128,
                max_latency_ms: 30_000,
                max_queries: 1,
                max_stages: 1,
                max_web_requests: 0,
                max_bytes_read: 0,
                max_concurrency: 2,
                max_candidates: 4,
                max_work_units: 100,
            },
        )?)
        .stop_conditions(sillage_domain::StopConditions {
            max_results: 1,
            min_score_threshold: 0,
        })
        .evidence_requirements(sillage_domain::EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        })
        .fingerprint(sillage_domain::RetrievalModelFingerprint::new(
            "sillage:test".to_string(),
        )?)
        .authorization(sillage_domain::RetrievalPolicySnapshot::global_default())
        .build()?)
}

fn ranked_fixture_candidate(
    id: u64,
    path: &str,
    raw_rank: u32,
) -> Result<sillage_domain::EvidenceCandidate, Box<dyn std::error::Error>> {
    let representation = RepresentationName::new("lexical_text_v1");
    let scores =
        sillage_domain::RetrievalScoreSet::new(vec![sillage_domain::RetrievalLaneScore::new(
            sillage_domain::RetrievalScoreKind::LexicalBm25,
            100,
            sillage_domain::RetrievalRawRank::ranked(raw_rank),
            sillage_domain::RetrievalScoreScale::unbounded("fixture_bm25"),
            representation.clone(),
            sillage_domain::RetrievalScoreFingerprint::new(
                sillage_domain::RetrievalModelFingerprint::new(
                    "fixture:bounded-window:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        )])?;
    Ok(sillage_domain::EvidenceCandidate::new(
        sillage_domain::EvidenceCandidateDto {
            evidence_id: sillage_domain::EvidenceId::new(id),
            artifact_version: sillage_domain::ArtifactVersionId::new(id),
            source_span: sillage_domain::EvidenceSpan::new(
                None,
                sillage_domain::SourceLocation::file(path.to_string(), 1, 1)?,
                sillage_domain::ContentRange::new(0, 1)?,
            )?,
            scores,
            trust: sillage_domain::TrustLabel::Verified,
            freshness: sillage_domain::FreshnessStatus::UpToDate,
            duplicate_cluster: None,
            reasons: vec![sillage_domain::RetrievalReason::LexicalMatch],
            coverage_keys: Vec::new(),
        },
    )?)
}

#[test]
fn final_result_ceiling_does_not_hide_cross_lane_consensus_before_fusion()
-> Result<(), Box<dyn std::error::Error>> {
    let first = ranked_fixture_candidate(1, "records/first-ranked.md", 1)?;
    let second = ranked_fixture_candidate(2, "records/alternate-rank.md", 1)?;
    let shared = ranked_fixture_candidate(3, "records/corroborated-match.md", 2)?;
    let retrievers: Vec<Arc<dyn crate::traits::CandidateRetriever>> = vec![
        Arc::new(OrderedFixtureRetriever {
            descriptor: descriptor("cards", "text"),
            candidates: vec![first, shared.clone()],
        }),
        Arc::new(OrderedFixtureRetriever {
            descriptor: descriptor("lexical_chunks", "text"),
            candidates: vec![second, shared],
        }),
    ];
    let engine = super::RetrievalEngine::new(
        retrievers,
        Arc::new(crate::adapters::EvidenceOutcomeEvaluator::new(Arc::new(
            sillage_ports::InMemoryEvidenceRepository::new(),
        ))),
        sillage_governance::RetrievalSecurityPolicy::default(),
    )
    .with_fusion(Arc::new(crate::HybridLexicalHead::new(
        crate::FixedKRrf::new(60),
    )));
    let plan = ranking_window_plan()?;
    let authorization = sillage_governance::RetrievalSecurityPolicy::default()
        .authorization_context(plan.scope())?;

    let outcome = engine.search_pre_authorized(&plan, authorization)?;

    assert_eq!(outcome.evidence.len(), 1);
    assert_eq!(
        outcome.evidence[0].evidence_id(),
        sillage_domain::EvidenceId::new(3),
        "the shared rank-2 lexical result should win after fusion, before the one-result final cap"
    );
    Ok(())
}
