use maestria_domain::{
    IndexFingerprint, IndexGenerationId, RepresentationName, SearchExecutionBudget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    pub generation_id: IndexGenerationId,
    pub fingerprint: IndexFingerprint,
    pub representation: RepresentationName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingInputKind {
    Document,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionPolicy {
    NoRetention,
    ProviderDefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDisclosure {
    pub remote: bool,
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProvenance {
    pub content_hash: String,
    pub identity: EmbeddingIdentity,
    pub provider_id: String,
    pub model: String,
    pub model_version: String,
    pub disclosure: ProviderDisclosure,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorEmbedding {
    pub chunk_id: maestria_domain::ChunkId,
    pub vector: Vec<f32>,
    pub provenance: EmbeddingProvenance,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorSearchQuery {
    pub vector: Vec<f32>,
    pub limit: u32,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub model_version: Option<String>,
    pub identity: Option<EmbeddingIdentity>,
    pub execution_budget: SearchExecutionBudget,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchHit {
    pub chunk_id: maestria_domain::ChunkId,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingRequest {
    pub text: String,
    pub model: String,
    pub kind: EmbeddingInputKind,
    pub identity: EmbeddingIdentity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingResponse {
    pub vector: Vec<f32>,
    pub provider_id: String,
    pub model: String,
    pub model_version: String,
    pub identity: EmbeddingIdentity,
    pub disclosure: ProviderDisclosure,
}

pub trait EmbeddingProvider: Send + Sync {
    /// The transport-bound disclosure that must be checked before input bytes.
    fn disclosure(&self) -> ProviderDisclosure;
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, crate::PortError>;

    /// Embed a batch with one provider round-trip when the transport
    /// supports it. Responses are position-aligned with `requests`.
    /// The default loops [`EmbeddingProvider::embed`]; providers with a
    /// batched transport override this.
    fn embed_batch(
        &self,
        requests: &[EmbeddingRequest],
    ) -> Result<Vec<EmbeddingResponse>, crate::PortError> {
        requests
            .iter()
            .cloned()
            .map(|request| self.embed(request))
            .collect()
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        None
    }
}
/// Durable identity of one indexed embedding, for incremental projection
/// recovery: a chunk can skip re-embedding when its content hash and
/// generation identity still match the active embedding profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedEmbeddingKey {
    pub chunk_id: maestria_domain::ChunkId,
    pub content_hash: String,
    pub generation_id: String,
    pub representation: String,
    pub fingerprint: String,
}

pub trait VectorIndex: Send + Sync {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), crate::PortError>;
    fn search_similar(
        &self,
        query: VectorSearchQuery,
    ) -> Result<crate::BoundedSearch<VectorSearchHit>, crate::PortError>;

    /// Identity keys of every indexed embedding, used by projection
    /// recovery to keep embeddings whose provenance still matches.
    fn indexed_embedding_keys(&self) -> Result<Vec<IndexedEmbeddingKey>, crate::PortError> {
        Err(crate::PortError::InternalContext {
            context: "embedding key readback is unsupported",
            source: "index must implement embedding key readback".to_string(),
        })
    }

    /// Reconcile the projection to `expected` chunk ids: upsert
    /// `upserted` embeddings and drop every chunk not listed in
    /// `expected`. Chunks absent from `upserted` but present in
    /// `expected` keep their existing embedding.
    fn reconcile_projection(
        &self,
        upserted: Vec<VectorEmbedding>,
        expected: &[maestria_domain::ChunkId],
    ) -> Result<(), crate::PortError> {
        let _ = (upserted, expected);
        Err(crate::PortError::InternalContext {
            context: "projection reconciliation is unsupported",
            source: "index must implement projection reconciliation".to_string(),
        })
    }

    /// Execute a vector search, applying a pre-score filter.
    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId) -> Result<bool, crate::PortError>,
    ) -> Result<crate::BoundedSearch<VectorSearchHit>, crate::PortError> {
        let _ = (query, filter);
        Err(crate::PortError::InternalContext {
            context: "filtered vector search is unsupported",
            source: "index must implement governed pre-score filtering".to_string(),
        })
    }
    fn delete_chunks(&self, chunk_ids: &[maestria_domain::ChunkId])
    -> Result<(), crate::PortError>;
    fn clear(&self) -> Result<(), crate::PortError>;
    fn rebuild(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), crate::PortError> {
        self.clear()?;
        self.index_embeddings(embeddings)
    }
}
