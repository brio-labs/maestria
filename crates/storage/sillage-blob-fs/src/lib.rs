#![forbid(unsafe_code)]

//! Content-addressed filesystem implementation of the Sillage blob port.
//!
//! Responsibility map:
//! - `store`: content-addressed blob storage implementation.

mod store;
pub use store::FsBlobStore;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
