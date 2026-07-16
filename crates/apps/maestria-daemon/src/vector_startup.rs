use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;
use maestria_vector_sqlite::SqliteVectorIndex;

use crate::reconcile_vector_projection;

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
                .map_or_else(
                    || maestria_ports::EmbeddingIdentity::legacy(&config.model, config.dimensions),
                    Ok,
                )?;
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
    if identity.fingerprint.revision == "legacy" {
        return Ok(());
    }
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
