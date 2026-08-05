use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use maestria_domain::{IndexGenerationId, RepresentationName};
use maestria_ports::SearchQuery;

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
    let Some(record) = HybridPromotionRecord::new("hybrid".to_string(), "2026-07-18".to_string())
    else {
        return Err("valid test promotion record was rejected");
    };
    Ok(HybridExecutionPolicy::Active(record))
}

#[test]
fn repository_code_lane_is_shadowed_until_promoted_for_query_class() -> Result<(), &'static str> {
    let code = descriptor("code_intel_symbols", "code");
    let hybrid = active_hybrid_policy()?;
    assert!(!batch_is_eligible(&code, &hybrid, false));
    assert!(batch_is_eligible(&code, &hybrid, true));
    Ok(())
}

#[test]
fn dense_shadow_filter_remains_independent_of_repository_policy() -> Result<(), &'static str> {
    let dense = descriptor("dense", "text");
    assert!(!batch_is_eligible(
        &dense,
        &HybridExecutionPolicy::Shadow,
        true
    ));
    assert!(batch_is_eligible(&dense, &active_hybrid_policy()?, true));
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

#[async_trait::async_trait]
impl crate::traits::CandidateRetriever for StubRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(
        &self,
        request: crate::types::CandidateRequest,
    ) -> Result<crate::types::CandidateBatch, crate::RetrievalError> {
        Ok(crate::types::CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q.clone(),
            candidates: Vec::new(),
            status: maestria_domain::SearchLaneStatus::Empty,
            generation: Some(self.descriptor.generation),
            execution: maestria_domain::SearchExecution::default(),
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
    let plan = maestria_domain::SearchPlan::builder()
        .query_id(maestria_domain::QueryId::from_query_text("test query"))
        .original_query("test query".to_string())
        .intent(maestria_domain::SearchIntent::FactualLocal)
        .scope(maestria_domain::CorpusScope::Global)
        .corpus_snapshot(maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(maestria_domain::IndexGenerationId::new(1))
        .freshness(maestria_domain::FreshnessRequirement::Any)
        .modalities(maestria_domain::ModalitySet::new(vec![
            maestria_domain::Modality::Text,
        ]))
        .stages(vec![maestria_domain::SearchStage::InitialRetrieval])
        .budgets(
            maestria_domain::SearchBudget::with_limits(1000, 1000, 8, 3, 0)
                .map_err(|_| "valid test budget was rejected")?,
        )
        .stop_conditions(maestria_domain::StopConditions {
            max_results: 10,
            min_score_threshold: 50,
        })
        .evidence_requirements(maestria_domain::EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(
            maestria_domain::RetrievalModelFingerprint::new("dummy-model".to_string())
                .map_err(|_| "valid fingerprint was rejected")?,
        )
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()
        .map_err(|_| "valid test plan was rejected")?;

    assert!(
        maestria_governance::SearchPlanValidator::validate(
            &plan,
            &capabilities,
            &maestria_governance::RetrievalSecurityPolicy::default(),
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

#[async_trait::async_trait]
impl crate::traits::CandidateRetriever for EmptyRecordingRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(
        &self,
        request: crate::types::CandidateRequest,
    ) -> Result<crate::types::CandidateBatch, crate::RetrievalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(crate::types::CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates: Vec::new(),
            status: maestria_domain::SearchLaneStatus::Empty,
            generation: Some(self.descriptor.generation),
            execution: maestria_domain::SearchExecution::new(
                request.execution_budget,
                maestria_domain::SearchExecutionUsage::default(),
                maestria_domain::SearchExecutionCompletion::Complete,
            ),
        })
    }
}

fn tight_execution_plan()
-> Result<maestria_domain::SearchPlan, maestria_domain::SearchCompatibilityError> {
    maestria_domain::SearchPlan::builder()
        .query_id(maestria_domain::QueryId::new(1))
        .original_query("needle".to_string())
        .intent(maestria_domain::SearchIntent::FactualLocal)
        .scope(maestria_domain::CorpusScope::Global)
        .corpus_snapshot(maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(maestria_domain::IndexGenerationId::new(1))
        .freshness(maestria_domain::FreshnessRequirement::Any)
        .modalities(maestria_domain::ModalitySet::new(vec![
            maestria_domain::Modality::Text,
        ]))
        .stages(vec![maestria_domain::SearchStage::InitialRetrieval])
        .budgets(maestria_domain::SearchBudget::with_execution_limits(
            maestria_domain::SearchBudgetLimits {
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
        .stop_conditions(maestria_domain::StopConditions {
            max_results: 1,
            min_score_threshold: 0,
        })
        .evidence_requirements(maestria_domain::EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(maestria_domain::RetrievalModelFingerprint::new(
            "maestria:test".to_string(),
        )?)
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()
}

#[tokio::test]
async fn empty_tightly_budgeted_lane_releases_capacity_to_later_lane()
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
    let authorization = maestria_governance::RetrievalSecurityPolicy::default()
        .authorization_context(plan.scope())?;
    let query = SearchQuery {
        q: "needle".to_string(),
        limit: 1,
        offset: 0,
        execution_budget: plan.execution_budget()?,
    };
    let mut web_requests_used = 0;
    let mut execution_usage = maestria_domain::SearchExecutionUsage::default();

    let batches = super::engine_pipeline::collect_batches(
        &retrievers,
        &plan,
        &query,
        &authorization,
        &mut web_requests_used,
        &mut execution_usage,
    )
    .await?;

    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    assert_eq!(batches.len(), 2);
    assert_eq!(
        execution_usage,
        maestria_domain::SearchExecutionUsage::default()
    );
    Ok(())
}
