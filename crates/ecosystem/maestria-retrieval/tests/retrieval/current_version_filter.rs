use async_trait::async_trait;
use maestria_domain::{ArtifactVersionId, EvidenceCandidate, SearchLaneStatus};
use maestria_retrieval::RetrievalResult;
use maestria_retrieval::adapters::CurrentVersionFilter;
use maestria_retrieval::traits::CandidateRetriever;
use maestria_retrieval::types::{
    CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor,
};
use std::collections::BTreeSet;

use super::common;

struct FixedRetriever {
    candidate: EvidenceCandidate,
}

#[async_trait]
impl CandidateRetriever for FixedRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        RetrieverDescriptor {
            id: "fixed".to_string(),
            modality: "text".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        }
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        Ok(CandidateBatch {
            descriptor: self.descriptor(),
            query: request.query.q,
            candidates: vec![self.candidate.clone()],
            status: SearchLaneStatus::Succeeded,
            generation: Some(maestria_domain::IndexGenerationId::new(1)),
            bytes_read: 0,
        })
    }
}

fn request() -> RetrievalResult<CandidateRequest> {
    let plan = common::dummy_plan()?;
    Ok(CandidateRequest {
        plan,
        query: maestria_ports::SearchQuery {
            q: "notes".to_string(),
            limit: 10,
            offset: 0,
        },
        expected_generation: maestria_domain::IndexGenerationId::new(1),
    })
}

fn filter() -> RetrievalResult<CurrentVersionFilter> {
    Ok(CurrentVersionFilter::new(
        std::sync::Arc::new(FixedRetriever {
            candidate: common::candidate_fixture()?,
        }),
        BTreeSet::new(),
    ))
}

#[tokio::test]
async fn empty_active_versions_fail_closed() -> RetrievalResult<()> {
    let batch = filter()?.retrieve(request()?).await?;
    assert!(batch.candidates.is_empty());
    assert_eq!(batch.status, SearchLaneStatus::Empty);
    Ok(())
}

#[tokio::test]
async fn active_versions_retain_matching_candidates() -> RetrievalResult<()> {
    let filtered = CurrentVersionFilter::new(
        std::sync::Arc::new(FixedRetriever {
            candidate: common::candidate_fixture()?,
        }),
        BTreeSet::from([ArtifactVersionId::new(19)]),
    );
    let batch = filtered.retrieve(request()?).await?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(
        batch.candidates[0].artifact_version,
        ArtifactVersionId::new(19)
    );
    assert_eq!(batch.status, SearchLaneStatus::Succeeded);
    Ok(())
}
