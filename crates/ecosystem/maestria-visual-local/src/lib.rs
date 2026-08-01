#![forbid(unsafe_code)]

//!
//! Responsibility map:
//! - `dto`: wire-format DTOs for the visual embedding HTTP contract.
//! - `visual_provider`: local visual embedding provider.

mod dto;
mod visual_provider;
pub use visual_provider::LocalHttpVisualProvider;
