use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use maestria_domain::{
    Evidence, EvidenceCandidate, EvidenceKind, RerankCandidateStatus, RetrievalModelFingerprint,
    SearchTraceRerank, SearchTraceRerankCandidate, SourceLocation,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, EmbeddingIdentity, EmbeddingResponse, EvidenceRepository,
    RetentionPolicy, VisualEmbeddingProvider, VisualEmbeddingRequest, VisualSource,
};

use crate::adapters::VisualGenerationCapability;
use crate::traits::CandidateReranker;
use crate::types::{RerankLimits, RerankRequest, RerankResult, RetrievalError};
#[path = "visual_reranker_order.rs"]
mod visual_reranker_order;
use visual_reranker_order::reorder_visual_candidates;

const MAX_VISUAL_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_VISUAL_VECTOR_DIMENSIONS: usize = 4_096;

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
        status: RerankCandidateStatus,
    ) -> Vec<SearchTraceRerankCandidate> {
        candidates
            .iter()
            .map(|candidate| SearchTraceRerankCandidate {
                candidate_id: candidate.candidate.evidence_id,
                original_rank: candidate.rank,
                new_rank: Some(candidate.rank),
                status: status.clone(),
                relevance_score: None,
                constraint_score: None,
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
            self.trace_for_all(
                &candidates,
                RerankCandidateStatus::ErrorFallback(reason.into()),
            ),
        )
    }

    fn preflight(&self, query: &str) -> Result<(), String> {
        if !scan_secrets(query).is_clean() {
            return Err("visual reranker query rejected by secret scanner".to_string());
        }
        if self.parts.provider.identity().as_ref() != Some(self.identity()) {
            return Err("visual reranker provider identity changed".to_string());
        }
        let Some(disclosure) = self.parts.provider.disclosure() else {
            return Err("visual reranker provider disclosure unavailable".to_string());
        };
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

    fn visual_source(evidence: &Evidence) -> Option<VisualSource> {
        match evidence.kind {
            EvidenceKind::PdfSpan {
                blob,
                page_start,
                page_end,
            } => Some(VisualSource::Page {
                blob,
                page_start,
                page_end,
            }),
            EvidenceKind::PdfRegion {
                blob,
                page,
                x,
                y,
                width,
                height,
            } => Some(VisualSource::Region {
                blob,
                page,
                x,
                y,
                width,
                height,
            }),
            _ => None,
        }
    }

    fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
        if left.is_empty() || left.len() != right.len() {
            return None;
        }
        let mut dot = 0.0_f32;
        let mut left_norm = 0.0_f32;
        let mut right_norm = 0.0_f32;
        for (left_value, right_value) in left.iter().zip(right) {
            if !left_value.is_finite() || !right_value.is_finite() {
                return None;
            }
            dot += left_value * right_value;
            left_norm += left_value * left_value;
            right_norm += right_value * right_value;
        }
        let denominator = left_norm.sqrt() * right_norm.sqrt();
        (denominator > 0.0 && denominator.is_finite())
            .then_some((dot / denominator).clamp(-1.0, 1.0))
    }

    fn source_bytes(&self, evidence: &Evidence) -> Result<(VisualSource, Vec<u8>), RetrievalError> {
        let Some(source) = Self::visual_source(evidence) else {
            return Err(RetrievalError::Internal(
                "visual reranker candidate has no PDF source".to_string(),
            ));
        };
        let Some(artifact) = self
            .parts
            .artifacts
            .get(evidence.artifact_id)
            .map_err(|error| RetrievalError::Internal(error.to_string()))?
        else {
            return Err(RetrievalError::Internal(
                "visual reranker artifact is missing".to_string(),
            ));
        };
        if artifact.index_status != maestria_domain::IndexStatus::Indexed
            || self.parts.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
            || self.parts.policy.evaluate(&evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&evidence.excerpt).is_clean()
        {
            return Err(RetrievalError::Internal(
                "visual reranker candidate failed security checks".to_string(),
            ));
        }
        let blob = match &source {
            VisualSource::Page { blob, .. } | VisualSource::Region { blob, .. } => *blob,
        };
        let bytes = self
            .parts
            .blobs
            .get(blob)
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        if bytes.is_empty()
            || bytes.len() > MAX_VISUAL_SOURCE_BYTES
            || !scan_secrets(&String::from_utf8_lossy(&bytes)).is_clean()
        {
            return Err(RetrievalError::Internal(
                "visual reranker source failed privacy checks".to_string(),
            ));
        }
        Ok((source, bytes))
    }
}
impl VisualReranker {
    async fn query_vector(
        &self,
        query: &str,
        remaining: Duration,
    ) -> Result<EmbeddingResponse, String> {
        let response = tokio::time::timeout(remaining, async {
            self.parts
                .provider
                .embed_query(query, self.identity().clone())
        })
        .await
        .map_err(|_| RetrievalError::Timeout.to_string())?
        .map_err(|error| RetrievalError::Internal(error.to_string()).to_string())?;
        if response.identity != *self.identity()
            || response.disclosure.remote
            || response.disclosure.retention != RetentionPolicy::NoRetention
            || response.vector.len() > MAX_VISUAL_VECTOR_DIMENSIONS
        {
            return Err("visual query response failed identity/privacy checks".to_string());
        }
        Ok(response)
    }

