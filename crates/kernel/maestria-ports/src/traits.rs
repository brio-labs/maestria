//! Responsibility map:
//! - `errors`: module responsibility.
//! - `repositories`: module responsibility.
//! - `lifecycle`: module responsibility.
//! - `indexing`: module responsibility.
//! - `embedding`: module responsibility.
//! - `provider_transport`: module responsibility.
//! - `harness`: module responsibility.
//! - `graph`: module responsibility.
//! - `web`: module responsibility.
//! - `approval`: module responsibility.
//! - `search`: module responsibility.

pub use maestria_domain::HarnessRunId;
mod approval;
mod embedding;
mod errors;
mod graph;
mod harness;
mod indexing;
mod lifecycle;
mod provider_transport;
mod repositories;
mod search;
pub use provider_transport::{ProviderEndpoint, ProviderTransport};
mod web;

pub use approval::*;
pub use embedding::*;
pub use errors::PortError;
pub use graph::*;
pub use harness::*;
pub use lifecycle::*;
pub use repositories::*;
pub use search::{SearchFuture, SearchKnowledgeExecutor};
pub use web::*;

pub use indexing::{
    BoundedSearch, CardHit, FileHandle, FileMetadata, IndexedCard, IndexedChunk, SearchHit,
    SearchQuery,
};
