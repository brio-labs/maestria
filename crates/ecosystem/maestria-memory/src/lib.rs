#![forbid(unsafe_code)]

//! Pure memory workflow orchestration for Maestria.
//!
//! Responsibility map:
//! - `memory_service`: review workflow analysis.

mod memory_service;
pub use memory_service::review_queue;

#[cfg(test)]
mod tests;
