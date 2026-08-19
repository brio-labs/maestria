use std::sync::Arc;

use maestria_code_intel::RepositoryCodeIndex;
use maestria_domain::{CorpusSnapshotId, IndexGenerationId};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, GraphIndex, VectorIndex,
};
use maestria_retrieval::{CandidateRetriever, RepositoryExecutionPolicy};
use maestria_storage_sqlite::SqliteStore;

use super::SearchRuntime;

pub(crate) struct SearchRuntimeParts {
    pub(crate) artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub(crate) cards: Arc<dyn CardRepository + Send + Sync>,
    pub(crate) chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub(crate) evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub(crate) search_index: Arc<dyn FullTextIndex + Send + Sync>,
    pub(crate) blobs: Arc<dyn BlobStore + Send + Sync>,
    pub(crate) vector_index: Option<Arc<dyn VectorIndex + Send + Sync>>,
    pub(crate) graph_index: Option<Arc<dyn GraphIndex + Send + Sync>>,
    pub(crate) event_log: Arc<SqliteStore>,
    pub(crate) primary_generation: IndexGenerationId,
    pub(crate) dense_generation: Option<IndexGenerationId>,
    pub(crate) repository_code_index: Option<Arc<RepositoryCodeIndex>>,
    pub(crate) repository_execution_policy: RepositoryExecutionPolicy,
    pub(crate) hybrid_execution_policy: maestria_retrieval::HybridExecutionPolicy,
    pub(crate) learned_sparse_execution_policy: maestria_retrieval::LearnedSparseExecutionPolicy,
    pub(crate) sparse_retriever: Option<Arc<dyn CandidateRetriever>>,
    pub(crate) corpus_snapshot: CorpusSnapshotId,
    pub(crate) scope_id: maestria_domain::ScopeId,
}

impl Clone for SearchRuntime {
    fn clone(&self) -> Self {
        Self {
            artifacts: self.artifacts.clone(),
            cards: self.cards.clone(),
            chunks: self.chunks.clone(),
            evidence: self.evidence.clone(),
            search_index: self.search_index.clone(),
            blobs: self.blobs.clone(),
            vector_index: self.vector_index.clone(),
            visual_vector_index: self.visual_vector_index.clone(),
            graph_index: self.graph_index.clone(),
            event_log: self.event_log.clone(),
            persist_learned_sparse_observations: self.persist_learned_sparse_observations,
            embedding_provider: self.embedding_provider.clone(),
            reranker: self.reranker.clone(),
            retrieval_policy: self.retrieval_policy.clone(),
            primary_generation: self.primary_generation,
            dense_generation: self.dense_generation,
            visual_embedding_provider: self.visual_embedding_provider.clone(),
            visual_generation: self.visual_generation.clone(),
            repository_code_index: self.repository_code_index.clone(),
            repository_execution_policy: self.repository_execution_policy.clone(),
            hybrid_execution_policy: self.hybrid_execution_policy.clone(),
            visual_execution_policy: self.visual_execution_policy.clone(),
            learned_sparse_execution_policy: self.learned_sparse_execution_policy.clone(),
            sparse_retriever: self.sparse_retriever.clone(),
            corpus_snapshot: self.corpus_snapshot,
            scope_id: self.scope_id,
            fingerprint: self.fingerprint.clone(),
            engine_cache: self.engine_cache.clone(),
        }
    }
}
