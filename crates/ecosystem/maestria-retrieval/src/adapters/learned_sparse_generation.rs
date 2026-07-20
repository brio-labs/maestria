use maestria_domain::{IndexGenerationRegistry, IndexLifecycle};
use maestria_ports::SparseIdentity;

use crate::types::RetrievalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnedSparseGenerationMode {
    Shadow,
    Active,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LearnedSparseGenerationCapability {
    identity: SparseIdentity,
    mode: LearnedSparseGenerationMode,
}

impl LearnedSparseGenerationCapability {
    pub fn shadow(
        registry: &IndexGenerationRegistry,
        identity: SparseIdentity,
    ) -> Result<Self, RetrievalError> {
        Self::validate(registry, identity, LearnedSparseGenerationMode::Shadow)
    }

    pub fn activate(
        registry: &IndexGenerationRegistry,
        identity: SparseIdentity,
    ) -> Result<Self, RetrievalError> {
        Self::validate(registry, identity, LearnedSparseGenerationMode::Active)
    }

    pub fn identity(&self) -> &SparseIdentity {
        &self.identity
    }

    pub fn mode(&self) -> LearnedSparseGenerationMode {
        self.mode
    }

    pub fn is_serving_eligible(&self) -> bool {
        self.mode == LearnedSparseGenerationMode::Active
    }

    fn validate(
        registry: &IndexGenerationRegistry,
        identity: SparseIdentity,
        mode: LearnedSparseGenerationMode,
    ) -> Result<Self, RetrievalError> {
        identity
            .validate()
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        let generation = registry.get(identity.generation_id).ok_or_else(|| {
            RetrievalError::Internal("sparse index generation is not registered".to_string())
        })?;
        let lifecycle_matches = match mode {
            LearnedSparseGenerationMode::Shadow => generation.lifecycle == IndexLifecycle::Shadow,
            LearnedSparseGenerationMode::Active => {
                generation.lifecycle == IndexLifecycle::Active
                    && registry.is_serveable(identity.generation_id)
            }
        };
        if !lifecycle_matches {
            return Err(RetrievalError::Internal(match mode {
                LearnedSparseGenerationMode::Shadow => {
                    "sparse shadow capability requires a shadow generation".to_string()
                }
                LearnedSparseGenerationMode::Active => {
                    "sparse active capability requires the active serveable generation".to_string()
                }
            }));
        }
        if generation.name != identity.representation
            || generation.corpus_snapshot != identity.corpus_snapshot
            || !fingerprints_match(&generation.fingerprint, &identity)
        {
            return Err(RetrievalError::Internal(
                "sparse index generation identity is incompatible".to_string(),
            ));
        }
        Ok(Self { identity, mode })
    }
}

fn fingerprints_match(
    generation: &maestria_domain::IndexFingerprint,
    identity: &SparseIdentity,
) -> bool {
    let sparse = &identity.fingerprint;
    generation.provider == sparse.provider
        && generation.model == sparse.model
        && generation.revision == sparse.revision
        && generation.artifact_hash == sparse.artifact_hash
        && generation.dimensions == sparse.vocabulary_size
        && generation.quantization == sparse.quantization
        && generation.query_template_hash == sparse.query_template_hash.as_str()
        && generation.document_template_hash == sparse.document_template_hash.as_str()
        && generation.preprocessing_version == sparse.preprocessing_version
}
