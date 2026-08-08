#![forbid(unsafe_code)]

//!
//! Responsibility map:
//! - `dto`: wire-format DTOs for the sparse encoding HTTP contract.
//! - `sparse_provider`: local learned sparse provider.

mod dto;
mod sparse_provider;
pub use sparse_provider::LocalHttpSparseProvider;
