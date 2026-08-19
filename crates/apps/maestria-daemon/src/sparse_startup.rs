use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    ContentHash, IndexFingerprint, IndexGenerationId, IndexLifecycle, KernelState,
    RepresentationName, SparseNamespace, TrustZone,
};
use maestria_ports::{
    LearnedSparseIndex, LearnedSparseProjectionLifecycle, LearnedSparseProvider,
    SPARSE_REPRESENTATION_V1, SparseDocument, SparseFingerprint, SparseIdentity, SparseInputKind,
};
use maestria_storage_sqlite::{SqliteLearnedSparseIndex, SqliteStore};

use crate::providers::build_sparse_provider;
use crate::vector_startup::ensure_generation;

/// Templates applied by the pinned sidecar; the fingerprint binds them so a
/// checkpoint serving different templates cannot masquerade as this profile.
const QUERY_TEMPLATE: &str = "query: {text}";
const DOCUMENT_TEMPLATE: &str = "document: {text}";
const TERM_NAMESPACE: &str = "splade-vocabulary-v1";
const WEIGHTING_VERSION: &str = "splade-log1p-relu-v1";
/// The sidecar contract caps encoded text at 8192 characters; document
/// chunks are truncated to that bound before encoding. The chunk content
/// hash still covers the full source text.
pub const MAX_ENCODE_TEXT_CHARS: usize = 8_192;

/// Bounds document text to the sidecar encoding contract.
pub fn truncate_document_text(text: &str) -> String {
    text.chars().take(MAX_ENCODE_TEXT_CHARS).collect()
}

/// The instance's learned-sparse namespace: one per realm, verified zone,
/// sparse projection. Deterministic from the manifest.
pub fn sparse_namespace(manifest: &InstanceManifest) -> Result<SparseNamespace> {
    SparseNamespace::new(
        manifest.realm_id.as_str(),
        TrustZone::Verified,
        SPARSE_REPRESENTATION_V1,
    )
    .map_err(|error| anyhow!("build sparse namespace: {error}"))
}

fn template_hash(template: &str) -> Result<ContentHash> {
    ContentHash::new(maestria_domain::content_hash(template.as_bytes()))
        .map_err(|error| anyhow!("invalid sparse template hash: {error}"))
}

/// Derives a pinned component hash from the manifest identity.
///
/// The manifest artifact hash covers every pinned artifact (model, tokenizer,
/// vocabulary); the tokenizer and vocabulary hashes are derived from it so
/// the fingerprint stays deterministic without duplicating artifact state.
fn derived_component_hash(label: &str, identity: &str) -> Result<ContentHash> {
    ContentHash::new(maestria_domain::content_hash(
        format!("{label}:{identity}").as_bytes(),
    ))
    .map_err(|error| anyhow!("derive sparse component hash: {error}"))
}

/// The SparseFingerprint the instance binds to its pinned sparse profile.
pub fn sparse_fingerprint(manifest: &InstanceManifest) -> Result<SparseFingerprint> {
    let config = manifest
        .sparse
        .as_ref()
        .filter(|config| config.enabled)
        .ok_or_else(|| anyhow!("sparse profile is not enabled"))?;
    let artifact_hash = ContentHash::new(config.artifact_hash.clone())
        .map_err(|error| anyhow!("invalid sparse artifact hash: {error}"))?;
    let identity = format!("{}:{}", config.model, config.artifact_hash);
    Ok(SparseFingerprint {
        provider: config.provider.clone(),
        model: config.model.clone(),
        revision: config.revision.clone(),
        artifact_hash,
        tokenizer_hash: derived_component_hash("splade-tokenizer", &identity)?,
        vocabulary_hash: derived_component_hash("splade-vocabulary", &identity)?,
        vocabulary_size: config.vocabulary_size,
        term_namespace: TERM_NAMESPACE.to_string(),
        query_template_hash: template_hash(QUERY_TEMPLATE)?,
        document_template_hash: template_hash(DOCUMENT_TEMPLATE)?,
        preprocessing_version: config.preprocessing_version.clone(),
        weighting_version: WEIGHTING_VERSION.to_string(),
        quantization: "f32".to_string(),
        pruning_threshold: 0.0,
        max_terms: config.term_cap,
    })
}

/// Ensures the fingerprinted `sparse_text_v1` generation exists and is active.
///
/// Mirrors the lexical/dense generation reconciliation; the sparse namespace
/// is bound at registration so capability checks can validate it.
pub fn reconcile_sparse_generation(
    layout: &InstanceLayout,
    state: &mut KernelState,
    manifest: &InstanceManifest,
) -> Result<IndexGenerationId> {
    if manifest
        .sparse
        .as_ref()
        .is_none_or(|config| !config.enabled)
    {
        return Err(anyhow!("sparse profile is not enabled"));
    }
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let fingerprint = sparse_fingerprint(manifest)?;
    let namespace = sparse_namespace(manifest)?;
    let index_fingerprint = IndexFingerprint {
        provider: maestria_domain::ProviderName::new(fingerprint.provider.clone()),
        model: maestria_domain::ModelName::new(fingerprint.model.clone()),
        revision: maestria_domain::FingerprintRevision::new(fingerprint.revision.clone()),
        artifact_hash: fingerprint.artifact_hash.clone(),
        dimensions: fingerprint.vocabulary_size,
        quantization: maestria_domain::QuantizationScheme::new("f32"),
        query_template_hash: fingerprint.query_template_hash.clone(),
        document_template_hash: fingerprint.document_template_hash.clone(),
        preprocessing_version: maestria_domain::PreprocessingVersion::new(
            fingerprint.preprocessing_version.clone(),
        ),
    };
    ensure_generation(
        state,
        &store,
        RepresentationName::new(SPARSE_REPRESENTATION_V1),
        index_fingerprint,
        maestria_domain::DEFAULT_CORPUS_SNAPSHOT_ID,
        Some(namespace),
    )
}

