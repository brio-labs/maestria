#![forbid(unsafe_code)]

//! Pure memory workflow orchestration for Maestria.
//!
//! Responsibility map:
//! - `memory_service`: promotion, contradiction, duplicate, review, and lifecycle workflows.

mod memory_service;
pub use memory_service::{
    ContradictionCheck, MemoryService, PromoteMemoryInput, PromoteMemoryOutput,
};

#[cfg(test)]
mod tests;
