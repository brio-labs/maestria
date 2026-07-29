#![forbid(unsafe_code)]

//! Tantivy-backed full-text search projection for Maestria.
//!
//! This crate stores only rebuildable indexed chunks. Artifact metadata and blob
//! contents remain owned by their source repositories.

/// Responsibility map:
/// - `constructors`: module responsibility.
/// - `lexical_helpers`: module responsibility.
/// - `lexical_operations`: module responsibility.
/// - `migration`: module responsibility.
/// - `operations`: module responsibility.
/// - `schema`: module responsibility.
/// - `search_helpers`: module responsibility.
/// - `documents`: Tantivy document conversion.
/// - `tantivy_index`: public Tantivy index façade.
mod constructors;
mod documents;
mod lexical_helpers;
mod lexical_operations;
mod migration;
mod operations;
mod schema;
mod search_helpers;

mod tantivy_index;
pub use tantivy_index::TantivyFullTextIndex;

#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;
