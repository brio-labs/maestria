#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Sillage.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

/// Responsibility map:
/// - `constructors`: durable/read-only and ephemeral admission constructors.
/// - `lexical_operations`: canonical lexical-metadata indexing adapters.
/// - `migration`: legacy schema detection and stored-chunk recovery.
/// - `operations`: public port and whole-batch quota-prepared artifact indexing.
/// - `execution`: bounded search execution metering.
/// - `operations_cards`: card indexing and bounded search operations.
/// - `operations_chunks`: chunk indexing and bounded search operations.
/// - `schema`: canonical fields and query-index schema compatibility.
/// - `search_helpers`: bounded native collection, parsing and key-scoped queries.
/// - `documents`: Tantivy document decoding.
/// - `error`: Tantivy and I/O error conversion helpers.
/// - `keys`: document key formatting helpers.
/// - `scoring`: score ordering and quantization helpers.
/// - `memory_document`: document construction and reservation estimates.
/// - `memory_query`: query workspace and transient output admission estimates.
/// - `ram_directory`: RAM directory file retention and reservation tracking.
/// - `ram_memory`: shared byte ledger and reservation/error types.
/// - `tantivy_index`: public Tantivy index façade.
mod constructors;
mod documents;
mod error;
mod execution;
mod keys;
mod lexical_operations;
mod memory_document;
mod memory_query;
mod migration;
mod operations;
mod operations_cards;
mod operations_chunks;
mod ram_directory;
mod ram_memory;
mod schema;
mod scoring;
mod search_helpers;

pub use ram_memory::{
    InMemoryIndexError, MemoryBudgetError, RamMemoryBudget, RamMemoryReservation,
};

mod tantivy_index;
pub use tantivy_index::TantivyFullTextIndex;

#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_support;
#[cfg(test)]
pub(crate) use tests_support::test_ram_index;
