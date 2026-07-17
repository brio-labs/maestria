#![forbid(unsafe_code)]

//! Deterministic domain kernel for Maestria.
//!
//! This module is pure and side-effect free. All environment interaction is
//! represented via `MaestriaEffect` values and executed by a runtime layer.

mod effects;
mod entities;
mod errors;
mod events;
mod evidence_pack;
mod generations;
mod ids;
mod input;
mod inputs;
mod kernel_state;
mod provenance;
mod replay;
mod search;
mod security;
mod types;

// Public API — stable boundary types re-exported at crate root.
// Only `pub` items from each module are re-exported; `pub(crate)` items
// (constructors, internal constants) remain crate-internal.
pub use crate::effects::*;
pub use crate::entities::*;
pub use crate::errors::*;
pub use crate::events::*;
pub use crate::evidence_pack::*;
pub use crate::generations::*;
pub use crate::ids::*;
pub use crate::inputs::*;
pub use crate::kernel_state::*;

pub use crate::provenance::*;
pub use crate::search::*;
pub use crate::security::*;
pub use replay::{replay_events, replay_inputs};
