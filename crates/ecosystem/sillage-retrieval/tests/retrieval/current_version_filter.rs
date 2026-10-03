use sillage_domain::{ArtifactVersionId, EvidenceCandidate, SearchLaneStatus};
use sillage_retrieval::RetrievalResult;
use sillage_retrieval::adapters::CurrentVersionFilter;
use sillage_retrieval::traits::CandidateRetriever;
use sillage_retrieval::types::{
    CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor,
};
use std::collections::BTreeSet;

use super::common;

struct FixedRetriever {
    candidate: EvidenceCandidate,
    descriptor: RetrieverDescriptor,
}

impl CandidateRetriever for FixedRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        Ok(CandidateBatch {
            descriptor: (*self.descriptor()).clone(),
            query: request.query.q,
            candidates: vec![self.candidate.clone()],
            status: SearchLaneStatus::Succeeded,
            generation: Some(sillage_domain::IndexGenerationId::new(1)),
            execution: sillage_domain::SearchExecution::new(
                request.execution_budget,
                sillage_domain::SearchExecutionUsage::new(1, 1, 1, 0),
                sillage_domain::SearchExecutionCompletion::Complete,
            ),
        })
    }
}

fn request() -> RetrievalResult<CandidateRequest> {
    let plan = common::dummy_plan()?;
    let authorization = sillage_governance::RetrievalSecurityPolicy::default()
        .authorization_context(plan.scope())
        .map_err(|error| RetrievalError::Internal(format!("{error:?}")))?;
    let execution_budget = sillage_domain::SearchExecutionBudget::new(10, 10, 10, 0)
        .map_err(|error| RetrievalError::Internal(format!("{error:?}")))?;
    Ok(CandidateRequest {
        plan: std::sync::Arc::new(plan),
        query: sillage_ports::SearchQuery {
            q: "notes".to_string(),
            limit: 10,
            offset: 0,
            execution_budget,
        },
        execution_budget,
        expected_generation: sillage_domain::IndexGenerationId::new(1),
        authorization,
        source_filter: None,
        cancellation: None,
    })
}

fn filter() -> RetrievalResult<CurrentVersionFilter> {
    Ok(CurrentVersionFilter::new(
        std::sync::Arc::new(FixedRetriever {
            candidate: common::candidate_fixture()?,
            descriptor: RetrieverDescriptor {
                id: "fixed".to_string(),
                modality: "text".to_string(),
                representation: sillage_domain::RepresentationName::new("text"),
                generation: sillage_domain::IndexGenerationId::new(1),
            },
        }),
        BTreeSet::new(),
    ))
}

#[test]
fn empty_active_versions_fail_closed() -> RetrievalResult<()> {
    let batch = filter()?.retrieve(request()?)?;
    assert!(batch.candidates.is_empty());
    assert_eq!(batch.status, SearchLaneStatus::Empty);
    Ok(())
}

#[test]
fn active_versions_retain_matching_candidates() -> RetrievalResult<()> {
    let filtered = CurrentVersionFilter::new(
        std::sync::Arc::new(FixedRetriever {
            candidate: common::candidate_fixture()?,
            descriptor: RetrieverDescriptor {
                id: "fixed".to_string(),
                modality: "text".to_string(),
                representation: sillage_domain::RepresentationName::new("text"),
                generation: sillage_domain::IndexGenerationId::new(1),
            },
        }),
        BTreeSet::from([ArtifactVersionId::new(19)]),
    );
    let batch = filtered.retrieve(request()?)?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(
        batch.candidates[0].artifact_version(),
        ArtifactVersionId::new(19)
    );
    assert_eq!(batch.status, SearchLaneStatus::Succeeded);
    Ok(())
}
