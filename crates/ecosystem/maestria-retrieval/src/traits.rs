use async_trait::async_trait;
use maestria_ports::SearchQuery;

use crate::types::{
    CandidateBatch, CandidateRequest, ExpansionPolicy, FusedCandidate, RankedCandidate,
    RerankRequest, RerankResult, RerankScoreComponents, RerankScorerInput, RetrievalError,
    RetrievalEvaluationReport, RetrievalExperiment,
};

/// A retriever is a security boundary: implementations must apply the
/// configured scope, ACL, trust, sensitivity, quarantine, and prompt-injection
/// filters before returning candidates.
#[async_trait]
pub trait CandidateRetriever: Send + Sync {
    fn descriptor(&self) -> crate::types::RetrieverDescriptor;

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError>;
}

pub trait RankFusion: Send + Sync {
    fn fuse(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
    ) -> Result<Vec<FusedCandidate>, RetrievalError>;
}

#[async_trait]
pub trait CandidateReranker: Send + Sync {
    async fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError>;
}

#[async_trait]
pub trait RerankScorer: Send + Sync {
    fn model(&self) -> String;
    fn fingerprint(&self) -> maestria_domain::RetrievalModelFingerprint;
    fn compatible_with(&self, plan: &maestria_domain::RetrievalModelFingerprint) -> bool;
    async fn score(
        &self,
        input: RerankScorerInput,
    ) -> Result<RerankScoreComponents, RetrievalError>;
}

pub trait ContextExpander: Send + Sync {
    fn expand(
        &self,
        candidates: &[RankedCandidate],
        policy: &ExpansionPolicy,
    ) -> Result<Vec<maestria_domain::EvidenceCandidate>, RetrievalError>;
}

#[async_trait]
pub trait RetrievalEvaluator: Send + Sync {
    async fn evaluate(
        &self,
        experiment: RetrievalExperiment,
    ) -> Result<RetrievalEvaluationReport, RetrievalError>;
}
