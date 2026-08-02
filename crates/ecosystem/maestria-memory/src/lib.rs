#![forbid(unsafe_code)]

//! Pure memory workflow orchestration for Maestria.
//!
//! Responsibility map:
//! - `memory_service`: contradiction, duplicate, and review analysis workflows.

mod memory_service;
pub use memory_service::{ContradictionCheck, MemoryService};

#[cfg(test)]
mod tests;
