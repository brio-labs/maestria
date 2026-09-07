use maestria_ports::SearchQuery;

use crate::types::{
    CandidateBatch, CandidateRequest, ContextExpansion, ExpansionPolicy, FusedCandidate,
    RankedCandidate, RerankRequest, RerankResult, RerankScoreComponents, RerankScorerInput,
    RetrievalError, RetrievalEvaluationReport, RetrievalExperiment,
};

/// A retriever is a security boundary: implementations must apply the
/// configured scope, ACL, trust, sensitivity, quarantine, and prompt-injection
/// filters before returning candidates.
pub trait CandidateRetriever: Send + Sync {
    fn descriptor(&self) -> &crate::types::RetrieverDescriptor;

    fn sparse_namespace(&self) -> Option<maestria_domain::SparseNamespace> {
        None
    }

    fn sparse_identity(&self) -> Option<maestria_ports::SparseIdentity> {
        None
    }
    fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError>;
}

pub trait RankFusion: Send + Sync {
    fn fuse(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
    ) -> Result<Vec<FusedCandidate>, RetrievalError>;
}

pub trait CandidateReranker: Send + Sync {
    fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError>;
}

pub trait RerankScorer: Send + Sync {
    fn model(&self) -> String;
    fn fingerprint(&self) -> maestria_domain::RetrievalModelFingerprint;
    fn compatible_with(&self, plan: &maestria_domain::RetrievalModelFingerprint) -> bool;
    fn score(&self, input: RerankScorerInput) -> Result<RerankScoreComponents, RetrievalError>;
}

pub trait ContextExpander: Send + Sync {
    fn expand(
        &self,
        candidates: &[RankedCandidate],
        policy: &ExpansionPolicy,
    ) -> Result<ContextExpansion, RetrievalError>;
}

pub trait RetrievalEvaluator: Send + Sync {
    fn evaluate(
        &self,
        experiment: RetrievalExperiment,
    ) -> Result<RetrievalEvaluationReport, RetrievalError>;
}
