use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{
    ContentHash, DomainInput, IndexFingerprint, IndexGenerationId, IndexLifecycle, KernelState,
    RepresentationName, StartIndexGenerationInput, TransitionIndexGenerationInput,
};
use maestria_ports::EventLog;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;
use maestria_vector_sqlite::SqliteVectorIndex;

use crate::reconcile_vector_projection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrievalGenerations {
    pub primary: IndexGenerationId,
    pub dense: Option<IndexGenerationId>,
}

fn persist_input(state: &mut KernelState, store: &SqliteStore, input: DomainInput) -> Result<()> {
    let output = state
        .apply_input(input)
        .map_err(|error| anyhow!("apply generation input: {error}"))?;
    for event in output.events {
        store
            .append(event)
            .map_err(|error| anyhow!("persist generation event: {error}"))?;
    }
    Ok(())
}

fn next_generation_id(state: &KernelState) -> IndexGenerationId {
    IndexGenerationId::new(
        state
            .index_generations
            .iter()
            .map(|generation| generation.id.value())
            .max()
            .map_or(0, |value| value)
            .saturating_add(1),
    )
}

fn advance_generation(
    state: &mut KernelState,
    store: &SqliteStore,
    id: IndexGenerationId,
) -> Result<()> {
    let lifecycle = state
        .index_generations
        .get(id)
        .ok_or_else(|| anyhow!("generation {id} disappeared during reconciliation"))?
        .lifecycle;
    let transitions: Vec<IndexLifecycle> = match lifecycle {
        IndexLifecycle::Building => vec![
            IndexLifecycle::Evaluated,
            IndexLifecycle::Shadow,
            IndexLifecycle::Active,
        ],
        IndexLifecycle::Evaluated => vec![IndexLifecycle::Shadow, IndexLifecycle::Active],
        IndexLifecycle::Shadow | IndexLifecycle::Retired => vec![IndexLifecycle::Active],
        IndexLifecycle::Active | IndexLifecycle::Collectable | IndexLifecycle::Tombstoned => {
            Vec::new()
        }
    };
    for next in transitions {
        if state
            .index_generations
            .get(id)
            .is_some_and(|generation| generation.lifecycle == IndexLifecycle::Active)
        {
            break;
        }
        persist_input(
            state,
            store,
            DomainInput::TransitionIndexGeneration(TransitionIndexGenerationInput { id, to: next }),
        )?;
    }
    Ok(())
}

fn ensure_generation(
    state: &mut KernelState,
    store: &SqliteStore,
    name: RepresentationName,
    fingerprint: IndexFingerprint,
    snapshot: maestria_domain::CorpusSnapshotId,
) -> Result<IndexGenerationId> {
    if let Some(active) = state.index_generations.get_active(&name)
        && active.fingerprint == fingerprint
    {
        return Ok(active.id);
    }
    let matching = state
        .index_generations
        .iter()
        .find(|generation| {
            generation.name == name
                && generation.fingerprint == fingerprint
                && matches!(
                    generation.lifecycle,
                    IndexLifecycle::Building | IndexLifecycle::Evaluated | IndexLifecycle::Shadow
                )
        })
        .map(|generation| generation.id);
    let id = match matching {
        Some(id) => id,
        None => next_generation_id(state),
    };
    if matching.is_none() {
        persist_input(
            state,
            store,
            DomainInput::StartIndexGeneration(StartIndexGenerationInput {
                id,
                name,
                corpus_snapshot: snapshot,
                fingerprint,
            }),
        )?;
    }
    advance_generation(state, store, id)?;
    Ok(id)
}

