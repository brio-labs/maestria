use sillage_ports::SearchQuery;

use crate::types::{
    CandidateBatch, CandidateRequest, ContextExpansion, ExpansionPolicy, FusedCandidate,
    FusionOutput, RankedCandidate, RerankRequest, RerankResult, RerankScoreComponents,
    RerankScorerInput, RetrievalError, RetrievalEvaluationReport, RetrievalExperiment,
};

/// A retriever is a security boundary: implementations must apply the
/// configured scope, ACL, trust, sensitivity, quarantine, and prompt-injection
/// filters before returning candidates.
pub trait CandidateRetriever: Send + Sync {
    fn descriptor(&self) -> &crate::types::RetrieverDescriptor;

    fn sparse_namespace(&self) -> Option<sillage_domain::SparseNamespace> {
        None
    }

    fn sparse_identity(&self) -> Option<sillage_ports::SparseIdentity> {
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
    /// A reported protected head must be the exact first candidate; otherwise
    /// wrappers may repair ordering, metadata, and score provenance from input.
    fn fuse_with_protected_head(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
        _protected_lexical_head: Option<&sillage_domain::EvidenceCandidate>,
    ) -> Result<FusionOutput, RetrievalError> {
        Ok(FusionOutput {
            candidates: self.fuse(query, batches)?,
            protected_lexical_head: None,
        })
    }

    /// Fuses all lanes while retaining the first eligible result from the
    /// lexical-only Fixed-K RRF baseline. Implementors may combine both
    /// rankings in one pass; the default computes the baseline without
    /// materializing or cloning candidate batches.
    fn fuse_with_lexical_baseline_head(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
        protected_head: Option<&sillage_domain::EvidenceCandidate>,
    ) -> Result<(FusionOutput, Option<sillage_domain::EvidenceCandidate>), RetrievalError> {
        let head = match protected_head {
            Some(head) => Some(head.clone()),
            None => crate::fusion::FixedKRrf::new(60).lexical_baseline_head(batches)?,
        };
        let output = self.fuse_with_protected_head(query, batches, head.as_ref())?;
        Ok((output, head))
    }

    /// Stable policy identity recorded in `SearchTrace.fusion`.
    fn trace_identity(&self) -> String {
        let mut identity = String::with_capacity(64);
        self.append_trace_identity(&mut identity);
        identity
    }

    /// Appends the identity without allocating intermediate strings.
    ///
    /// Implementors with a stable policy should override this; the default
    /// keeps the legacy generic marker for custom fusions.
    fn append_trace_identity(&self, identity: &mut String) {
        identity.push_str("configured");
    }
}

impl<T: RankFusion + ?Sized> RankFusion for std::sync::Arc<T> {
    fn fuse(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
    ) -> Result<Vec<FusedCandidate>, RetrievalError> {
        (**self).fuse(query, batches)
    }

    fn fuse_with_protected_head(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
        protected_lexical_head: Option<&sillage_domain::EvidenceCandidate>,
    ) -> Result<FusionOutput, RetrievalError> {
        (**self).fuse_with_protected_head(query, batches, protected_lexical_head)
    }

    fn fuse_with_lexical_baseline_head(
        &self,
        query: &SearchQuery,
        batches: &[CandidateBatch],
        protected_head: Option<&sillage_domain::EvidenceCandidate>,
    ) -> Result<(FusionOutput, Option<sillage_domain::EvidenceCandidate>), RetrievalError> {
        (**self).fuse_with_lexical_baseline_head(query, batches, protected_head)
    }

    fn append_trace_identity(&self, identity: &mut String) {
        (**self).append_trace_identity(identity);
    }
}

pub trait CandidateReranker: Send + Sync {
    fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError>;
}

pub trait RerankScorer: Send + Sync {
    fn model(&self) -> String;
    fn fingerprint(&self) -> sillage_domain::RetrievalModelFingerprint;
    fn compatible_with(&self, plan: &sillage_domain::RetrievalModelFingerprint) -> bool;
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
