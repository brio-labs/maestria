#![forbid(unsafe_code)]

//! SQLite-backed metadata and event-log adapter for Maestria.
//!
//! This crate intentionally keeps storage serialization at the port boundary:
//! domain types do not implement or depend on serde.

/// Responsibility map:
/// - `events`: module responsibility.
/// - `id_allocator`: module responsibility.
/// - `journal`: durable effect-journal port implementation.
/// - `learned_sparse_io`: learned-sparse shadow observation JSON import/export.
/// - `legacy`: legacy stored-payload upcasting and kind mapping.
/// - `payloads`: module responsibility.
/// - `projection_cleanup`: stale projection row removal.
/// - `repositories`: module responsibility.
/// - `learned_sparse_projection`: durable sparse projection adapter.
/// - `schema`: module responsibility.
/// - `schema_validation`: module responsibility.
/// - `sqlite_store`: public SQLite store façade.
mod events;
mod id_allocator;
mod journal;
mod learned_sparse_io;
mod learned_sparse_projection;
mod legacy;
mod payloads;
mod projection_cleanup;
mod repositories;
mod schema;
mod schema_validation;

mod sqlite_store;
pub use learned_sparse_projection::SqliteLearnedSparseIndex;
pub use sqlite_store::SqliteStore;

#[cfg(test)]
mod tests;