/// Builds the full sparse identity from the registry generation.
pub fn sparse_identity(
    state: &KernelState,
    manifest: &InstanceManifest,
    generation_id: IndexGenerationId,
) -> Result<SparseIdentity> {
    let generation = state
        .index_generations
        .get(generation_id)
        .ok_or_else(|| anyhow!("sparse generation {generation_id} is not registered"))?;
    let namespace = generation
        .sparse_namespace
        .clone()
        .ok_or_else(|| anyhow!("sparse generation is missing its namespace"))?;
    let fingerprint = sparse_fingerprint(manifest)?;
    let identity = SparseIdentity {
        generation_id,
        corpus_snapshot: generation.corpus_snapshot,
        representation: generation.name.clone(),
        namespace,
        fingerprint,
    };
    identity
        .validate()
        .map_err(|error| anyhow!("sparse identity is invalid: {error}"))?;
    Ok(identity)
}

/// Advances the durable projection lifecycle to match the registry.
fn activate_projection(index: &SqliteLearnedSparseIndex) -> Result<()> {
    let lifecycle = index
        .lifecycle()
        .map_err(|error| anyhow!("read sparse projection lifecycle: {error}"))?;
    let path = match lifecycle {
        IndexLifecycle::Building => vec![
            IndexLifecycle::Evaluated,
            IndexLifecycle::Shadow,
            IndexLifecycle::Active,
        ],
        IndexLifecycle::Evaluated => vec![IndexLifecycle::Shadow, IndexLifecycle::Active],
        IndexLifecycle::Shadow => vec![IndexLifecycle::Active],
        IndexLifecycle::Active => Vec::new(),
        IndexLifecycle::Retired | IndexLifecycle::Collectable | IndexLifecycle::Tombstoned => {
            return Err(anyhow!(
                "sparse projection lifecycle {lifecycle:?} cannot be reactivated"
            ));
        }
    };
    for next in path {
        let expected = index
            .lifecycle()
            .map_err(|error| anyhow!("read sparse projection lifecycle: {error}"))?;
        index
            .transition(expected, next)
            .map_err(|error| anyhow!("advance sparse projection lifecycle: {error}"))?;
    }
    Ok(())
}

/// Encodes and indexes every eligible chunk into the sparse projection.
///
/// Idempotent: rows are upserted in stable chunk order, exactly mirroring the
/// vector projection recovery so a partially built projection is completed
/// instead of duplicated.
pub fn reconcile_sparse_projection_for_layout(
    layout: &InstanceLayout,
    state: &mut KernelState,
    manifest: &InstanceManifest,
) -> Result<()> {
    let generation_id = reconcile_sparse_generation(layout, state, manifest)?;
    let identity = sparse_identity(state, manifest, generation_id)?;
    let provider = build_sparse_provider(manifest, identity.clone())?
        .ok_or_else(|| anyhow!("sparse provider is not configured"))?;
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let index = SqliteLearnedSparseIndex::new(Arc::new(store), identity.clone())
        .map_err(|error| anyhow!("open sparse projection: {error}"))?;
    activate_projection(&index)?;

    let eligible = crate::projection_recovery::retrieval_eligible_chunks(state).collect::<Vec<_>>();
    // An empty chunk set reconciles to an empty projection; do not call the
    // provider at all (the batch contract requires at least one text).
    let Some(first) = eligible.first() else {
        return index
            .index_documents(Vec::new())
            .map_err(|error| anyhow!("index sparse projection: {error}"));
    };
    let _ = first;
    let encoded_texts = eligible
        .iter()
        .map(|chunk| truncate_document_text(&chunk.text))
        .collect::<Vec<_>>();
    let vectors =
        provider.encode_batch(&encoded_texts, SparseInputKind::Document, identity.clone())?;
    let documents = eligible
        .into_iter()
        .zip(vectors)
        .map(|(chunk, vector)| {
            if vector.identity() != &identity {
                return Err(anyhow!(
                    "encode chunk {} returned an incompatible generation identity",
                    chunk.id
                ));
            }
            let content_hash = match state
                .artifacts
                .get(&chunk.artifact_id)
                .and_then(|artifact| artifact.content_hash.clone())
            {
                Some(content_hash) => content_hash,
                None => {
                    let computed = maestria_domain::content_hash(chunk.text.as_bytes());
                    ContentHash::new(computed).map_err(|error| {
                        anyhow!(
                            "computed content hash for chunk {} is invalid: {error}",
                            chunk.id
                        )
                    })?
                }
            };
            Ok(SparseDocument {
                chunk_id: chunk.id,
                content_hash,
                vector,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    index
        .index_documents(documents)
        .map_err(|error| anyhow!("index sparse projection: {error}"))
}

/// Builds the configured sparse provider for the active sparse generation.
pub fn build_sparse_provider_for_layout(
    manifest: &InstanceManifest,
    state: &KernelState,
) -> Result<Option<Arc<dyn LearnedSparseProvider + Send + Sync>>> {
    let Some(_) = manifest.sparse.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    let generation_id = state
        .index_generations
        .get_active(&RepresentationName::new(SPARSE_REPRESENTATION_V1))
        .map(|generation| generation.id)
        .ok_or_else(|| anyhow!("active sparse generation is missing"))?;
    let identity = sparse_identity(state, manifest, generation_id)?;
    build_sparse_provider(manifest, identity)
}
