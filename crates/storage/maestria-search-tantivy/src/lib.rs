#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

/// Responsibility map:
/// - `constructors`: module responsibility.
/// - `lexical_scoring`: module responsibility.
/// - `lexical_helpers`: lexical query construction and scoring helpers.
/// - `lexical_operations`: module responsibility.
/// - `migration`: module responsibility.
/// - `operations`: module responsibility.
/// - `execution`: bounded search execution metering.
/// - `operations_cards`: card indexing and bounded search operations.
/// - `operations_chunks`: chunk indexing and bounded search operations.
/// - `schema`: module responsibility.
/// - `search_helpers`: module responsibility.
/// - `documents`: Tantivy document conversion.
/// - `error`: Tantivy and I/O error conversion helpers.
/// - `keys`: document key formatting helpers.
/// - `scoring`: score ordering and quantization helpers.
/// - `tantivy_index`: public Tantivy index façade.
mod constructors;
mod documents;
mod error;
mod execution;
mod keys;
mod lexical_helpers;
mod lexical_operations;
mod lexical_scoring;
mod migration;
mod operations;
mod operations_cards;
mod operations_chunks;
mod schema;
mod scoring;
mod search_helpers;

mod tantivy_index;
pub use tantivy_index::TantivyFullTextIndex;

#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;
