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
    pub(super) fn retrieval_engine(&self) -> Result<RetrievalEngine> {
        let events = self.domain_events()?;
        let active_versions = reconcile_active_versions(&events);
        let card: Arc<dyn CandidateRetriever> = Arc::new(CurrentVersionFilter::new(
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
        ));
        let lexical: Arc<dyn CandidateRetriever> = Arc::new(CurrentVersionFilter::new(
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
        ));
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = vec![card, lexical];
        if let Some(index) = self.repository_code_index.clone() {
            let security = CodeIntelSecurityResolver::from_events(
                CodeIntelSecurityResolverParts {
                    artifacts: self.artifacts.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                &events,
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
        if let Some(retriever) = self.visual_retriever(active_versions) {
            retrievers.push(retriever);
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
        if let Some(reranker) = self.reranker.clone() {
            engine = engine.with_visual_reranker(reranker);
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
            .with_hybrid_policy(HybridExecutionPolicy::Shadow)
            .with_repository_execution_policy(self.repository_execution_policy.clone())
            .with_visual_execution_policy(self.visual_execution_policy.clone()))
    }
}
