use maestria_domain::{IndexGenerationRegistry, IndexLifecycle};
use maestria_ports::SparseIdentity;

use crate::types::RetrievalError;

#[derive(Debug, Clone, PartialEq)]
pub struct LearnedSparseGenerationCapability {
    identity: SparseIdentity,
}

impl LearnedSparseGenerationCapability {
    pub fn activate(
        registry: &IndexGenerationRegistry,
        identity: SparseIdentity,
    ) -> Result<Self, RetrievalError> {
        identity
            .validate()
            .map_err(|error| RetrievalError::Internal(error.to_string()))?;
        let generation = registry.get(identity.generation_id).ok_or_else(|| {
            RetrievalError::Internal("sparse index generation is not registered".to_string())
        })?;
        if generation.lifecycle != IndexLifecycle::Active
            || !registry.is_serveable(identity.generation_id)
        {
            return Err(RetrievalError::Internal(
                "sparse index generation is not the active serveable generation".to_string(),
            ));
        }
        if generation.name != identity.representation
            || generation.corpus_snapshot != identity.corpus_snapshot
            || !fingerprints_match(&generation.fingerprint, &identity)
        {
            return Err(RetrievalError::Internal(
                "sparse index generation identity is incompatible".to_string(),
            ));
        }
        Ok(Self { identity })
    }

    pub fn identity(&self) -> &SparseIdentity {
        &self.identity
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
