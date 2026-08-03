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
#[cfg(test)]
#[path = "tests_persistence.rs"]
mod tests_persistence;
#[cfg(test)]
#[path = "tests_schema.rs"]
mod tests_schema;
#[cfg(test)]
#[path = "tests_scoring.rs"]
mod tests_scoring;
#[cfg(test)]
mod tests_support;
mod vector_index;
pub use vector_index::SqliteVectorIndex;
