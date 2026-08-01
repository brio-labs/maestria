//! Optional visual-runtime activation for [`SearchRuntime`].
//!
//! The visual lane is an independently testable concern: provider identity
//! and disclosure checks, generation activation, projection rebuild, and
//! reranker construction. It lives outside `search_executor.rs` so the
//! runtime assembly module keeps one responsibility.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_domain::IndexGenerationRegistry;
use maestria_ports::{VectorIndex, VisualEmbeddingProvider};
use maestria_retrieval::adapters::{
    VisualGenerationCapability, VisualProjectionRebuildParts, rebuild_visual_projection,
};
use maestria_retrieval::{
    RerankLimits, VisualExecutionPolicy, VisualReranker, VisualRerankerParts,
};

use super::SearchRuntime;

impl SearchRuntime {
    /// Enables the optional visual page/region lane for this runtime.
    ///
    /// The provider, visual index, and active registry generation are supplied
    /// separately so text and visual representations cannot share rows.
    pub fn with_visual_embedding_provider(
        self: Arc<Self>,
        provider: Arc<dyn VisualEmbeddingProvider + Send + Sync>,
        visual_index: Arc<dyn VectorIndex + Send + Sync>,
        registry: &IndexGenerationRegistry,
    ) -> Result<Arc<Self>> {
        let mut runtime = (*self).clone();
        runtime.configure_visual_embedding_provider(provider, visual_index, registry)?;
        Ok(Arc::new(runtime))
    }

    fn configure_visual_embedding_provider(
        &mut self,
        provider: Arc<dyn VisualEmbeddingProvider + Send + Sync>,
        visual_index: Arc<dyn VectorIndex + Send + Sync>,
        registry: &IndexGenerationRegistry,
    ) -> Result<()> {
        let identity = provider
            .identity()
            .ok_or_else(|| anyhow!("visual provider identity is unavailable"))?;
        let disclosure = provider.disclosure();
        if disclosure.remote || disclosure.retention != maestria_ports::RetentionPolicy::NoRetention
        {
            return Err(anyhow!(
                "visual provider must be local and no-retention before activation"
            ));
        }
        let capability =
            VisualGenerationCapability::activate(registry, identity, self.corpus_snapshot)
                .map_err(|error| anyhow!("activate visual generation: {error}"))?;
        let artifact_ids = self
            .current_artifact_versions()?
            .into_iter()
            .map(|version| maestria_domain::ArtifactId::new(version.value()))
            .collect::<Vec<_>>();
        rebuild_visual_projection(
            VisualProjectionRebuildParts {
                index: visual_index.as_ref(),
                artifacts: self.artifacts.as_ref(),
                chunks: self.chunks.as_ref(),
                evidence: self.evidence.as_ref(),
                blobs: self.blobs.as_ref(),
                policy: &self.retrieval_policy,
                provider: provider.as_ref(),
            },
            &artifact_ids,
            &capability,
        )
        .map_err(|error| anyhow!("rebuild visual projection: {error}"))?;
        self.visual_vector_index = Some(visual_index);
        self.visual_embedding_provider = Some(provider);
        self.visual_generation = Some(capability);
        Ok(())
    }

    /// Installs the optional visual reranker using the active visual capability.
    pub fn with_visual_reranker(self: Arc<Self>, limits: RerankLimits) -> Result<Arc<Self>> {
        let provider = self
            .visual_embedding_provider
            .clone()
            .ok_or_else(|| anyhow!("visual embedding provider is not configured"))?;
        let capability = self
            .visual_generation
            .clone()
            .ok_or_else(|| anyhow!("visual generation is not configured"))?;
        let reranker = VisualReranker::new(
            VisualRerankerParts {
                artifacts: self.artifacts.clone(),
                evidence: self.evidence.clone(),
                blobs: self.blobs.clone(),
                provider,
                capability,
                policy: self.retrieval_policy.clone(),
            },
            limits,
        )
        .map_err(|error| anyhow!("create visual reranker: {error}"))?;
        let mut runtime = (*self).clone();
        runtime.reranker = Some(Arc::new(reranker));
        Ok(Arc::new(runtime))
    }

    /// Installs benchmark evidence governing visual lane activation.
    pub fn with_visual_execution_policy(
        self: Arc<Self>,
        policy: VisualExecutionPolicy,
    ) -> Arc<Self> {
        let mut runtime = (*self).clone();
        runtime.visual_execution_policy = policy;
        Arc::new(runtime)
    }
}
