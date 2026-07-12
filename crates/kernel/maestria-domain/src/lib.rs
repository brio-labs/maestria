#![forbid(unsafe_code)]

//! Deterministic domain kernel for Maestria.
//!
//! This module is pure and side-effect free. All environment interaction is
//! represented via `MaestriaEffect` values and executed by a runtime layer.

mod input;
mod provenance;
mod replay;
mod types;

pub use provenance::{content_hash, evidence_id_for, excerpt_for, line_range_for_chunk};
pub use replay::{replay_events, replay_inputs};
pub use types::*;
