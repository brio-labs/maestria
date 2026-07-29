#![forbid(unsafe_code)]

//!
//! Responsibility map:
//! - `visual_provider`: local visual embedding provider.

mod visual_provider;
pub use visual_provider::LocalHttpVisualProvider;
