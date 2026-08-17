//! Durable-record execution policies and the learned-sparse lane builder.
//!
//! Kept out of the runtime assembly module: the policy functions are pure
//! store/manifest reads with fail-closed degradation, and the sparse
//! retriever construction owns its provider/index lifecycle.

use std::sync::Arc;

use crate::providers::build_sparse_provider;
use crate::sparse_startup::sparse_identity;
use maestria_blob_fs::FsBlobStore;
use maestria_core::InstanceManifest;
use maestria_domain::KernelState;
use maestria_ports::{
    LearnedSparseIndex, LearnedSparseProvider, SPARSE_REPRESENTATION_V1, SparseIdentity,
};
use maestria_retrieval::CandidateRetriever;
use maestria_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use maestria_storage_sqlite::SqliteLearnedSparseIndex;
use maestria_storage_sqlite::SqliteStore;

/// Loads the durable promotion record and derives the execution policy.
///
/// Fail-closed: an unparsable or invalid record degrades to shadow serving;
/// a manifest without the sparse profile disables the lane entirely.
pub(crate) fn learned_sparse_policy(
    store: &SqliteStore,
    manifest: &InstanceManifest,
) -> maestria_retrieval::LearnedSparseExecutionPolicy {
    use maestria_retrieval::LearnedSparseExecutionPolicy;
    if manifest
        .sparse
        .as_ref()
        .is_none_or(|config| !config.enabled)
    {
        return LearnedSparseExecutionPolicy::Disabled;
    }
    match store.load_latest_promotion_record() {
        Ok(Some(record)) => match serde_json::from_str::<
            maestria_retrieval::LearnedSparsePromotionRecord,
        >(&record.record_json)
        {
            Ok(record) if record.validate().is_ok() => {
                LearnedSparseExecutionPolicy::Active(Box::new(record))
            }
            Ok(_) => {
                tracing::warn!("sparse promotion record is invalid; serving shadow");
                LearnedSparseExecutionPolicy::Shadow
            }
            Err(error) => {
                tracing::warn!("sparse promotion record is unparsable; serving shadow: {error}");
                LearnedSparseExecutionPolicy::Shadow
            }
        },
        Ok(None) => LearnedSparseExecutionPolicy::Shadow,
        Err(error) => {
            tracing::warn!("sparse promotion record is unreadable; serving shadow: {error}");
            LearnedSparseExecutionPolicy::Shadow
        }
    }
}

/// The dense lane's execution policy: a valid hybrid promotion record
/// activates the lexical+dense fusion; otherwise the dense lane stays
/// shadowed. Fail-closed on unparsable records.
pub(crate) fn hybrid_policy(store: &SqliteStore) -> maestria_retrieval::HybridExecutionPolicy {
    use maestria_retrieval::HybridExecutionPolicy;
    match store.load_latest_hybrid_promotion_record() {
        Ok(Some(record)) => {
            match serde_json::from_str::<maestria_retrieval::HybridPromotionRecord>(
                &record.record_json,
            ) {
                Ok(record) => HybridExecutionPolicy::Active(record),
                Err(error) => {
                    tracing::warn!(
                        "hybrid promotion record is unparsable; serving shadow: {error}"
                    );
                    HybridExecutionPolicy::Shadow
                }
            }
        }
        Ok(None) => HybridExecutionPolicy::Shadow,
        Err(error) => {
            tracing::warn!("hybrid promotion record is unreadable; serving shadow: {error}");
            HybridExecutionPolicy::Shadow
        }
    }
}

/// Builds the registered learned-sparse retriever for the active generation.
///
/// Degrades to no lane (hybrid serving) when the generation is not active or
/// the provider is unavailable; the policy still gates the lane's eligibility.
pub(crate) fn build_sparse_retriever(
    state: &KernelState,
    manifest: &InstanceManifest,
    store: Arc<SqliteStore>,
    blobs: Arc<FsBlobStore>,
) -> Option<Arc<dyn CandidateRetriever>> {
    let generation_id = resolve_sparse_generation(state, manifest)?;
    let identity = resolve_sparse_identity(state, manifest, generation_id)?;
    let provider = resolve_sparse_provider(manifest, identity.clone())?;
    let index = resolve_sparse_index(&store, identity.clone())?;
    let capability = resolve_sparse_capability(state, identity.clone())?;
    match LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index: Arc::new(index) as Arc<dyn LearnedSparseIndex + Send + Sync>,
            artifacts: store.clone(),
            chunks: store.clone(),
            evidence: store.clone(),
            blobs,
            provider,
        },
        capability,
    ) {
        Ok(retriever) => Some(Arc::new(retriever)),
        Err(error) => {
            tracing::error!("sparse retriever construction failed; serving hybrid: {error}");
            None
        }
    }
}

/// The active sparse generation for the manifest's sparse profile, or `None`
/// when the profile is disabled or no generation is active (hybrid serving).
fn resolve_sparse_generation(
    state: &KernelState,
    manifest: &InstanceManifest,
) -> Option<maestria_domain::IndexGenerationId> {
    if manifest
        .sparse
        .as_ref()
        .is_none_or(|config| !config.enabled)
    {
        return None;
    }
    match state
        .index_generations
        .get_active(&maestria_domain::RepresentationName::new(
            SPARSE_REPRESENTATION_V1,
        )) {
        Some(generation) => Some(generation.id),
        None => {
            tracing::warn!(
                "sparse profile enabled but no active sparse generation; serving hybrid"
            );
            None
        }
    }
}

/// The sparse identity for the active generation, or `None` on failure.
fn resolve_sparse_identity(
    state: &KernelState,
    manifest: &InstanceManifest,
    generation_id: maestria_domain::IndexGenerationId,
) -> Option<SparseIdentity> {
    match sparse_identity(state, manifest, generation_id) {
        Ok(identity) => Some(identity),
        Err(error) => {
            tracing::warn!("sparse identity is unavailable; serving hybrid: {error}");
            None
        }
    }
}

/// The configured sparse provider, or `None` when unconfigured/unavailable.
fn resolve_sparse_provider(
    manifest: &InstanceManifest,
    identity: SparseIdentity,
) -> Option<Arc<dyn LearnedSparseProvider + Send + Sync>> {
    match build_sparse_provider(manifest, identity) {
        Ok(Some(provider)) => Some(provider),
        Ok(None) => {
            tracing::warn!("sparse provider is not configured; serving hybrid");
            None
        }
        Err(error) => {
            tracing::warn!("sparse provider is unavailable; serving hybrid: {error}");
            None
        }
    }
}

/// The durable sparse projection index, or `None` when it cannot be opened.
fn resolve_sparse_index(
    store: &Arc<SqliteStore>,
    identity: SparseIdentity,
) -> Option<SqliteLearnedSparseIndex> {
    match SqliteLearnedSparseIndex::new(store.clone(), identity) {
        Ok(index) => Some(index),
        Err(error) => {
            tracing::warn!("sparse projection is unavailable; serving hybrid: {error}");
            None
        }
    }
}

/// The active generation capability for the sparse lane, or `None`.
fn resolve_sparse_capability(
    state: &KernelState,
    identity: SparseIdentity,
) -> Option<LearnedSparseGenerationCapability> {
    match LearnedSparseGenerationCapability::activate(&state.index_generations, identity) {
        Ok(capability) => Some(capability),
        Err(error) => {
            tracing::warn!("sparse generation is not active; serving hybrid: {error}");
            None
        }
    }
}
