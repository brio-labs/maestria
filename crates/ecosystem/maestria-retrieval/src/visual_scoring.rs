use std::time::Duration;

use maestria_ports::{EmbeddingResponse, VisualEmbeddingRequest};

use super::VisualReranker;
use crate::types::{RankedCandidate, RetrievalError};

/// Maximum number of dimensions accepted for a visual embedding vector.
pub(super) const MAX_VISUAL_VECTOR_DIMENSIONS: usize = 4_096;

impl VisualReranker {
    /// Embeds the query with the visual provider under a latency deadline.
    pub(super) async fn query_vector(
        &self,
        query: &str,
        remaining: Duration,
    ) -> Result<EmbeddingResponse, String> {
        let disclosure = self.parts.provider.disclosure();
        let response = tokio::time::timeout(remaining, async {
            self.parts
                .provider
                .embed_query(query, self.identity().clone())
        })
        .await
        .map_err(|_| RetrievalError::Timeout.to_string())?
        .map_err(|error| RetrievalError::Internal(error.to_string()).to_string())?;
        if response.identity != *self.identity()
            || response.disclosure != disclosure
            || response.vector.len() > MAX_VISUAL_VECTOR_DIMENSIONS
        {
            return Err("visual query response failed identity/privacy checks".to_string());
        }
        Ok(response)
    }

    /// Scores one candidate against the query vector within the latency budget.
    pub(super) async fn score_candidate(
        &self,
        candidate: &RankedCandidate,
        query_vector: &[f32],
        started: tokio::time::Instant,
        deadline: Duration,
    ) -> Result<u32, String> {
        let evidence = self
            .parts
            .evidence
            .get(candidate.candidate.evidence_id())
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
            || response.disclosure != self.parts.provider.disclosure()
            || response.vector.len() > MAX_VISUAL_VECTOR_DIMENSIONS
        {
            return Err("visual source response failed identity/privacy checks".to_string());
        }
        let similarity = Self::cosine(query_vector, &response.vector)
            .ok_or_else(|| "visual reranker returned incompatible vectors".to_string())?;
        Ok((((similarity + 1.0) * 0.5) * 1_000_000.0).round() as u32)
    }

    /// Cosine similarity between two equal-length finite vectors.
    pub(super) fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
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
}
