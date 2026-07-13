//! Facade module: re-exports all domain types at crate visibility for
//! internal consumers (`input.rs`, `replay.rs`, `handlers.rs`).
//! External consumers access through the crate root via `lib.rs`.

pub(crate) use crate::effects::*;
pub(crate) use crate::entities::*;
pub(crate) use crate::errors::*;
pub(crate) use crate::events::*;
pub(crate) use crate::ids::*;
pub(crate) use crate::inputs::*;
pub(crate) use crate::kernel_state::*;
