#![forbid(unsafe_code)]

//! SQLite-backed vector projection for Maestria.
//!
//! The table in this crate is a rebuildable projection: the domain event log and
//! chunk store remain the source of truth. The adapter attempts to create a
//! `sqlite-vec` virtual table when the extension is already available on the
//! supplied connection, and always maintains a portable BLOB-backed table used by
//! the `VectorIndex` implementation.

/// Responsibility map:
/// - `encoding`: module responsibility.
/// - `operations`: vector projection mutations.
/// - `vector_index`: public vector index façade.
/// - `schema`: module responsibility.
mod encoding;
mod operations;
mod schema;
#[cfg(test)]
mod tests;
mod vector_index;
pub use vector_index::SqliteVectorIndex;
