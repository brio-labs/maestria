use maestria_domain::{IndexFingerprint, IndexGenerationId, RepresentationName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    pub generation_id: IndexGenerationId,
    pub fingerprint: IndexFingerprint,
    pub representation: RepresentationName,
}

impl EmbeddingIdentity {
    pub fn legacy(model: impl Into<String>, dimensions: usize) -> Result<Self, crate::PortError> {
        let artifact_hash = maestria_domain::ContentHash::new(format!("sha256:{}", "0".repeat(64)))
            .map_err(|error| crate::PortError::Internal {
                message: format!("create legacy embedding fingerprint: {error}"),
            })?;
        Ok(Self {
            generation_id: IndexGenerationId::new(1),
            fingerprint: IndexFingerprint {
                provider: "legacy-local".to_string(),
                model: model.into(),
                revision: "legacy".to_string(),
                artifact_hash,
                dimensions: dimensions as u32,
                quantization: "f32".to_string(),
                query_template_hash: "legacy-query".to_string(),
                document_template_hash: "legacy-document".to_string(),
                preprocessing_version: "legacy".to_string(),
            },
            representation: RepresentationName::new("dense_text_v1"),
        })
    }
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
    ) -> Result<Vec<VectorSearchHit>, crate::PortError>;

    /// Execute a vector search, applying a pre-score filter.
    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(maestria_domain::ChunkId) -> bool,
    ) -> Result<Vec<VectorSearchHit>, crate::PortError> {
        let _ = (query, filter);
        Err(crate::PortError::Internal {
            message: "search_similar_filtered not supported by this index".into(),
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
