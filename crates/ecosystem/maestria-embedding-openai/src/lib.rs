//! Responsibility map:
//! - `embedding_provider`: OpenAI-compatible embedding provider.
//!
mod embedding_provider;
pub use embedding_provider::{
    EmbeddingTransport, LocalHttpEmbeddingProvider, parse_loopback_endpoint,
};