    async fn score_candidate(
        &self,
        candidate: &crate::types::RankedCandidate,
        query_vector: &[f32],
        started: tokio::time::Instant,
        deadline: Duration,
    ) -> Result<u32, String> {
        let evidence = self
            .parts
            .evidence
            .get(candidate.candidate.evidence_id)
            .map_err(|error| RetrievalError::Internal(error.to_string()).to_string())?
            .ok_or_else(|| "visual reranker evidence is missing".to_string())?;
        let (source, bytes) = self
            .source_bytes(&evidence)
            .map_err(|error| error.to_string())?;
        let remaining = deadline
            .checked_sub(started.elapsed())
            .ok_or_else(|| "visual reranker latency budget exhausted".to_string())?;
        let response = tokio::time::timeout(remaining, async {
            self.parts.provider.embed_source(VisualEmbeddingRequest {
                source,
                bytes,
                identity: self.identity().clone(),
            })
        })
        .await
        .map_err(|_| RetrievalError::Timeout.to_string())?
        .map_err(|error| RetrievalError::Internal(error.to_string()).to_string())?;
        if response.identity != *self.identity()
            || response.disclosure.remote
            || response.disclosure.retention != RetentionPolicy::NoRetention
            || response.vector.len() > MAX_VISUAL_VECTOR_DIMENSIONS
        {
            return Err("visual source response failed identity/privacy checks".to_string());
        }
        let similarity = Self::cosine(query_vector, &response.vector)
            .ok_or_else(|| "visual reranker returned incompatible vectors".to_string())?;
        Ok((((similarity + 1.0) * 0.5) * 1_000_000.0).round() as u32)
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
        if plan.intent != maestria_domain::SearchIntent::VisualDocument {
            return Ok(self.result_with_trace(
                candidates.clone(),
                self.trace_for_all(&candidates, RerankCandidateStatus::SkippedNotApplicable),
            ));
        }
        if let Err(reason) = self.preflight(&plan.original_query) {
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
                self.trace_for_all(&candidates, RerankCandidateStatus::SkippedNotApplicable),
            ));
        }

        let started = tokio::time::Instant::now();
        let deadline = Duration::from_millis(u64::from(max_latency_ms));
        let query_response = match self.query_vector(&plan.original_query, deadline).await {
            Ok(response) => response,
            Err(reason) => return Ok(self.fallback(candidates, reason)),
        };
        let trace = self.trace_for_all(&candidates, RerankCandidateStatus::SkippedNotApplicable);
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
