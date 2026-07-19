use maestria_domain::BlobId;

use super::traits::{EmbeddingIdentity, EmbeddingResponse, PortError, ProviderDisclosure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualSource {
    Page {
        blob: BlobId,
        page_start: u32,
        page_end: u32,
    },
    Region {
        blob: BlobId,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualEmbeddingRequest {
    pub source: VisualSource,
    pub bytes: Vec<u8>,
    pub identity: EmbeddingIdentity,
}

/// Optional provider boundary for rendered pages and PDF regions.
///
/// Implementations must use a visual model; text-only providers must not be
/// adapted to this trait. Absence of a provider is an explicit capability
/// downgrade to text/layout retrieval.
pub trait VisualEmbeddingProvider: Send + Sync {
    /// Declares the provider's locality and retention behavior before input.
    fn disclosure(&self) -> Option<ProviderDisclosure>;
    fn embed_query(
        &self,
        query: &str,
        identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError>;
    fn embed_source(&self, request: VisualEmbeddingRequest)
    -> Result<EmbeddingResponse, PortError>;
    fn identity(&self) -> Option<EmbeddingIdentity>;
}