/// Reconcile lexical and configured dense generations before projections serve.
pub fn reconcile_retrieval_generations(
    layout: &InstanceLayout,
    state: &mut KernelState,
    manifest: &InstanceManifest,
) -> Result<RetrievalGenerations> {
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let lexical_index = TantivyFullTextIndex::open(&layout.full_text_index_dir)
        .with_context(|| "open lexical index for fingerprint")?;
    let snapshot = maestria_domain::CorpusSnapshotId::new(1);
    let primary = ensure_generation(
        state,
        &store,
        RepresentationName::new("lexical_text_v1"),
        lexical_index.fingerprint()?,
        snapshot,
    )?;
    let dense = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| {
            let artifact_hash = ContentHash::new(config.artifact_hash.clone())
                .map_err(|error| anyhow!("invalid embedding artifact hash: {error}"))?;
            let fingerprint = IndexFingerprint {
                provider: config.provider.clone(),
                model: config.model.clone(),
                revision: config.revision.clone(),
                artifact_hash,
                dimensions: u32::try_from(config.dimensions)
                    .map_err(|_| anyhow!("embedding dimensions exceed u32"))?,
                quantization: "f32".to_string(),
                query_template_hash: maestria_domain::content_hash(b"query: {{text}}"),
                document_template_hash: maestria_domain::content_hash(b"doc: {{text}}"),
                preprocessing_version: config.preprocessing_version.clone(),
            };
            ensure_generation(
                state,
                &store,
                RepresentationName::new("dense_text_v1"),
                fingerprint,
                snapshot,
            )
        })
        .transpose()?;
    Ok(RetrievalGenerations { primary, dense })
}

pub fn build_embedding_provider(
    manifest: &InstanceManifest,
    state: &KernelState,
) -> Result<Option<Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>>> {
    manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| {
            let identity = state
                .index_generations
                .get_active(&maestria_domain::RepresentationName::new("dense_text_v1"))
                .map(|generation| maestria_ports::EmbeddingIdentity {
                    generation_id: generation.id,
                    fingerprint: generation.fingerprint.clone(),
                    representation: generation.name.clone(),
                })
                .ok_or_else(|| anyhow!("active dense embedding generation is missing"))?;
            let document_template = "doc: {{text}}";
            let query_template = "query: {{text}}";
            validate_profile_fingerprint(&identity, document_template, query_template)?;
            maestria_embedding_openai::LocalHttpEmbeddingProvider::with_profile(
                &config.endpoint,
                &config.model,
                Some(config.dimensions),
                identity,
                document_template.to_string(),
                query_template.to_string(),
                maestria_ports::ProviderDisclosure {
                    remote: config.remote_provider,
                    retention: config.retention_policy.clone(),
                },
            )
            .map(|provider| {
                Arc::new(provider) as Arc<dyn maestria_ports::EmbeddingProvider + Send + Sync>
            })
            .map_err(Into::into)
        })
        .transpose()
}

fn validate_profile_fingerprint(
    identity: &maestria_ports::EmbeddingIdentity,
    document_template: &str,
    query_template: &str,
) -> Result<()> {
    let expected_document = maestria_domain::content_hash(document_template.as_bytes());
    let expected_query = maestria_domain::content_hash(query_template.as_bytes());
    if identity.fingerprint.document_template_hash != expected_document.as_str()
        || identity.fingerprint.query_template_hash != expected_query.as_str()
    {
        return Err(anyhow!(
            "active embedding generation fingerprint does not match configured templates"
        ));
    }
    Ok(())
}
/// Rebuild the vector projection for a prepared instance from replayed state.
///
/// Application entry points call this shared recovery boundary before starting
/// runtime work so missing, stale, or corrupt vector state is handled once.
pub fn reconcile_vector_projection_for_layout(
    layout: &InstanceLayout,
    state: &KernelState,
) -> Result<()> {
    let manifest_contents = std::fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let embedding_provider = build_embedding_provider(&manifest, state)?;
    let embedding_model = manifest
        .embeddings
        .as_ref()
        .filter(|config| config.enabled)
        .map(|config| config.model.as_str());
    let vector_index = SqliteVectorIndex::open(layout.vector_index_dir.join("projection.db"))
        .with_context(|| format!("open vector index {}", layout.vector_index_dir.display()))?;
    reconcile_vector_projection(
        state,
        &vector_index,
        embedding_provider.as_deref(),
        embedding_model,
    )
    .with_context(|| "reconcile vector projection")
}
