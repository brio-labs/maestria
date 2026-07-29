#![forbid(unsafe_code)]

//! SQLite-backed graph projection for Maestria.
//!
//! This crate stores domain relations in a rebuildable edge table. The domain
//! event log remains the source of truth; this adapter only serves `GraphIndex`
//! reads for projected graph edges.
//!
//! Runtime wiring owns projection updates; this adapter only persists and
//! serves rebuildable graph edges.

/// Responsibility map:
/// - `conversion`: module responsibility.
/// - `migration`: module responsibility.
/// - `graph`: graph index implementation.
mod conversion;
mod graph;
mod migration;
pub use graph::SqliteGraphIndex;

#[cfg(test)]
mod tests;
