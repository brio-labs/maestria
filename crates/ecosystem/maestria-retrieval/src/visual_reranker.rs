use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, RerankPosition, RetrievalModelFingerprint, SearchTraceRerank,
    SearchTraceRerankCandidate, SourceLocation,
};
use maestria_governance::{RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, EmbeddingIdentity, EvidenceRepository, RetentionPolicy,
    VisualEmbeddingProvider,
};

use crate::adapters::VisualGenerationCapability;
use crate::traits::CandidateReranker;
use crate::types::{RerankLimits, RerankRequest, RerankResult, RetrievalError};
#[path = "visual_reranker_order.rs"]
mod visual_reranker_order;
use visual_reranker_order::reorder_visual_candidates;
#[path = "visual_scoring.rs"]
mod visual_scoring;
#[path = "visual_source.rs"]
mod visual_source;

/// Dependencies for the optional visual evidence reranker.
pub struct VisualRerankerParts {
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
    pub provider: Arc<dyn VisualEmbeddingProvider + Send + Sync>,
    pub capability: VisualGenerationCapability,
    pub policy: RetrievalSecurityPolicy,
}

/// Bounded multimodal reranking over PDF page and region candidates.
///
/// Text/layout candidates are never discarded. Only visual candidates occupy
/// visual reranking slots; provider failures return the original ranking with
/// an explicit fallback trace.
pub struct VisualReranker {
    parts: VisualRerankerParts,
    limits: RerankLimits,
    model: String,
    fingerprint: RetrievalModelFingerprint,
}

impl VisualReranker {
    /// Creates a reranker bound to an already validated visual generation.
    pub fn new(parts: VisualRerankerParts, limits: RerankLimits) -> Result<Self, RetrievalError> {
        let identity = parts.capability.identity();
        let model = format!("visual-reranker:{}", identity.fingerprint.model);
        let fingerprint = RetrievalModelFingerprint::new(format!(
            "visual-reranker:{}:{}:{}",
            identity.fingerprint.provider,
            identity.fingerprint.model,
            identity.fingerprint.revision
        ))
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        Ok(Self {
            parts,
            limits,
            model,
            fingerprint,
        })
    }

    fn identity(&self) -> &EmbeddingIdentity {
        self.parts.capability.identity()
    }

    fn trace_for_all(
        &self,
        candidates: &[crate::types::RankedCandidate],
        position: RerankPosition,
    ) -> Vec<SearchTraceRerankCandidate> {
        candidates
            .iter()
            .map(|candidate| SearchTraceRerankCandidate {
                candidate_id: candidate.candidate.evidence_id,
                original_rank: candidate.rank,
                position: position.clone(),
                relevance_score: None,
                constraint_scores: Vec::new(),
            })
            .collect()
    }

    fn result_with_trace(
        &self,
        candidates: Vec<crate::types::RankedCandidate>,
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

    fn fallback(
        &self,
        candidates: Vec<crate::types::RankedCandidate>,
        reason: impl Into<String>,
    ) -> RerankResult {
        self.result_with_trace(
            candidates.clone(),
            self.trace_for_all(&candidates, RerankPosition::ErrorFallback(reason.into())),
        )
    }

    fn preflight(&self, query: &str) -> Result<(), String> {
        if !scan_secrets(query).is_clean() {
            return Err("visual reranker query rejected by secret scanner".to_string());
        }
        if self.parts.provider.identity().as_ref() != Some(self.identity()) {
            return Err("visual reranker provider identity changed".to_string());
        }
        let disclosure = self.parts.provider.disclosure();
        if disclosure.remote || disclosure.retention != RetentionPolicy::NoRetention {
            return Err("visual reranker provider is not local and no-retention".to_string());
        }
        Ok(())
    }

    fn visual_candidate(candidate: &EvidenceCandidate) -> bool {
        matches!(
            candidate.source_span.location(),
            SourceLocation::Page { .. } | SourceLocation::Region { .. }
        )
    }
}
#[async_trait]
impl CandidateReranker for VisualReranker {
    async fn rerank(&self, request: RerankRequest) -> Result<RerankResult, RetrievalError> {
        let RerankRequest {
            plan,
            candidates,
            max_latency_ms,
        } = request;
        if plan.intent() != maestria_domain::SearchIntent::VisualDocument {
            return Ok(self.result_with_trace(
                candidates.clone(),
                self.trace_for_all(&candidates, RerankPosition::SkippedNotApplicable),
            ));
        }
        if let Err(reason) = self.preflight(plan.original_query()) {
            return Ok(self.fallback(candidates, reason));
        }
        let visual_positions = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                Self::visual_candidate(&candidate.candidate).then_some(index)
            })
            .collect::<Vec<_>>();
        if visual_positions.is_empty()
            || max_latency_ms == 0
            || self.limits.input_cap == 0
            || self.limits.score_cap == 0
        {
            return Ok(self.result_with_trace(
                candidates.clone(),
                self.trace_for_all(&candidates, RerankPosition::SkippedNotApplicable),
            ));
        }

        let started = tokio::time::Instant::now();
        let deadline = Duration::from_millis(u64::from(max_latency_ms));
        let query_response = match self.query_vector(plan.original_query(), deadline).await {
            Ok(response) => response,
            Err(reason) => return Ok(self.fallback(candidates, reason)),
        };
        let trace = self.trace_for_all(&candidates, RerankPosition::SkippedNotApplicable);
        let score_limit = self.limits.input_cap.min(self.limits.score_cap);
        let mut scored = Vec::with_capacity(score_limit.min(visual_positions.len()));
        for position in visual_positions.iter().copied().take(score_limit) {
            let score = match self
                .score_candidate(
                    &candidates[position],
                    &query_response.vector,
                    started,
                    deadline,
                )
                .await
            {
                Ok(score) => score,
                Err(reason) => return Ok(self.fallback(candidates, reason)),
            };
            scored.push((position, score));
        }
        Ok(reorder_visual_candidates(
            candidates,
            &visual_positions,
            scored,
            trace,
            self.limits.clone(),
            self.model.clone(),
            self.fingerprint.clone(),
        ))
    }
}

#[cfg(test)]
#[path = "visual_reranker_tests.rs"]
mod tests;
