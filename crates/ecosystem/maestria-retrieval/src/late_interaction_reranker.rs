use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock},
};

use maestria_domain::{
    EvidenceCandidate, RerankPosition, RetrievalModelFingerprint, RetrievalReason, SearchIntent,
    SearchTraceRerank, SearchTraceRerankCandidate, SourceLocation,
};
use maestria_governance::RetrievalAuthorizationContext;
use maestria_ports::{
    ArtifactRepository, BlobStore, EvidenceRepository, LateInteractionProvider,
    LateInteractionScorer, LateInteractionScoringResult, MultiVectorDocument, MultiVectorIdentity,
    MultiVectorQuery,
};

use crate::cancellation::SearchCallControl;
use crate::types::{
    CandidateSourceFilter, RankedCandidate, RerankLimits, RerankResult, RetrievalError,
};

const MAX_DOCUMENT_BYTES: usize = 65_536;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 8_192;

struct ScoredLateCandidate {
    position: usize,
    score: LateInteractionScoringResult,
    document: MultiVectorDocument,
}

struct ScoreCandidatesRequest<'a> {
    query: &'a MultiVectorQuery,
    candidates: &'a [RankedCandidate],
    eligible_positions: &'a [usize],
    score_limit: usize,
    authorization: &'a RetrievalAuthorizationContext,
    source_filter: Option<&'a CandidateSourceFilter>,
    control: &'a SearchCallControl,
}
/// Dependencies for an explicitly configured, prepared late generation.
pub struct LateInteractionRerankerParts {
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
    pub provider: Arc<dyn LateInteractionProvider>,
    pub scorer: Arc<dyn LateInteractionScorer>,
    pub identity: MultiVectorIdentity,
}

/// Bounded text/code late-interaction Stage A reranker.
///
pub struct LateInteractionReranker {
    parts: LateInteractionRerankerParts,
    limits: RerankLimits,
    model: String,
    fingerprint: RetrievalModelFingerprint,
    /// `None` means every non-protected Stage A intent; `Some` is the
    /// validated per-class opt-in set used by active serving.
    allowed_intents: Arc<RwLock<Option<BTreeSet<SearchIntent>>>>,
}

impl LateInteractionReranker {
    pub fn new(
        parts: LateInteractionRerankerParts,
        limits: RerankLimits,
    ) -> Result<Self, RetrievalError> {
        parts
            .identity
            .validate()
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        if parts.provider.identity().as_ref() != Some(&parts.identity) {
            return Err(RetrievalError::Internal(
                "late provider identity does not match prepared generation".into(),
            ));
        }
        let fingerprint = parts
            .identity
            .retrieval_fingerprint()
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        let model = format!(
            "late-interaction-reranker:{}",
            parts.identity.fingerprint.base.model
        );
        Ok(Self {
            parts,
            limits,
            model,
            fingerprint,
            allowed_intents: Arc::new(RwLock::new(None)),
        })
    }

    pub fn identity(&self) -> &MultiVectorIdentity {
        &self.parts.identity
    }
    /// Restrict active serving to the classes authorized by a validated record.
    pub fn set_allowed_intents(
        &self,
        intents: Option<BTreeSet<SearchIntent>>,
    ) -> Result<(), RetrievalError> {
        let mut guard = self.allowed_intents.write().map_err(|_| {
            RetrievalError::Internal("late interaction activation lock poisoned".into())
        })?;
        *guard = intents;
        Ok(())
    }

    fn intent_is_allowed(&self, intent: SearchIntent) -> bool {
        match self.allowed_intents.read() {
            Ok(guard) => guard
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&intent)),
            Err(_) => false,
        }
    }

    fn all_trace(
        &self,
        candidates: &[RankedCandidate],
        position: RerankPosition,
    ) -> Vec<SearchTraceRerankCandidate> {
        candidates
            .iter()
            .map(|candidate| SearchTraceRerankCandidate {
                candidate_id: candidate.candidate.evidence_id(),
                original_rank: candidate.rank,
                position: position.clone(),
                relevance_score: None,
                constraint_scores: Vec::new(),
                late_interaction: None,
            })
            .collect()
    }

    fn result(
        &self,
        candidates: Vec<RankedCandidate>,
        trace: Vec<SearchTraceRerankCandidate>,
    ) -> RerankResult {
        RerankResult {
            candidates,
            trace: SearchTraceRerank {
                model: self.model.clone(),
                fingerprint: self.fingerprint.clone(),
                input_cap: self.limits.input_cap,
                score_cap: self.limits.score_cap,
                output_cap: self.limits.output_cap,
                candidates: trace,
            },
        }
    }

    fn fallback(&self, candidates: Vec<RankedCandidate>, code: &'static str) -> RerankResult {
        self.result(
            candidates.clone(),
            self.all_trace(&candidates, RerankPosition::ErrorFallback(code.to_string())),
        )
    }

    fn applicable_intent(intent: SearchIntent) -> bool {
        matches!(
            intent,
            SearchIntent::FactualLocal
                | SearchIntent::SemanticDiscovery
                | SearchIntent::CompositionalConstraints
                | SearchIntent::RepositoryCode
        )
    }

    fn protected_candidate(candidate: &EvidenceCandidate) -> bool {
        candidate.reasons().iter().any(|reason| {
            matches!(
                reason,
                RetrievalReason::ExactMatch | RetrievalReason::SpecializedRetrieval { .. }
            )
        }) || matches!(
            candidate.source_span().location(),
            SourceLocation::Symbol { .. }
        )
    }
}

#[path = "late_interaction_reranker_execution.rs"]
mod execution;
#[path = "late_interaction_reranker_source.rs"]
mod source;
#[cfg(test)]
#[path = "late_interaction_reranker_tests.rs"]
mod tests;
