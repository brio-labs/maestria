use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_retrieval::adapters::{
    CardRetriever, CardRetrieverParts, CodeIntelRetriever, CodeIntelRetrieverParts,
    CodeIntelSecurityResolver, CodeIntelSecurityResolverParts, CurrentVersionFilter,
    DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
    HierarchyGraphExpander, HierarchyGraphExpanderParts, LexicalChunkRetriever,
    LexicalChunkRetrieverParts,
};
use maestria_retrieval::{CandidateRetriever, FixedKRrf, HybridExecutionPolicy, RetrievalEngine};

use super::{SearchRuntime, reconcile_active_versions};

impl SearchRuntime {
    /// The production engine: the loaded hybrid policy (dense lane), the
    /// loaded sparse policy, and the registered sparse lane.
    pub(crate) fn retrieval_engine(&self) -> Result<RetrievalEngine> {
        self.retrieval_engine_with_policies(
            self.hybrid_execution_policy.clone(),
            self.learned_sparse_execution_policy.clone(),
            self.sparse_retriever.clone(),
            true,
        )
    }

    /// One shared assembly for every engine variant.
    ///
    /// The benchmark executor and the daemon both build engines here (R28);
    /// only the policies, the optional learned-sparse lane, and whether the
    /// base retrievers are registered differ.
    /// The base serving lanes: cards, lexical chunks, repository code, and dense
    /// chunks, all generation-filtered.
    fn base_retrievers(
        &self,
        events: &[maestria_domain::DomainEventEnvelope],
        sources: &std::collections::BTreeMap<
            std::path::PathBuf,
            (
                maestria_domain::ArtifactId,
                maestria_domain::ArtifactVersionId,
                maestria_domain::ContentHash,
            ),
        >,
        active_versions: std::collections::BTreeSet<maestria_domain::ArtifactVersionId>,
    ) -> Result<Vec<Arc<dyn CandidateRetriever>>> {
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = Vec::new();
        retrievers.push(Arc::new(CurrentVersionFilter::new(
            Arc::new(CardRetriever::new(
                CardRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    cards: self.cards.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        )));
        retrievers.push(Arc::new(CurrentVersionFilter::new(
            Arc::new(LexicalChunkRetriever::new(
                LexicalChunkRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        )));
        if let Some(index) = self.repository_code_index.clone() {
            let security = CodeIntelSecurityResolver::from_events(
                CodeIntelSecurityResolverParts {
                    artifacts: self.artifacts.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                sources,
                events,
            )
            .map_err(|error| anyhow!("prepare repository code security resolver: {error}"))?;
            retrievers.push(Arc::new(CodeIntelRetriever::new(
                CodeIntelRetrieverParts { index, security },
                self.primary_generation,
            )));
        }
        if let (Some(vector_index), Some(provider), Some(generation)) = (
            self.vector_index.clone(),
            self.embedding_provider.clone(),
            self.dense_generation,
        ) {
            retrievers.push(Arc::new(CurrentVersionFilter::new(
                Arc::new(DenseChunkRetriever::new(
                    DenseChunkRetrieverParts {
                        index: vector_index,
                        artifacts: self.artifacts.clone(),
                        chunks: self.chunks.clone(),
                        evidence: self.evidence.clone(),
                        blobs: self.blobs.clone(),
                        embedding_provider: provider,
                    },
                    generation,
                )),
                active_versions.clone(),
            )));
        }
        Ok(retrievers)
    }

    pub(crate) fn retrieval_engine_with_policies(
        &self,
        hybrid_policy: HybridExecutionPolicy,
        sparse_policy: maestria_retrieval::LearnedSparseExecutionPolicy,
        sparse_retriever: Option<Arc<dyn CandidateRetriever>>,
        include_base_retrievers: bool,
    ) -> Result<RetrievalEngine> {
        let events = self.domain_events()?;
        // Single projection scan shared by the version filter and the
        // repository-code security resolver.
        let sources = maestria_domain::active_source_versions(&events);
        let active_versions = reconcile_active_versions(&sources);
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = Vec::new();
        if include_base_retrievers {
            retrievers = self.base_retrievers(&events, &sources, active_versions)?;
        }
        // The sparse lane registers after the base lanes so the engine's
        // primary generation stays the lexical generation (R24).
        if let Some(sparse_retriever) = sparse_retriever {
            retrievers.push(sparse_retriever);
        }
        let mut engine = RetrievalEngine::new(
            retrievers,
            Arc::new(EvidenceOutcomeEvaluator::new(self.evidence.clone())),
            self.retrieval_policy.clone(),
        )
        .with_fusion(Arc::new(FixedKRrf::new(60)));
        if self.persist_learned_sparse_observations {
            engine = engine.with_learned_sparse_observation_repository(self.event_log.clone());
        }
        if let Some(graph) = self.graph_index.clone() {
            engine = engine.with_expander(Arc::new(HierarchyGraphExpander::new(
                HierarchyGraphExpanderParts {
                    graph,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
            )));
        }
        Ok(engine
            .with_hybrid_policy(hybrid_policy)
            .with_learned_sparse_execution_policy(sparse_policy)
            .with_repository_execution_policy(self.repository_execution_policy.clone()))
    }
}
