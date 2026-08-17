//! Deterministic fixtures and test helpers shared across Maestria test suites.
//!
//! This crate is dev-only (`publish = false`): every item exists to keep test
//! fixtures byte-identical across crates while removing duplicated literals
//! and ad-hoc helper functions.
//!
//! Responsibility map:
//! - `error`: shared helper error type.
//! - `git`: git invocation helper for fixture repositories.
//! - `fs`: recursive tree-copy helper for fixture directories.
//! - `fixtures`: deterministic content-hash and realm-id fixture generators.

mod error;
pub use error::TestSupportError;
mod fixtures;
pub use fixtures::{content_hash, content_hash_str, realm_id, realm_id_str};
mod fs;
pub use fs::copy_tree;
mod git;
pub use git::run_git;
