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
    fn identity(&self) -> Option<EmbeddingIdentity> {
        None
    }
}
pub trait VectorIndex: Send + Sync {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), crate::PortError>;
    fn search_similar(
        &self,
        query: VectorSearchQuery,
    ) -> Result<crate::BoundedSearch<VectorSearchHit>, crate::PortError>;

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
