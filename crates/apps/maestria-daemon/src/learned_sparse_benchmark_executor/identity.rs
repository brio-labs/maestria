//! Benchmark identity construction for dense-only evaluations.
//!
//! The benchmark schema names the sparse representation and requires an
//! identity on every observation. When no sparse lane is configured, the
//! identity reuses that representation while carrying the embedding model's
//! fingerprint; `vocabulary_size` and `max_terms` carry the dense dimension.

use maestria_domain::RepresentationName;
use maestria_ports::{SPARSE_REPRESENTATION_V1, SparseFingerprint};
use maestria_retrieval::{LearnedSparseBenchmarkError, LearnedSparseBenchmarkIdentity};

use super::{BACKEND_FINGERPRINT, LearnedSparseBenchmarkExecutor};

impl LearnedSparseBenchmarkExecutor {
    /// The observation identity for a dense-only evaluation (no sparse lane
    /// configured), built from the live embedding provider identity.
    pub(super) fn dense_benchmark_identity(
        &self,
    ) -> Result<LearnedSparseBenchmarkIdentity, LearnedSparseBenchmarkError> {
        let provider = self.runtime.embedding_provider.clone().ok_or_else(|| {
            LearnedSparseBenchmarkError::InvalidIdentity(
                "dense embedding provider is unavailable for the observation".to_string(),
            )
        })?;
        let identity = provider.identity().ok_or_else(|| {
            LearnedSparseBenchmarkError::InvalidIdentity(
                "dense embedding provider has no identity".to_string(),
            )
        })?;
        let fingerprint = &identity.fingerprint;
        let dimension = fingerprint.dimensions.max(1);
        let namespace = self.corpus.namespace.clone().ok_or_else(|| {
            LearnedSparseBenchmarkError::InvalidIdentity("corpus namespace is missing".to_string())
        })?;
        let dense = LearnedSparseBenchmarkIdentity {
            corpus_snapshot: self.runtime.corpus_snapshot,
            index_generation: identity.generation_id,
            representation: RepresentationName(SPARSE_REPRESENTATION_V1.to_string()),
            namespace,
            fingerprint: SparseFingerprint {
                provider: fingerprint.provider.0.clone(),
                model: fingerprint.model.0.clone(),
                revision: fingerprint.revision.0.clone(),
                artifact_hash: fingerprint.artifact_hash.clone(),
                tokenizer_hash: fingerprint.query_template_hash.clone(),
                vocabulary_hash: fingerprint.document_template_hash.clone(),
                vocabulary_size: dimension,
                term_namespace: "dense_text_v1".to_string(),
                query_template_hash: fingerprint.query_template_hash.clone(),
                document_template_hash: fingerprint.document_template_hash.clone(),
                preprocessing_version: fingerprint.preprocessing_version.0.clone(),
                weighting_version: "cosine".to_string(),
                quantization: fingerprint.quantization.0.clone(),
                pruning_threshold: 0.0,
                max_terms: dimension,
            },
            backend_fingerprint: BACKEND_FINGERPRINT.to_string(),
        };
        dense
            .validate()
            .map_err(|error| LearnedSparseBenchmarkError::InvalidIdentity(error.to_string()))?;
        Ok(dense)
    }
}
