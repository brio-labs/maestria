use std::sync::Arc;

use super::{RetrievalEngine, engine_capabilities, learned_sparse_shadow};
use crate::traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RetrievalEvaluator,
};

impl RetrievalEngine {
    pub fn new(
        retrievers: Vec<Arc<dyn CandidateRetriever>>,
        evaluator: Arc<dyn RetrievalEvaluator>,
        security_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self {
        let capabilities = engine_capabilities::capabilities_from_retrievers(&retrievers);
        Self {
            retrievers,
            fusion: None,
            reranker: None,
            visual_reranker: false,
            expander: None,
            evaluator,
            capabilities,
            security_policy,
            hybrid_policy: crate::types::HybridExecutionPolicy::Shadow,
            learned_sparse_execution_policy:
                crate::learned_sparse_policy::LearnedSparseExecutionPolicy::Shadow,
            learned_sparse_shadow_store: learned_sparse_shadow::LearnedSparseShadowStore::default(),
            repository_execution_policy:
                crate::repository_benchmark::RepositoryExecutionPolicy::Shadow,
            visual_execution_policy: crate::visual_benchmark::VisualExecutionPolicy::Shadow,
        }
    }

    pub fn with_hybrid_policy(mut self, policy: crate::types::HybridExecutionPolicy) -> Self {
        self.hybrid_policy = policy;
        self
    }

    pub fn with_learned_sparse_execution_policy(
        mut self,
        policy: crate::learned_sparse_policy::LearnedSparseExecutionPolicy,
    ) -> Self {
        self.learned_sparse_execution_policy = policy;
        self
    }

    pub fn with_learned_sparse_shadow_store(
        mut self,
        store: learned_sparse_shadow::LearnedSparseShadowStore,
    ) -> Self {
        self.learned_sparse_shadow_store = store;
        self
    }

    pub fn with_learned_sparse_observation_repository(
        mut self,
        repository: Arc<dyn maestria_ports::LearnedSparseObservationRepository>,
    ) -> Self {
        self.learned_sparse_shadow_store =
            self.learned_sparse_shadow_store.with_repository(repository);
        self
    }

    pub fn learned_sparse_shadow_store(&self) -> learned_sparse_shadow::LearnedSparseShadowStore {
        self.learned_sparse_shadow_store.clone()
    }

    pub fn with_repository_execution_policy(
        mut self,
        policy: crate::repository_benchmark::RepositoryExecutionPolicy,
    ) -> Self {
        self.repository_execution_policy = policy;
        self
    }

    pub fn with_visual_execution_policy(
        mut self,
        policy: crate::visual_benchmark::VisualExecutionPolicy,
    ) -> Self {
        self.visual_execution_policy = policy;
        self
    }

    pub fn with_capabilities(
        mut self,
        capabilities: maestria_governance::SearchCapabilities,
    ) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_fusion(mut self, fusion: Arc<dyn RankFusion>) -> Self {
        self.fusion = Some(fusion);
        self
    }

    pub fn with_reranker(mut self, reranker: Arc<dyn CandidateReranker>) -> Self {
        self.reranker = Some(reranker);
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Reranking);
        self
    }

    pub fn with_visual_reranker(mut self, reranker: Arc<dyn CandidateReranker>) -> Self {
        self.reranker = Some(reranker);
        self.visual_reranker = true;
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Reranking);
        self
    }

    pub fn with_expander(mut self, expander: Arc<dyn ContextExpander>) -> Self {
        self.expander = Some(expander);
        self.capabilities = self
            .capabilities
            .clone()
            .with_stage(maestria_domain::SearchStage::Filtering);
        self
    }
}
